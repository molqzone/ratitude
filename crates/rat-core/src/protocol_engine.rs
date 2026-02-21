use rat_protocol::{
    cobs_decode, DynamicFieldDef, DynamicPacketDef, PacketData, ProtocolContext, ProtocolError,
    RatPacket,
};
use thiserror::Error;

pub use rat_protocol::DynamicFieldDef as RuntimeDynamicFieldDef;

pub type PacketEnvelope = RatPacket;
pub type PacketPayload = PacketData;

#[derive(Debug, Error, Clone)]
pub enum ProtocolEngineError {
    #[error("unknown packet id: 0x{0:02X}")]
    UnknownPacketId(u8),
    #[error("protocol parse failed: {0}")]
    Parse(String),
    #[error("protocol register failed: {0}")]
    Register(String),
    #[error("cobs decode failed: {0}")]
    Decode(String),
}

pub trait ProtocolEngine: Send + Sync {
    fn parse_packet(&self, id: u8, payload: &[u8]) -> Result<PacketPayload, ProtocolEngineError>;
}

#[derive(Clone, Debug)]
pub struct RatProtocolEngine {
    context: ProtocolContext,
}

impl RatProtocolEngine {
    pub fn new() -> Self {
        Self {
            context: ProtocolContext::new(),
        }
    }

    pub fn set_text_packet_id(&mut self, id: u8) {
        self.context.set_text_packet_id(id);
    }

    pub fn clear_dynamic_registry(&mut self) {
        self.context.clear_dynamic_registry();
    }

    pub fn register_dynamic(&mut self, def: DynamicPacketDef) -> Result<(), ProtocolEngineError> {
        self.context
            .register_dynamic(def.id, def)
            .map_err(|err| ProtocolEngineError::Register(err.to_string()))
    }
}

impl Default for RatProtocolEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolEngine for RatProtocolEngine {
    fn parse_packet(&self, id: u8, payload: &[u8]) -> Result<PacketPayload, ProtocolEngineError> {
        self.context.parse_packet(id, payload).map_err(|err| match err {
            ProtocolError::UnknownPacketId(unknown) => ProtocolEngineError::UnknownPacketId(unknown),
            other => ProtocolEngineError::Parse(other.to_string()),
        })
    }
}

pub fn decode_frame(frame: &[u8]) -> Result<Vec<u8>, ProtocolEngineError> {
    cobs_decode(frame).map_err(|err| ProtocolEngineError::Decode(err.to_string()))
}

pub fn build_dynamic_packet_def(
    id: u8,
    struct_name: String,
    packed: bool,
    byte_size: usize,
    fields: Vec<DynamicFieldDef>,
) -> DynamicPacketDef {
    DynamicPacketDef {
        id,
        struct_name,
        packed,
        byte_size,
        fields,
    }
}
