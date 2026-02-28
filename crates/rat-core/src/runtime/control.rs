use std::collections::HashSet;
use std::time::Duration;

use rat_protocol::{DynamicFieldDef, DynamicPacketDef};
use tokio::time::Instant as TokioInstant;
use tracing::{debug, info};

use crate::protocol_engine::{ProtocolEngineError, RatProtocolEngine};

use super::control_message::{parse_control_message, ControlMessage};
#[cfg(test)]
pub(crate) use super::control_message::{
    CONTROL_HELLO, CONTROL_MAGIC, CONTROL_SCHEMA_CHUNK, CONTROL_SCHEMA_COMMIT, CONTROL_VERSION,
};
use super::schema_assembly::SchemaAssembly;
use super::{RuntimeError, RuntimePacketDef};

pub(crate) const CONTROL_PACKET_ID: u8 = 0x00;

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
            let (received, total) = {
                let assembly = schema_state.assembly_mut()?;
                assembly.append(offset, &chunk)?;
                (assembly.bytes_len(), assembly.total_len())
            };
            schema_state.refresh_wait_deadline();
            debug!(
                received = received,
                total = total,
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
    let mut staged_protocol = protocol.clone();
    staged_protocol.clear_dynamic_registry();
    let mut seen_ids = HashSet::with_capacity(packets.len());

    debug!(
        packets = packets.len(),
        "registering runtime schema packets"
    );
    for packet in packets {
        if packet.id == CONTROL_PACKET_ID as u16 {
            return Err(RuntimeError::ReservedPacketId { id: packet.id });
        }
        if !seen_ids.insert(packet.id) {
            return Err(RuntimeError::DuplicatePacketId { id: packet.id });
        }
        if packet.id > 0xFF {
            return Err(RuntimeError::PacketIdOutOfRange { id: packet.id });
        }

        let fields = packet
            .fields
            .iter()
            .map(|field| DynamicFieldDef {
                name: field.name.clone(),
                c_type: field.c_type.clone(),
                offset: field.offset,
                size: field.size,
            })
            .collect::<Vec<DynamicFieldDef>>();

        staged_protocol
            .register_dynamic(DynamicPacketDef {
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

    *protocol = staged_protocol;
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
    timeout: Duration,
    wait_deadline: TokioInstant,
    ready: bool,
    assembly: Option<SchemaAssembly>,
}

impl SchemaState {
    pub(crate) fn new(timeout: Duration) -> Self {
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
        self.refresh_wait_deadline();
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

    pub(crate) fn refresh_wait_deadline(&mut self) {
        self.wait_deadline = TokioInstant::now() + self.timeout;
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rat_protocol::{DynamicFieldDef, DynamicPacketDef};

    use super::*;
    use crate::protocol_engine::ProtocolEngineError;

    fn encode_hello(total_len: u32, schema_hash: u64) -> Vec<u8> {
        let mut payload = vec![CONTROL_HELLO];
        payload.extend_from_slice(CONTROL_MAGIC);
        payload.push(CONTROL_VERSION);
        payload.extend_from_slice(&total_len.to_le_bytes());
        payload.extend_from_slice(&schema_hash.to_le_bytes());
        payload
    }

    fn encode_chunk(offset: u32, chunk: &[u8]) -> Vec<u8> {
        assert!(u16::try_from(chunk.len()).is_ok(), "chunk length overflow");
        let mut payload = vec![CONTROL_SCHEMA_CHUNK];
        payload.extend_from_slice(&offset.to_le_bytes());
        payload.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        payload.extend_from_slice(chunk);
        payload
    }

    fn encode_commit(schema_hash: u64) -> Vec<u8> {
        let mut payload = vec![CONTROL_SCHEMA_COMMIT];
        payload.extend_from_slice(&schema_hash.to_le_bytes());
        payload
    }

    #[test]
    fn invalid_hello_does_not_clear_existing_dynamic_registry() {
        let mut protocol = RatProtocolEngine::new();
        protocol
            .register_dynamic(DynamicPacketDef {
                id: 0x21,
                struct_name: "DemoPacket".to_string(),
                packed: true,
                byte_size: 4,
                fields: vec![DynamicFieldDef {
                    name: "value".to_string(),
                    c_type: "uint32_t".to_string(),
                    offset: 0,
                    size: 4,
                }],
            })
            .expect("register dynamic packet");

        let mut schema_state = SchemaState::new(Duration::from_secs(1));
        let err = match handle_control_payload(
            &encode_hello(70_000, 0x1122_3344_5566_7788),
            &mut schema_state,
            &mut protocol,
        ) {
            Err(err) => err,
            Ok(_) => panic!("oversized hello should fail"),
        };
        assert!(matches!(err, RuntimeError::SchemaTooLarge { .. }));

        let parsed = protocol.parse_packet(0x21, &1_u32.to_le_bytes());
        if let Err(ProtocolEngineError::UnknownPacketId(_)) = parsed {
            panic!("dynamic registry should remain available after invalid hello");
        }
        assert!(parsed.is_ok(), "dynamic packet parse should still work");
    }

    #[test]
    fn invalid_schema_commit_does_not_clear_existing_dynamic_registry() {
        let mut protocol = RatProtocolEngine::new();
        protocol
            .register_dynamic(DynamicPacketDef {
                id: 0x22,
                struct_name: "StablePacket".to_string(),
                packed: true,
                byte_size: 4,
                fields: vec![DynamicFieldDef {
                    name: "value".to_string(),
                    c_type: "uint32_t".to_string(),
                    offset: 0,
                    size: 4,
                }],
            })
            .expect("register dynamic packet");

        let schema = br#"
[[packets]]
id = 0
struct_name = "BadPacket"
type = "plot"
packed = true
byte_size = 4
fields = [{ name = "value", c_type = "uint32_t", offset = 0, size = 4 }]
"#;
        let schema_hash = rat_protocol::hash_schema_bytes(schema);
        let mut schema_state = SchemaState::new(Duration::from_secs(1));

        let hello = handle_control_payload(
            &encode_hello(schema.len() as u32, schema_hash),
            &mut schema_state,
            &mut protocol,
        )
        .expect("hello should pass");
        assert!(matches!(hello, ControlOutcome::SchemaReset));

        let chunk =
            handle_control_payload(&encode_chunk(0, schema), &mut schema_state, &mut protocol)
                .expect("schema chunk should pass");
        assert!(matches!(chunk, ControlOutcome::Noop));

        let err = match handle_control_payload(
            &encode_commit(schema_hash),
            &mut schema_state,
            &mut protocol,
        ) {
            Err(err) => err,
            Ok(_) => panic!("schema commit should fail"),
        };
        assert!(matches!(err, RuntimeError::ReservedPacketId { id: 0 }));

        let parsed = protocol.parse_packet(0x22, &1_u32.to_le_bytes());
        if let Err(ProtocolEngineError::UnknownPacketId(_)) = parsed {
            panic!("dynamic registry should remain available after invalid schema commit");
        }
        assert!(parsed.is_ok(), "dynamic packet parse should still work");
    }
}
