use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant as TokioInstant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::protocol_engine::{
    build_dynamic_packet_def, decode_frame, PacketEnvelope, ProtocolEngine, ProtocolEngineError,
    RatProtocolEngine, RuntimeDynamicFieldDef,
};
use crate::{spawn_listener, Hub, ListenerOptions};

const CONTROL_PACKET_ID: u8 = 0x00;
const CONTROL_HELLO: u8 = 0x01;
const CONTROL_SCHEMA_CHUNK: u8 = 0x02;
const CONTROL_SCHEMA_COMMIT: u8 = 0x03;
const CONTROL_MAGIC: &[u8; 4] = b"RATS";
const CONTROL_VERSION: u8 = 1;
const HELLO_PAYLOAD_LEN: usize = 18;
const COMMIT_PAYLOAD_LEN: usize = 9;
const MAX_SCHEMA_BYTES: usize = 64 * 1024;
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
        "schema hash mismatch: expected 0x{expected:016X}, actual 0x{actual:016X}; run ratsync, rebuild firmware, and reflash before starting rttd"
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

fn register_runtime_schema(
    protocol: &mut RatProtocolEngine,
    packets: &[RuntimePacketDef],
) -> Result<(), RuntimeError> {
    protocol.clear_dynamic_registry();

    debug!(
        packets = packets.len(),
        "registering runtime schema packets"
    );
    for packet in packets {
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

    Ok(())
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

enum ControlOutcome {
    Noop,
    SchemaReset,
    SchemaReady {
        schema_hash: u64,
        packets: Vec<RuntimePacketDef>,
    },
}

fn handle_control_payload(
    payload: &[u8],
    schema_state: &mut SchemaState,
    protocol: &mut RatProtocolEngine,
) -> Result<ControlOutcome, RuntimeError> {
    match parse_control_message(payload)? {
        ControlMessage::Hello {
            total_len,
            schema_hash,
        } => {
            protocol.clear_dynamic_registry();
            let assembly = SchemaAssembly::new(total_len, schema_hash)?;
            schema_state.begin_assembly(assembly);
            info!(
                schema_hash = format!("0x{:016X}", schema_hash),
                total_bytes = total_len,
                "runtime schema hello received"
            );
            Ok(ControlOutcome::SchemaReset)
        }
        ControlMessage::SchemaChunk { offset, chunk } => {
            let assembly = schema_state.assembly_mut()?;
            assembly.append(offset, &chunk)?;
            debug!(
                received = assembly.bytes_len(),
                total = assembly.total_len(),
                "runtime schema chunk accepted"
            );
            Ok(ControlOutcome::Noop)
        }
        ControlMessage::SchemaCommit { schema_hash } => {
            let assembly = schema_state.take_assembly()?;
            let ready = assembly.finalize(schema_hash)?;
            register_runtime_schema(protocol, &ready.packets)?;
            schema_state.mark_ready();
            info!(
                schema_hash = format!("0x{:016X}", ready.schema_hash),
                packets = ready.packets.len(),
                "runtime schema committed and activated"
            );
            Ok(ControlOutcome::SchemaReady {
                schema_hash: ready.schema_hash,
                packets: ready.packets,
            })
        }
    }
}

#[derive(Debug)]
struct SchemaState {
    timeout: Duration,
    wait_deadline: TokioInstant,
    ready: bool,
    assembly: Option<SchemaAssembly>,
}

impl SchemaState {
    fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            wait_deadline: TokioInstant::now() + timeout,
            ready: false,
            assembly: None,
        }
    }

    fn begin_assembly(&mut self, assembly: SchemaAssembly) {
        self.ready = false;
        self.assembly = Some(assembly);
        self.wait_deadline = TokioInstant::now() + self.timeout;
    }

    fn mark_ready(&mut self) {
        self.ready = true;
        self.assembly = None;
    }

    fn is_ready(&self) -> bool {
        self.ready
    }

    fn wait_deadline(&self) -> TokioInstant {
        self.wait_deadline
    }

    fn assembly_mut(&mut self) -> Result<&mut SchemaAssembly, RuntimeError> {
        self.assembly
            .as_mut()
            .ok_or_else(|| RuntimeError::ControlProtocol {
                reason: "schema chunk received before hello".to_string(),
            })
    }

    fn take_assembly(&mut self) -> Result<SchemaAssembly, RuntimeError> {
        self.assembly
            .take()
            .ok_or_else(|| RuntimeError::ControlProtocol {
                reason: "schema commit received before hello".to_string(),
            })
    }
}

enum ControlMessage {
    Hello { total_len: usize, schema_hash: u64 },
    SchemaChunk { offset: usize, chunk: Vec<u8> },
    SchemaCommit { schema_hash: u64 },
}

fn parse_control_message(payload: &[u8]) -> Result<ControlMessage, RuntimeError> {
    let Some(op) = payload.first().copied() else {
        return Err(RuntimeError::ControlProtocol {
            reason: "empty control payload".to_string(),
        });
    };

    match op {
        CONTROL_HELLO => parse_hello_message(payload),
        CONTROL_SCHEMA_CHUNK => parse_chunk_message(payload),
        CONTROL_SCHEMA_COMMIT => parse_commit_message(payload),
        other => Err(RuntimeError::ControlProtocol {
            reason: format!("unknown control opcode: 0x{other:02X}"),
        }),
    }
}

fn parse_hello_message(payload: &[u8]) -> Result<ControlMessage, RuntimeError> {
    if payload.len() != HELLO_PAYLOAD_LEN {
        return Err(RuntimeError::ControlProtocol {
            reason: format!(
                "invalid hello payload length: expected {HELLO_PAYLOAD_LEN}, got {}",
                payload.len()
            ),
        });
    }
    if payload.get(1..5) != Some(CONTROL_MAGIC.as_slice()) {
        return Err(RuntimeError::ControlProtocol {
            reason: "invalid hello magic".to_string(),
        });
    }
    if payload[5] != CONTROL_VERSION {
        return Err(RuntimeError::ControlProtocol {
            reason: format!(
                "unsupported control version: expected {CONTROL_VERSION}, got {}",
                payload[5]
            ),
        });
    }

    let total_len = read_u32_le(&payload[6..10])? as usize;
    let schema_hash = read_u64_le(&payload[10..18])?;
    if total_len == 0 {
        return Err(RuntimeError::ControlProtocol {
            reason: "schema total length must be > 0".to_string(),
        });
    }

    Ok(ControlMessage::Hello {
        total_len,
        schema_hash,
    })
}

fn parse_chunk_message(payload: &[u8]) -> Result<ControlMessage, RuntimeError> {
    if payload.len() < 7 {
        return Err(RuntimeError::ControlProtocol {
            reason: "schema chunk payload too short".to_string(),
        });
    }

    let offset = read_u32_le(&payload[1..5])? as usize;
    let chunk_len = read_u16_le(&payload[5..7])? as usize;
    let expected_len = 7 + chunk_len;
    if payload.len() != expected_len {
        return Err(RuntimeError::ControlProtocol {
            reason: format!(
                "schema chunk length mismatch: declared {chunk_len}, payload {}",
                payload.len() - 7
            ),
        });
    }

    Ok(ControlMessage::SchemaChunk {
        offset,
        chunk: payload[7..].to_vec(),
    })
}

fn parse_commit_message(payload: &[u8]) -> Result<ControlMessage, RuntimeError> {
    if payload.len() != COMMIT_PAYLOAD_LEN {
        return Err(RuntimeError::ControlProtocol {
            reason: format!(
                "invalid schema commit payload length: expected {COMMIT_PAYLOAD_LEN}, got {}",
                payload.len()
            ),
        });
    }

    let schema_hash = read_u64_le(&payload[1..9])?;
    Ok(ControlMessage::SchemaCommit { schema_hash })
}

fn read_u16_le(raw: &[u8]) -> Result<u16, RuntimeError> {
    if raw.len() != 2 {
        return Err(RuntimeError::ControlProtocol {
            reason: format!("invalid u16 width: {}", raw.len()),
        });
    }
    let mut bytes = [0_u8; 2];
    bytes.copy_from_slice(raw);
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32_le(raw: &[u8]) -> Result<u32, RuntimeError> {
    if raw.len() != 4 {
        return Err(RuntimeError::ControlProtocol {
            reason: format!("invalid u32 width: {}", raw.len()),
        });
    }
    let mut bytes = [0_u8; 4];
    bytes.copy_from_slice(raw);
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64_le(raw: &[u8]) -> Result<u64, RuntimeError> {
    if raw.len() != 8 {
        return Err(RuntimeError::ControlProtocol {
            reason: format!("invalid u64 width: {}", raw.len()),
        });
    }
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(raw);
    Ok(u64::from_le_bytes(bytes))
}

#[derive(Debug)]
struct SchemaAssembly {
    total_len: usize,
    expected_hash: u64,
    bytes: Vec<u8>,
}

impl SchemaAssembly {
    fn new(total_len: usize, expected_hash: u64) -> Result<Self, RuntimeError> {
        if total_len > MAX_SCHEMA_BYTES {
            return Err(RuntimeError::SchemaTooLarge {
                actual: total_len,
                max: MAX_SCHEMA_BYTES,
            });
        }
        Ok(Self {
            total_len,
            expected_hash,
            bytes: Vec::with_capacity(total_len),
        })
    }

    fn append(&mut self, offset: usize, chunk: &[u8]) -> Result<(), RuntimeError> {
        let expected = self.bytes.len();
        if offset != expected {
            return Err(RuntimeError::SchemaChunkOutOfOrder {
                expected,
                actual: offset,
            });
        }

        let new_len = self.bytes.len().saturating_add(chunk.len());
        if new_len > self.total_len {
            return Err(RuntimeError::SchemaChunkOverflow {
                offset,
                chunk_len: chunk.len(),
                total: self.total_len,
            });
        }

        self.bytes.extend_from_slice(chunk);
        Ok(())
    }

    fn bytes_len(&self) -> usize {
        self.bytes.len()
    }

    fn total_len(&self) -> usize {
        self.total_len
    }

    fn finalize(self, commit_hash: u64) -> Result<SchemaReadyPayload, RuntimeError> {
        if commit_hash != self.expected_hash {
            return Err(RuntimeError::SchemaHashMismatch {
                expected: self.expected_hash,
                actual: commit_hash,
            });
        }
        if self.bytes.len() != self.total_len {
            return Err(RuntimeError::SchemaCommitBeforeComplete {
                received: self.bytes.len(),
                expected: self.total_len,
            });
        }

        let computed_hash = hash_schema_bytes(&self.bytes);
        if computed_hash != self.expected_hash {
            return Err(RuntimeError::SchemaHashMismatch {
                expected: self.expected_hash,
                actual: computed_hash,
            });
        }

        let packets = parse_runtime_packets_from_schema(&self.bytes)?;
        Ok(SchemaReadyPayload {
            schema_hash: self.expected_hash,
            packets,
        })
    }
}

struct SchemaReadyPayload {
    schema_hash: u64,
    packets: Vec<RuntimePacketDef>,
}

#[derive(Debug, Deserialize)]
struct RuntimeSchemaDocument {
    #[serde(default)]
    packets: Vec<RuntimeSchemaPacket>,
}

#[derive(Debug, Deserialize)]
struct RuntimeSchemaPacket {
    id: u16,
    struct_name: String,
    #[serde(default)]
    packed: bool,
    byte_size: usize,
    #[serde(default)]
    fields: Vec<RuntimeSchemaField>,
}

#[derive(Debug, Deserialize)]
struct RuntimeSchemaField {
    name: String,
    c_type: String,
    offset: usize,
    size: usize,
}

fn parse_runtime_packets_from_schema(
    schema_bytes: &[u8],
) -> Result<Vec<RuntimePacketDef>, RuntimeError> {
    let raw = std::str::from_utf8(schema_bytes).map_err(|err| RuntimeError::SchemaParseFailed {
        reason: format!("schema payload is not utf-8: {err}"),
    })?;

    let doc: RuntimeSchemaDocument =
        toml::from_str(raw).map_err(|err| RuntimeError::SchemaParseFailed {
            reason: format!("schema payload is not valid toml: {err}"),
        })?;

    if doc.packets.is_empty() {
        return Err(RuntimeError::SchemaParseFailed {
            reason: "schema has no packets".to_string(),
        });
    }

    Ok(doc
        .packets
        .into_iter()
        .map(|packet| RuntimePacketDef {
            id: packet.id,
            struct_name: packet.struct_name,
            packed: packet.packed,
            byte_size: packet.byte_size,
            fields: packet
                .fields
                .into_iter()
                .map(|field| RuntimeFieldDef {
                    name: field.name,
                    c_type: field.c_type,
                    offset: field.offset,
                    size: field.size,
                })
                .collect(),
        })
        .collect())
}

fn hash_schema_bytes(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
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

    fn schema_toml() -> String {
        [
            "[[packets]]",
            "id = 33",
            "struct_name = \"DemoPacket\"",
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
