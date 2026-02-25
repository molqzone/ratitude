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
    protocol.clear_dynamic_registry();
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

        protocol
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
