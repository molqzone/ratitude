use std::fmt;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("invalid COBS code 0x00")]
    InvalidCobsCode,
    #[error("cobs frame truncated")]
    TruncatedFrame,
    #[error("payload size mismatch for id 0x{id:02X}: got {got}, expected {expected}")]
    PayloadSizeMismatch { id: u8, got: usize, expected: usize },
    #[error("dynamic packet requires at least one field")]
    MissingDynamicFields,
    #[error("duplicate dynamic field name: {0}")]
    DuplicateDynamicFieldName(String),
    #[error("dynamic packet has invalid byte size: {0}")]
    InvalidDynamicByteSize(usize),
    #[error("unsupported c type: {0}")]
    UnsupportedCType(String),
    #[error("dynamic field size mismatch for {name}: got {got}, expected {expected}")]
    DynamicFieldSizeMismatch {
        name: String,
        got: usize,
        expected: usize,
    },
    #[error("dynamic field {name} exceeds packet size")]
    DynamicFieldOutOfRange { name: String },
    #[error("dynamic field {current} overlaps with {previous}")]
    DynamicFieldOverlap { current: String, previous: String },
    #[error("dynamic field {name} has invalid offset {offset}")]
    DynamicFieldOffset { name: String, offset: usize },
    #[error("unknown packet id: 0x{0:02X}")]
    UnknownPacketId(u8),
}

#[derive(Clone, Debug)]
pub enum PacketData {
    Text(String),
    Dynamic(Map<String, Value>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PacketType {
    Plot,
    Quat,
    Image,
    Log,
}

impl PacketType {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "plot" => Some(Self::Plot),
            "quat" => Some(Self::Quat),
            "image" => Some(Self::Image),
            "log" => Some(Self::Log),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plot => "plot",
            Self::Quat => "quat",
            Self::Image => "image",
            Self::Log => "log",
        }
    }
}

impl fmt::Display for PacketType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug)]
pub struct RatPacket {
    pub id: u8,
    pub timestamp: SystemTime,
    pub payload: Vec<u8>,
    pub data: PacketData,
}

#[derive(Clone, Debug)]
pub struct DynamicFieldDef {
    pub name: String,
    pub c_type: String,
    pub offset: usize,
    pub size: usize,
}

#[derive(Clone, Debug)]
pub struct DynamicPacketDef {
    pub id: u8,
    pub struct_name: String,
    pub packed: bool,
    pub byte_size: usize,
    pub fields: Vec<DynamicFieldDef>,
}
