use rat_protocol::{cobs_decode, PacketData, ProtocolContext, ProtocolError, RatPacket};
use thiserror::Error;

pub type PacketEnvelope = RatPacket;
pub type PacketPayload = PacketData;

#[derive(Clone, Debug)]
pub(crate) struct RuntimeDynamicFieldDef {
    pub name: String,
    pub c_type: String,
    pub offset: usize,
    pub size: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeDynamicPacketDef {
    pub id: u8,
    pub struct_name: String,
    pub packed: bool,
    pub byte_size: usize,
    pub fields: Vec<RuntimeDynamicFieldDef>,
}

#[derive(Debug, Error, Clone)]
pub(crate) enum ProtocolEngineError {
    #[error("unknown packet id: 0x{0:02X}")]
    UnknownPacketId(u8),
    #[error("protocol parse failed: {0}")]
    Parse(String),
    #[error("protocol register failed: {0}")]
    Register(String),
    #[error("cobs decode failed: {0}")]
    Decode(String),
}

#[derive(Clone, Debug)]
pub(crate) struct RatProtocolEngine {
    context: ProtocolContext,
}

impl RatProtocolEngine {
    pub(crate) fn new() -> Self {
        Self {
            context: ProtocolContext::new(),
        }
    }

    fn register_dynamic_impl(
        &mut self,
        def: RuntimeDynamicPacketDef,
    ) -> Result<(), ProtocolEngineError> {
        let packet_def = rat_protocol::DynamicPacketDef {
            id: def.id,
            struct_name: def.struct_name,
            packed: def.packed,
            byte_size: def.byte_size,
            fields: def
                .fields
                .into_iter()
                .map(|field| rat_protocol::DynamicFieldDef {
                    name: field.name,
                    c_type: field.c_type,
                    offset: field.offset,
                    size: field.size,
                })
                .collect(),
        };

        self.context
            .register_dynamic(packet_def.id, packet_def)
            .map_err(|err| ProtocolEngineError::Register(err.to_string()))
    }

    pub(crate) fn set_text_packet_id(&mut self, id: u8) {
        self.context.set_text_packet_id(id);
    }

    pub(crate) fn clear_dynamic_registry(&mut self) {
        self.context.clear_dynamic_registry();
    }

    pub(crate) fn register_dynamic(
        &mut self,
        def: RuntimeDynamicPacketDef,
    ) -> Result<(), ProtocolEngineError> {
        self.register_dynamic_impl(def)
    }

    pub(crate) fn parse_packet(
        &self,
        id: u8,
        payload: &[u8],
    ) -> Result<PacketPayload, ProtocolEngineError> {
        self.context
            .parse_packet(id, payload)
            .map_err(|err| match err {
                ProtocolError::UnknownPacketId(unknown) => {
                    ProtocolEngineError::UnknownPacketId(unknown)
                }
                other => ProtocolEngineError::Parse(other.to_string()),
            })
    }
}

pub(crate) fn decode_frame(frame: &[u8]) -> Result<Vec<u8>, ProtocolEngineError> {
    cobs_decode(frame).map_err(|err| ProtocolEngineError::Decode(err.to_string()))
}

impl Default for RatProtocolEngine {
    fn default() -> Self {
        Self::new()
    }
}
