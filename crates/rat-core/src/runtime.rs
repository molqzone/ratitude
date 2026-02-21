use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::protocol_engine::{
    build_dynamic_packet_def, decode_frame, PacketEnvelope, ProtocolEngine, ProtocolEngineError,
    RatProtocolEngine, RuntimeDynamicFieldDef,
};
use crate::{spawn_listener, Hub, ListenerOptions};

const INIT_MAGIC_PACKET_ID: u8 = 0x00;
const INIT_MAGIC_PREFIX: &[u8] = b"RATI";
const INIT_MAGIC_PAYLOAD_LEN: usize = 12;
const SIGNAL_BUFFER: usize = 8;

#[derive(Clone, Debug)]
pub struct RuntimeFieldDef {
    pub name: String,
    pub c_type: String,
    pub offset: usize,
    pub size: usize,
}

#[derive(Clone, Debug)]
pub struct RuntimePacketDef {
    pub id: u16,
    pub struct_name: String,
    pub packed: bool,
    pub byte_size: usize,
    pub fields: Vec<RuntimeFieldDef>,
}

#[derive(Clone, Debug)]
pub struct IngestRuntimeConfig {
    pub addr: String,
    pub listener: ListenerOptions,
    pub hub_buffer: usize,
    pub text_packet_id: u8,
    pub expected_fingerprint: u64,
    pub packets: Vec<RuntimePacketDef>,
    pub unknown_window: Duration,
    pub unknown_threshold: u32,
}

#[derive(Clone, Debug)]
pub enum RuntimeSignal {
    InitMagicVerified,
    Fatal(RuntimeError),
}

#[derive(Debug, Error, Clone)]
pub enum RuntimeError {
    #[error("packet id out of range: 0x{id:X}")]
    PacketIdOutOfRange { id: u16 },

    #[error("register packet 0x{id:02X} ({struct_name}) failed: {reason}")]
    PacketRegisterFailed {
        id: u16,
        struct_name: String,
        reason: String,
    },

    #[error(
        "init magic fingerprint mismatch: firmware=0x{firmware:016X}, generated=0x{generated:016X}"
    )]
    FingerprintMismatch { firmware: u64, generated: u64 },

    #[error("frame consumer stopped before shutdown")]
    FrameConsumerStopped,

    #[error("frame consumer task failed: {reason}")]
    TaskJoinFailed { reason: String },
}

pub struct IngestRuntime {
    shutdown: CancellationToken,
    hub: Hub,
    listener_task: JoinHandle<()>,
    consume_task: JoinHandle<()>,
    signals_rx: mpsc::Receiver<RuntimeSignal>,
}

impl IngestRuntime {
    pub fn hub(&self) -> Hub {
        self.hub.clone()
    }

    pub async fn recv_signal(&mut self) -> Option<RuntimeSignal> {
        self.signals_rx.recv().await
    }

    pub async fn shutdown(self, join_consumer: bool) {
        let IngestRuntime {
            shutdown,
            hub: _hub,
            listener_task,
            consume_task,
            signals_rx: _signals_rx,
        } = self;

        shutdown.cancel();
        listener_task.abort();
        let _ = listener_task.await;
        if join_consumer {
            let _ = consume_task.await;
        }
    }
}

pub async fn start_ingest_runtime(cfg: IngestRuntimeConfig) -> Result<IngestRuntime, RuntimeError> {
    let protocol = Arc::new(build_protocol_engine(&cfg)?);

    let shutdown = CancellationToken::new();
    let hub = Hub::new(cfg.hub_buffer.max(1));
    let (frame_tx, frame_rx) = mpsc::channel::<Vec<u8>>(cfg.hub_buffer.max(1));
    let (signals_tx, signals_rx) = mpsc::channel::<RuntimeSignal>(SIGNAL_BUFFER);

    let listener_task = spawn_listener(shutdown.clone(), cfg.addr, frame_tx, cfg.listener);

    let consume_task = spawn_frame_consumer_monitor(
        frame_rx,
        hub.clone(),
        protocol,
        shutdown.clone(),
        cfg.expected_fingerprint,
        cfg.unknown_window,
        cfg.unknown_threshold,
        signals_tx,
    );

    Ok(IngestRuntime {
        shutdown,
        hub,
        listener_task,
        consume_task,
        signals_rx,
    })
}

fn build_protocol_engine(cfg: &IngestRuntimeConfig) -> Result<RatProtocolEngine, RuntimeError> {
    let mut protocol = RatProtocolEngine::new();
    protocol.set_text_packet_id(cfg.text_packet_id);

    protocol.clear_dynamic_registry();
    debug!(
        packets = cfg.packets.len(),
        "registering dynamic packet definitions"
    );

    for packet in &cfg.packets {
        if packet.id > 0xFF {
            return Err(RuntimeError::PacketIdOutOfRange { id: packet.id });
        }

        let fields = packet
            .fields
            .iter()
            .map(|field| RuntimeDynamicFieldDef {
                name: field.name.clone(),
                c_type: field.c_type.clone(),
                offset: field.offset,
                size: field.size,
            })
            .collect::<Vec<RuntimeDynamicFieldDef>>();

        protocol
            .register_dynamic(build_dynamic_packet_def(
                packet.id as u8,
                packet.struct_name.clone(),
                packet.packed,
                packet.byte_size,
                fields,
            ))
            .map_err(|err| RuntimeError::PacketRegisterFailed {
                id: packet.id,
                struct_name: packet.struct_name.clone(),
                reason: format_protocol_register_error(err),
            })?;
    }

    Ok(protocol)
}

fn format_protocol_register_error(error: ProtocolEngineError) -> String {
    match error {
        ProtocolEngineError::Register(reason) => reason,
        other => other.to_string(),
    }
}

fn spawn_frame_consumer_monitor(
    receiver: mpsc::Receiver<Vec<u8>>,
    hub: Hub,
    protocol: Arc<RatProtocolEngine>,
    shutdown: CancellationToken,
    expected_fingerprint: u64,
    unknown_window: Duration,
    unknown_threshold: u32,
    signals: mpsc::Sender<RuntimeSignal>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let consume = tokio::spawn(run_frame_consumer(
            receiver,
            hub,
            protocol,
            shutdown.clone(),
            expected_fingerprint,
            unknown_window,
            unknown_threshold,
            signals.clone(),
        ));

        let fatal = match consume.await {
            Ok(Ok(())) => None,
            Ok(Err(err)) => Some(err),
            Err(err) => Some(RuntimeError::TaskJoinFailed {
                reason: err.to_string(),
            }),
        };

        if let Some(err) = fatal {
            let _ = signals.try_send(RuntimeSignal::Fatal(err));
            shutdown.cancel();
        }
    })
}

async fn run_frame_consumer(
    mut receiver: mpsc::Receiver<Vec<u8>>,
    hub: Hub,
    protocol: Arc<RatProtocolEngine>,
    shutdown: CancellationToken,
    expected_fingerprint: u64,
    unknown_window: Duration,
    unknown_threshold: u32,
    signals: mpsc::Sender<RuntimeSignal>,
) -> Result<(), RuntimeError> {
    let mut unknown_monitor = UnknownPacketMonitor::new(unknown_window, unknown_threshold);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            maybe_frame = receiver.recv() => {
                let Some(frame) = maybe_frame else {
                    return Err(RuntimeError::FrameConsumerStopped);
                };
                let decoded = match decode_frame(&frame) {
                    Ok(decoded) => decoded,
                    Err(err) => {
                        debug!(error = %err, frame_len = frame.len(), "dropping invalid COBS frame");
                        continue;
                    }
                };
                if decoded.is_empty() {
                    continue;
                }

                let id = decoded[0];
                let payload = decoded[1..].to_vec();

                if let Some(fingerprint) = decode_init_magic_packet(id, &payload) {
                    if fingerprint != expected_fingerprint {
                        error!(
                            firmware_fingerprint = format!("0x{:016X}", fingerprint),
                            generated_fingerprint = format!("0x{:016X}", expected_fingerprint),
                            "librat init magic fingerprint mismatch"
                        );
                        shutdown.cancel();
                        return Err(RuntimeError::FingerprintMismatch {
                            firmware: fingerprint,
                            generated: expected_fingerprint,
                        });
                    }
                    let _ = signals.try_send(RuntimeSignal::InitMagicVerified);
                    info!(
                        fingerprint = format!("0x{:016X}", fingerprint),
                        "received librat init magic packet (fingerprint verified)"
                    );
                    continue;
                }

                let data = match protocol.parse_packet(id, &payload) {
                    Ok(data) => data,
                    Err(ProtocolEngineError::UnknownPacketId(unknown_id)) => {
                        let observation = unknown_monitor.record(unknown_id);
                        if let Some(report) = observation.rolled_over {
                            warn!(
                                window_secs = unknown_monitor.window.as_secs(),
                                dropped = report.count,
                                unique_ids = report.unique_ids,
                                "unknown packets dropped in previous window"
                            );
                        }
                        if observation.threshold_crossed {
                            error!(
                                packet_id = format!("0x{:02X}", unknown_id),
                                window_secs = unknown_monitor.window.as_secs(),
                                threshold = unknown_monitor.threshold,
                                window_count = observation.window_count,
                                total_unknown = observation.total_count,
                                "unknown packet flood detected (not declared in rat_gen.toml)"
                            );
                        } else {
                            warn!(
                                packet_id = format!("0x{:02X}", unknown_id),
                                window_count = observation.window_count,
                                total_unknown = observation.total_count,
                                "dropping unknown packet id (not declared in rat_gen.toml)"
                            );
                        }
                        continue;
                    }
                    Err(err) => {
                        warn!(packet_id = format!("0x{:02X}", id), error = %err, payload_len = payload.len(), "dropping undecodable packet");
                        continue;
                    }
                };

                hub.publish(PacketEnvelope {
                    id,
                    timestamp: SystemTime::now(),
                    payload,
                    data,
                });
            }
        }
    }

    Ok(())
}

fn decode_init_magic_packet(id: u8, payload: &[u8]) -> Option<u64> {
    if id != INIT_MAGIC_PACKET_ID || payload.len() != INIT_MAGIC_PAYLOAD_LEN {
        return None;
    }
    if payload.get(0..4) != Some(INIT_MAGIC_PREFIX) {
        return None;
    }

    let mut fingerprint = 0_u64;
    for (idx, byte) in payload[4..12].iter().enumerate() {
        fingerprint |= (*byte as u64) << (idx * 8);
    }
    Some(fingerprint)
}

#[derive(Clone, Debug)]
struct UnknownPacketWindowReport {
    count: u32,
    unique_ids: usize,
}

#[derive(Clone, Debug)]
struct UnknownPacketObservation {
    total_count: u64,
    window_count: u32,
    threshold_crossed: bool,
    rolled_over: Option<UnknownPacketWindowReport>,
}

#[derive(Clone, Debug)]
struct UnknownPacketMonitor {
    window: Duration,
    threshold: u32,
    window_started_at: Instant,
    window_count: u32,
    total_count: u64,
    per_window_ids: BTreeMap<u8, u32>,
}

impl UnknownPacketMonitor {
    fn new(window: Duration, threshold: u32) -> Self {
        Self {
            window,
            threshold: threshold.max(1),
            window_started_at: Instant::now(),
            window_count: 0,
            total_count: 0,
            per_window_ids: BTreeMap::new(),
        }
    }

    fn record(&mut self, packet_id: u8) -> UnknownPacketObservation {
        self.record_at(packet_id, Instant::now())
    }

    fn record_at(&mut self, packet_id: u8, now: Instant) -> UnknownPacketObservation {
        let mut rolled_over = None;
        if now.duration_since(self.window_started_at) >= self.window {
            if self.window_count > 0 {
                rolled_over = Some(UnknownPacketWindowReport {
                    count: self.window_count,
                    unique_ids: self.per_window_ids.len(),
                });
            }
            self.window_started_at = now;
            self.window_count = 0;
            self.per_window_ids.clear();
        }

        self.window_count = self.window_count.saturating_add(1);
        self.total_count = self.total_count.saturating_add(1);
        *self.per_window_ids.entry(packet_id).or_insert(0) += 1;

        UnknownPacketObservation {
            total_count: self.total_count,
            window_count: self.window_count,
            threshold_crossed: self.window_count == self.threshold,
            rolled_over,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    use super::*;
    use crate::PacketPayload;

    fn encode_frame(decoded: &[u8]) -> Vec<u8> {
        let mut encoded = cobs::encode_vec(decoded);
        encoded.push(0);
        encoded
    }

    fn listener_opts() -> ListenerOptions {
        ListenerOptions {
            reconnect: Duration::from_millis(50),
            reconnect_max: Duration::from_millis(200),
            dial_timeout: Duration::from_millis(200),
            reader_buf_bytes: 1024,
        }
    }

    fn demo_packet_defs() -> Vec<RuntimePacketDef> {
        vec![RuntimePacketDef {
            id: 0x21,
            struct_name: "DemoPacket".to_string(),
            packed: true,
            byte_size: 4,
            fields: vec![RuntimeFieldDef {
                name: "value".to_string(),
                c_type: "uint32_t".to_string(),
                offset: 0,
                size: 4,
            }],
        }]
    }

    async fn spawn_once_sender(listener: TcpListener, payload: Vec<u8>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            socket.write_all(&payload).await.expect("write frame");
        })
    }

    #[test]
    fn decode_init_magic_packet_extracts_fingerprint() {
        let payload = [
            b'R', b'A', b'T', b'I', 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
        ];
        let fp = decode_init_magic_packet(0x00, &payload).expect("decode");
        assert_eq!(fp, 0x1122_3344_5566_7788);
    }

    #[test]
    fn unknown_packet_monitor_threshold_once_per_window() {
        let mut monitor = UnknownPacketMonitor::new(Duration::from_secs(10), 3);
        let start = Instant::now();

        let first = monitor.record_at(0x10, start);
        assert!(!first.threshold_crossed);

        let second = monitor.record_at(0x10, start + Duration::from_millis(1));
        assert!(!second.threshold_crossed);

        let third = monitor.record_at(0x10, start + Duration::from_millis(2));
        assert!(third.threshold_crossed);

        let fourth = monitor.record_at(0x10, start + Duration::from_millis(3));
        assert!(!fourth.threshold_crossed);
    }

    #[tokio::test]
    async fn runtime_decodes_and_publishes_valid_packet() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let send_task = spawn_once_sender(listener, encode_frame(&[0x21, 1, 0, 0, 0])).await;

        let runtime = start_ingest_runtime(IngestRuntimeConfig {
            addr,
            listener: listener_opts(),
            hub_buffer: 8,
            text_packet_id: 0xFF,
            expected_fingerprint: 0xAABB_CCDD_EEFF_0011,
            packets: demo_packet_defs(),
            unknown_window: Duration::from_secs(5),
            unknown_threshold: 20,
        })
        .await
        .expect("start runtime");

        let mut sub = runtime.hub().subscribe();
        let packet = tokio::time::timeout(Duration::from_secs(1), sub.recv())
            .await
            .expect("recv timeout")
            .expect("recv packet");

        assert_eq!(packet.id, 0x21);
        let PacketPayload::Dynamic(map) = packet.data else {
            panic!("expected dynamic packet");
        };
        let expected = BTreeMap::from([("value".to_string(), serde_json::Value::from(1_u64))]);
        let actual = map
            .into_iter()
            .collect::<BTreeMap<String, serde_json::Value>>();
        assert_eq!(actual, expected);

        runtime.shutdown(true).await;
        let _ = send_task.await;
    }

    #[tokio::test]
    async fn runtime_emits_init_magic_verified_signal() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let fingerprint = 0x1122_3344_5566_7788_u64;

        let mut init_packet = vec![0x00, b'R', b'A', b'T', b'I'];
        init_packet.extend_from_slice(&fingerprint.to_le_bytes());
        let send_task = spawn_once_sender(listener, encode_frame(&init_packet)).await;

        let mut runtime = start_ingest_runtime(IngestRuntimeConfig {
            addr,
            listener: listener_opts(),
            hub_buffer: 8,
            text_packet_id: 0xFF,
            expected_fingerprint: fingerprint,
            packets: demo_packet_defs(),
            unknown_window: Duration::from_secs(5),
            unknown_threshold: 20,
        })
        .await
        .expect("start runtime");

        let signal = tokio::time::timeout(Duration::from_secs(1), runtime.recv_signal())
            .await
            .expect("signal timeout")
            .expect("signal");
        assert!(matches!(signal, RuntimeSignal::InitMagicVerified));

        runtime.shutdown(true).await;
        let _ = send_task.await;
    }

    #[tokio::test]
    async fn runtime_emits_fatal_on_fingerprint_mismatch() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();

        let mut init_packet = vec![0x00, b'R', b'A', b'T', b'I'];
        init_packet.extend_from_slice(&0x8899_AABB_CCDD_EEFF_u64.to_le_bytes());
        let send_task = spawn_once_sender(listener, encode_frame(&init_packet)).await;

        let mut runtime = start_ingest_runtime(IngestRuntimeConfig {
            addr,
            listener: listener_opts(),
            hub_buffer: 8,
            text_packet_id: 0xFF,
            expected_fingerprint: 0x1122_3344_5566_7788,
            packets: demo_packet_defs(),
            unknown_window: Duration::from_secs(5),
            unknown_threshold: 20,
        })
        .await
        .expect("start runtime");

        let signal = tokio::time::timeout(Duration::from_secs(1), runtime.recv_signal())
            .await
            .expect("signal timeout")
            .expect("signal");

        match signal {
            RuntimeSignal::Fatal(RuntimeError::FingerprintMismatch {
                firmware,
                generated,
            }) => {
                assert_eq!(firmware, 0x8899_AABB_CCDD_EEFF);
                assert_eq!(generated, 0x1122_3344_5566_7788);
            }
            other => panic!("unexpected signal: {other:?}"),
        }

        runtime.shutdown(true).await;
        let _ = send_task.await;
    }
}
