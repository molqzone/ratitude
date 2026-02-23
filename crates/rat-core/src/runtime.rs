mod control;
mod unknown_monitor;

use std::time::{Duration, SystemTime};

use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

use crate::protocol_engine::{
    decode_frame, PacketEnvelope, ProtocolEngine, ProtocolEngineError, RatProtocolEngine,
};
use crate::{spawn_listener, Hub, ListenerOptions};

use self::control::{handle_control_payload, ControlOutcome, SchemaState, CONTROL_PACKET_ID};
#[cfg(test)]
use self::control::{
    hash_schema_bytes, CONTROL_HELLO, CONTROL_MAGIC, CONTROL_SCHEMA_CHUNK, CONTROL_SCHEMA_COMMIT,
    CONTROL_VERSION,
};
use self::unknown_monitor::UnknownPacketMonitor;

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
    pub packet_type: String,
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
    pub schema_timeout: Duration,
    pub unknown_window: Duration,
    pub unknown_threshold: u32,
}

#[derive(Clone, Debug)]
pub enum RuntimeSignal {
    SchemaReady {
        schema_hash: u64,
        packets: Vec<RuntimePacketDef>,
    },
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

    #[error("runtime schema timeout after {timeout_ms}ms (no complete schema received)")]
    SchemaTimeout { timeout_ms: u64 },

    #[error("schema control protocol error: {reason}")]
    ControlProtocol { reason: String },

    #[error("schema size exceeds limit: {actual} > {max}")]
    SchemaTooLarge { actual: usize, max: usize },

    #[error("schema chunk out of order: expected offset {expected}, got {actual}")]
    SchemaChunkOutOfOrder { expected: usize, actual: usize },

    #[error("schema chunk overflow: offset={offset}, chunk={chunk_len}, total={total}")]
    SchemaChunkOverflow {
        offset: usize,
        chunk_len: usize,
        total: usize,
    },

    #[error("schema commit before completion: received {received}, expected {expected}")]
    SchemaCommitBeforeComplete { received: usize, expected: usize },

    #[error(
        "schema hash mismatch: expected 0x{expected:016X}, actual 0x{actual:016X}; run ratsync, rebuild firmware, and reflash before starting ratd"
    )]
    SchemaHashMismatch { expected: u64, actual: u64 },

    #[error("schema parse failed: {reason}")]
    SchemaParseFailed { reason: String },

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
    let protocol = build_protocol_engine(cfg.text_packet_id);

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
        cfg.schema_timeout,
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

fn build_protocol_engine(text_packet_id: u8) -> RatProtocolEngine {
    let mut protocol = RatProtocolEngine::new();
    protocol.set_text_packet_id(text_packet_id);
    protocol.clear_dynamic_registry();
    protocol
}

fn spawn_frame_consumer_monitor(
    receiver: mpsc::Receiver<Vec<u8>>,
    hub: Hub,
    protocol: RatProtocolEngine,
    shutdown: CancellationToken,
    schema_timeout: Duration,
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
            schema_timeout,
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
    mut protocol: RatProtocolEngine,
    shutdown: CancellationToken,
    schema_timeout: Duration,
    unknown_window: Duration,
    unknown_threshold: u32,
    signals: mpsc::Sender<RuntimeSignal>,
) -> Result<(), RuntimeError> {
    let timeout = schema_timeout.max(Duration::from_millis(1));
    let mut schema_state = SchemaState::new(timeout);
    let mut unknown_monitor = UnknownPacketMonitor::new(unknown_window, unknown_threshold);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = tokio::time::sleep_until(schema_state.wait_deadline()), if !schema_state.is_ready() => {
                shutdown.cancel();
                return Err(RuntimeError::SchemaTimeout { timeout_ms: timeout.as_millis() as u64 });
            }
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

                if id == CONTROL_PACKET_ID {
                    let control_outcome = handle_control_payload(&payload, &mut schema_state, &mut protocol)?;
                    match control_outcome {
                        ControlOutcome::SchemaReset => {
                            unknown_monitor = UnknownPacketMonitor::new(unknown_window, unknown_threshold);
                        }
                        ControlOutcome::SchemaReady {
                            schema_hash,
                            packets,
                        } => {
                            unknown_monitor = UnknownPacketMonitor::new(unknown_window, unknown_threshold);
                            let _ = signals.try_send(RuntimeSignal::SchemaReady {
                                schema_hash,
                                packets,
                            });
                        }
                        ControlOutcome::Noop => {}
                    }
                    continue;
                }

                if !schema_state.is_ready() {
                    debug!(packet_id = format!("0x{:02X}", id), "dropping packet before runtime schema becomes ready");
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
                                "unknown packet flood detected (not declared in runtime schema)"
                            );
                        } else {
                            warn!(
                                packet_id = format!("0x{:02X}", unknown_id),
                                window_count = observation.window_count,
                                total_unknown = observation.total_count,
                                "dropping unknown packet id (not declared in runtime schema)"
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Instant;

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

    fn schema_toml() -> String {
        [
            "[[packets]]",
            "id = 33",
            "struct_name = \"DemoPacket\"",
            "type = \"plot\"",
            "packed = true",
            "byte_size = 4",
            "",
            "[[packets.fields]]",
            "name = \"value\"",
            "c_type = \"uint32_t\"",
            "offset = 0",
            "size = 4",
        ]
        .join("\n")
    }

    fn control_frames(schema: &[u8], chunk_size: usize) -> Vec<Vec<u8>> {
        let schema_hash = hash_schema_bytes(schema);
        let mut decoded_frames = Vec::new();

        let mut hello = vec![CONTROL_PACKET_ID, CONTROL_HELLO];
        hello.extend_from_slice(CONTROL_MAGIC);
        hello.push(CONTROL_VERSION);
        hello.extend_from_slice(&(schema.len() as u32).to_le_bytes());
        hello.extend_from_slice(&schema_hash.to_le_bytes());
        decoded_frames.push(hello);

        let mut offset = 0usize;
        let chunk_size = chunk_size.max(1);
        while offset < schema.len() {
            let end = (offset + chunk_size).min(schema.len());
            let chunk = &schema[offset..end];
            let mut frame = vec![CONTROL_PACKET_ID, CONTROL_SCHEMA_CHUNK];
            frame.extend_from_slice(&(offset as u32).to_le_bytes());
            frame.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
            frame.extend_from_slice(chunk);
            decoded_frames.push(frame);
            offset = end;
        }

        let mut commit = vec![CONTROL_PACKET_ID, CONTROL_SCHEMA_COMMIT];
        commit.extend_from_slice(&schema_hash.to_le_bytes());
        decoded_frames.push(commit);

        decoded_frames
            .into_iter()
            .map(|decoded| encode_frame(&decoded))
            .collect()
    }

    async fn spawn_frames_once(listener: TcpListener, frames: Vec<Vec<u8>>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            for frame in frames {
                socket.write_all(&frame).await.expect("write frame");
            }
        })
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
    async fn runtime_decodes_and_publishes_valid_packet_after_schema_ready() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let schema = schema_toml().into_bytes();

        let mut frames = control_frames(&schema, 16);
        frames.push(encode_frame(&[0x21, 1, 0, 0, 0]));
        let send_task = spawn_frames_once(listener, frames).await;

        let mut runtime = start_ingest_runtime(IngestRuntimeConfig {
            addr,
            listener: listener_opts(),
            hub_buffer: 8,
            text_packet_id: 0xFF,
            schema_timeout: Duration::from_secs(1),
            unknown_window: Duration::from_secs(5),
            unknown_threshold: 20,
        })
        .await
        .expect("start runtime");

        let mut sub = runtime.hub().subscribe();
        let ready = tokio::time::timeout(Duration::from_secs(1), runtime.recv_signal())
            .await
            .expect("schema ready timeout")
            .expect("schema ready signal");
        assert!(matches!(ready, RuntimeSignal::SchemaReady { .. }));

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
    async fn runtime_emits_schema_ready_signal() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let schema = schema_toml().into_bytes();

        let send_task = spawn_frames_once(listener, control_frames(&schema, 7)).await;

        let mut runtime = start_ingest_runtime(IngestRuntimeConfig {
            addr,
            listener: listener_opts(),
            hub_buffer: 8,
            text_packet_id: 0xFF,
            schema_timeout: Duration::from_secs(1),
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
            RuntimeSignal::SchemaReady {
                schema_hash,
                packets,
            } => {
                assert_eq!(schema_hash, hash_schema_bytes(&schema));
                assert_eq!(packets.len(), 1);
                assert_eq!(packets[0].id, 0x21);
                assert_eq!(packets[0].packet_type, "plot");
            }
            other => panic!("unexpected signal: {other:?}"),
        }

        runtime.shutdown(true).await;
        let _ = send_task.await;
    }

    #[tokio::test]
    async fn runtime_emits_fatal_on_schema_hash_mismatch() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let schema = schema_toml().into_bytes();
        let schema_hash = hash_schema_bytes(&schema);

        let mut frames = Vec::new();
        let mut hello = vec![CONTROL_PACKET_ID, CONTROL_HELLO];
        hello.extend_from_slice(CONTROL_MAGIC);
        hello.push(CONTROL_VERSION);
        hello.extend_from_slice(&(schema.len() as u32).to_le_bytes());
        hello.extend_from_slice(&schema_hash.to_le_bytes());
        frames.push(encode_frame(&hello));

        let mut chunk = vec![CONTROL_PACKET_ID, CONTROL_SCHEMA_CHUNK];
        chunk.extend_from_slice(&0_u32.to_le_bytes());
        chunk.extend_from_slice(&(schema.len() as u16).to_le_bytes());
        chunk.extend_from_slice(&schema);
        frames.push(encode_frame(&chunk));

        let mut commit = vec![CONTROL_PACKET_ID, CONTROL_SCHEMA_COMMIT];
        commit.extend_from_slice(&(schema_hash ^ 0xFF).to_le_bytes());
        frames.push(encode_frame(&commit));

        let send_task = spawn_frames_once(listener, frames).await;

        let mut runtime = start_ingest_runtime(IngestRuntimeConfig {
            addr,
            listener: listener_opts(),
            hub_buffer: 8,
            text_packet_id: 0xFF,
            schema_timeout: Duration::from_secs(1),
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
            RuntimeSignal::Fatal(RuntimeError::SchemaHashMismatch { .. }) => {}
            other => panic!("unexpected signal: {other:?}"),
        }

        runtime.shutdown(true).await;
        let _ = send_task.await;
    }
}
