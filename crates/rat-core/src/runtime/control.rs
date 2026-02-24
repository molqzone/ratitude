use rat_protocol::hash_schema_bytes;
use serde::Deserialize;
use tokio::time::Instant as TokioInstant;
use tracing::{debug, info};

use crate::protocol_engine::{
    ProtocolEngineError, RatProtocolEngine, RuntimeDynamicFieldDef, RuntimeDynamicPacketDef,
};

use super::{RuntimeError, RuntimeFieldDef, RuntimePacketDef};

pub(crate) const CONTROL_PACKET_ID: u8 = 0x00;
pub(crate) const CONTROL_HELLO: u8 = 0x01;
pub(crate) const CONTROL_SCHEMA_CHUNK: u8 = 0x02;
pub(crate) const CONTROL_SCHEMA_COMMIT: u8 = 0x03;
pub(crate) const CONTROL_MAGIC: &[u8; 4] = b"RATS";
pub(crate) const CONTROL_VERSION: u8 = 1;
const HELLO_PAYLOAD_LEN: usize = 18;
const COMMIT_PAYLOAD_LEN: usize = 9;
const MAX_SCHEMA_BYTES: usize = 64 * 1024;

pub(crate) enum ControlOutcome {
    Noop,
    SchemaReset,
    SchemaReady {
        schema_hash: u64,
        packets: Vec<RuntimePacketDef>,
    },
}

pub(crate) fn handle_control_payload(
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
            .register_dynamic(RuntimeDynamicPacketDef {
                id: packet.id as u8,
                struct_name: packet.struct_name.clone(),
                packed: packet.packed,
                byte_size: packet.byte_size,
                fields,
            })
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

#[derive(Debug)]
pub(crate) struct SchemaState {
    timeout: std::time::Duration,
    wait_deadline: TokioInstant,
    ready: bool,
    assembly: Option<SchemaAssembly>,
}

impl SchemaState {
    pub(crate) fn new(timeout: std::time::Duration) -> Self {
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

    pub(crate) fn is_ready(&self) -> bool {
        self.ready
    }

    pub(crate) fn wait_deadline(&self) -> TokioInstant {
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
    #[serde(rename = "type")]
    packet_type: String,
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
            packet_type: packet.packet_type,
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
