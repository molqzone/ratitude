use std::collections::HashMap;
use std::fmt;
use std::time::SystemTime;

use nom::number::complete::{
    le_f32, le_f64, le_i16, le_i32, le_i64, le_i8, le_u16, le_u32, le_u64, le_u8,
};
use nom::IResult;
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

#[derive(Clone, Debug)]
pub struct ProtocolContext {
    text_packet_id: u8,
    dynamic_registry: HashMap<u8, DynamicPacketDef>,
}

impl Default for ProtocolContext {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolContext {
    pub fn new() -> Self {
        Self {
            text_packet_id: 0xFF,
            dynamic_registry: HashMap::new(),
        }
    }

    pub fn set_text_packet_id(&mut self, id: u8) {
        self.text_packet_id = id;
    }

    pub fn text_packet_id(&self) -> u8 {
        self.text_packet_id
    }

    pub fn clear_dynamic_registry(&mut self) {
        self.dynamic_registry.clear();
    }

    pub fn register_dynamic(&mut self, id: u8, def: DynamicPacketDef) -> Result<(), ProtocolError> {
        if def.byte_size == 0 {
            return Err(ProtocolError::InvalidDynamicByteSize(def.byte_size));
        }
        if def.fields.is_empty() {
            return Err(ProtocolError::MissingDynamicFields);
        }

        let mut normalized = DynamicPacketDef {
            id,
            struct_name: def.struct_name,
            packed: def.packed,
            byte_size: def.byte_size,
            fields: Vec::with_capacity(def.fields.len()),
        };

        for field in def.fields {
            let c_type = normalize_c_type(&field.c_type);
            let expected_size = c_type_size(&c_type)
                .ok_or_else(|| ProtocolError::UnsupportedCType(field.c_type.clone()))?;
            if field.size != expected_size {
                return Err(ProtocolError::DynamicFieldSizeMismatch {
                    name: field.name,
                    got: field.size,
                    expected: expected_size,
                });
            }
            let field_end = field.offset.checked_add(field.size).ok_or_else(|| {
                ProtocolError::DynamicFieldOffset {
                    name: field.name.clone(),
                    offset: field.offset,
                }
            })?;
            if field_end > normalized.byte_size {
                return Err(ProtocolError::DynamicFieldOutOfRange { name: field.name });
            }
            normalized.fields.push(DynamicFieldDef {
                name: field.name,
                c_type,
                offset: field.offset,
                size: field.size,
            });
        }

        self.dynamic_registry.insert(id, normalized);

        Ok(())
    }

    pub fn parse_packet(&self, id: u8, payload: &[u8]) -> Result<PacketData, ProtocolError> {
        if id == self.text_packet_id() {
            return Ok(PacketData::Text(parse_text(payload)));
        }

        if let Some(decoded) = self.parse_dynamic_packet(id, payload)? {
            return Ok(PacketData::Dynamic(decoded));
        }

        Err(ProtocolError::UnknownPacketId(id))
    }

    fn parse_dynamic_packet(
        &self,
        id: u8,
        payload: &[u8],
    ) -> Result<Option<Map<String, Value>>, ProtocolError> {
        let Some(def) = self.dynamic_registry.get(&id) else {
            return Ok(None);
        };

        if payload.len() != def.byte_size {
            return Err(ProtocolError::PayloadSizeMismatch {
                id,
                got: payload.len(),
                expected: def.byte_size,
            });
        }

        let mut out = Map::new();
        for field in &def.fields {
            let start = field.offset;
            let end = field.offset.checked_add(field.size).ok_or_else(|| {
                ProtocolError::DynamicFieldOffset {
                    name: field.name.clone(),
                    offset: field.offset,
                }
            })?;
            let slice =
                payload
                    .get(start..end)
                    .ok_or_else(|| ProtocolError::DynamicFieldOutOfRange {
                        name: field.name.clone(),
                    })?;
            let value = decode_dynamic_value(field, slice)?;
            out.insert(field.name.clone(), value);
        }

        Ok(Some(out))
    }
}

pub fn parse_text(payload: &[u8]) -> String {
    let mut end = payload.len();
    for (index, value) in payload.iter().enumerate() {
        if *value == 0 {
            end = index;
            break;
        }
    }
    String::from_utf8_lossy(&payload[..end])
        .trim_end_matches('\0')
        .to_string()
}

fn parse_nom_exact<'a, O, F>(input: &'a [u8], mut parser: F) -> Option<O>
where
    F: FnMut(&'a [u8]) -> IResult<&'a [u8], O>,
{
    let (rest, value) = parser(input).ok()?;
    if rest.is_empty() {
        Some(value)
    } else {
        None
    }
}

fn decode_dynamic_value(field: &DynamicFieldDef, data: &[u8]) -> Result<Value, ProtocolError> {
    let parse_err = || ProtocolError::DynamicFieldSizeMismatch {
        name: field.name.clone(),
        got: data.len(),
        expected: field.size,
    };

    let value = match field.c_type.as_str() {
        "float" => Value::from(parse_nom_exact(data, le_f32).ok_or_else(parse_err)? as f64),
        "double" => Value::from(parse_nom_exact(data, le_f64).ok_or_else(parse_err)?),
        "int8_t" => Value::from(parse_nom_exact(data, le_i8).ok_or_else(parse_err)? as i64),
        "uint8_t" => Value::from(parse_nom_exact(data, le_u8).ok_or_else(parse_err)? as u64),
        "int16_t" => Value::from(parse_nom_exact(data, le_i16).ok_or_else(parse_err)? as i64),
        "uint16_t" => Value::from(parse_nom_exact(data, le_u16).ok_or_else(parse_err)? as u64),
        "int32_t" => Value::from(parse_nom_exact(data, le_i32).ok_or_else(parse_err)? as i64),
        "uint32_t" => Value::from(parse_nom_exact(data, le_u32).ok_or_else(parse_err)? as u64),
        "int64_t" => Value::from(parse_nom_exact(data, le_i64).ok_or_else(parse_err)?),
        "uint64_t" => Value::from(parse_nom_exact(data, le_u64).ok_or_else(parse_err)?),
        "bool" | "_bool" => Value::from(parse_nom_exact(data, le_u8).ok_or_else(parse_err)? != 0),
        other => return Err(ProtocolError::UnsupportedCType(other.to_string())),
    };
    Ok(value)
}

fn c_type_size(c_type: &str) -> Option<usize> {
    match c_type {
        "float" => Some(4),
        "double" => Some(8),
        "int8_t" | "uint8_t" | "bool" | "_bool" => Some(1),
        "int16_t" | "uint16_t" => Some(2),
        "int32_t" | "uint32_t" => Some(4),
        "int64_t" | "uint64_t" => Some(8),
        _ => None,
    }
}

fn normalize_c_type(raw: &str) -> String {
    let mut value = raw.trim().to_ascii_lowercase();
    while value.contains("  ") {
        value = value.replace("  ", " ");
    }
    value = value
        .trim_start_matches("const ")
        .trim_start_matches("volatile ")
        .to_string();
    value.trim().to_string()
}

pub fn cobs_decode(frame: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    if frame.is_empty() {
        return Ok(Vec::new());
    }

    cobs::decode_vec(frame).map_err(|_| {
        if frame.contains(&0) {
            ProtocolError::InvalidCobsCode
        } else {
            ProtocolError::TruncatedFrame
        }
    })
}

pub fn hash_schema_bytes(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_text_stops_at_null() {
        assert_eq!(parse_text(b"abc\0def"), "abc");
    }

    #[test]
    fn packet_type_parse_is_case_insensitive() {
        assert_eq!(PacketType::parse("plot"), Some(PacketType::Plot));
        assert_eq!(PacketType::parse(" QuAt "), Some(PacketType::Quat));
        assert_eq!(PacketType::parse("unknown"), None);
    }

    #[test]
    fn cobs_simple() {
        assert_eq!(
            cobs_decode(&[0x03, 0x11, 0x22]).expect("decode"),
            vec![0x11, 0x22]
        );
    }

    #[test]
    fn text_id_isolation_between_contexts() {
        let mut ctx_a = ProtocolContext::new();
        ctx_a.set_text_packet_id(0x01);
        let mut ctx_b = ProtocolContext::new();
        ctx_b.set_text_packet_id(0x02);

        let data_a = ctx_a.parse_packet(0x01, b"abc").expect("ctx_a parse");
        let data_b = ctx_b.parse_packet(0x01, b"abc");

        assert!(matches!(data_a, PacketData::Text(_)));
        assert!(matches!(data_b, Err(ProtocolError::UnknownPacketId(0x01))));
    }

    #[test]
    fn dynamic_registry_isolation_between_contexts() {
        let mut ctx_a = ProtocolContext::new();
        let ctx_b = ProtocolContext::new();
        ctx_a
            .register_dynamic(
                0x20,
                DynamicPacketDef {
                    id: 0x20,
                    struct_name: "Demo".to_string(),
                    packed: true,
                    byte_size: 4,
                    fields: vec![DynamicFieldDef {
                        name: "value".to_string(),
                        c_type: "int32_t".to_string(),
                        offset: 0,
                        size: 4,
                    }],
                },
            )
            .expect("register dynamic");

        let payload = 42_i32.to_le_bytes();

        let data_a = ctx_a.parse_packet(0x20, &payload).expect("ctx_a parse");
        let data_b = ctx_b.parse_packet(0x20, &payload);

        match data_a {
            PacketData::Dynamic(map) => {
                assert_eq!(map.get("value").and_then(Value::as_i64), Some(42));
            }
            other => panic!("unexpected packet kind: {other:?}"),
        }
        assert!(matches!(data_b, Err(ProtocolError::UnknownPacketId(0x20))));
    }

    #[test]
    fn unknown_packet_id_returns_error() {
        let ctx = ProtocolContext::new();
        let err = ctx
            .parse_packet(0x42, &[0x01, 0x02])
            .expect_err("should fail");
        assert!(matches!(err, ProtocolError::UnknownPacketId(0x42)));
    }

    #[test]
    fn schema_hash_is_stable() {
        assert_eq!(
            hash_schema_bytes(b"abc"),
            0xE71FA2190541574B,
            "schema hash must stay stable across crates"
        );
    }
}
