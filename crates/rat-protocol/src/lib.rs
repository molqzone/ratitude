use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::RwLock;
use std::time::SystemTime;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("invalid COBS code 0x00")]
    InvalidCobsCode,
    #[error("cobs frame truncated")]
    TruncatedFrame,
    #[error("unsupported type size for id 0x{0:02X}")]
    UnsupportedTypeSize(u8),
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuatPacket {
    pub w: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TemperaturePacket {
    pub celsius: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawPacket {
    pub id: String,
    pub payload_hex: String,
}

#[derive(Clone, Debug)]
pub enum PacketData {
    Text(String),
    Dynamic(Map<String, Value>),
    Quat(QuatPacket),
    Temperature(TemperaturePacket),
    Raw(RawPacket),
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
enum StaticPacketKind {
    Quat,
    Temperature,
}

static TEXT_PACKET_ID: AtomicU8 = AtomicU8::new(0xFF);
static STATIC_REGISTRY: Lazy<RwLock<HashMap<u8, StaticPacketKind>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));
static DYNAMIC_REGISTRY: Lazy<RwLock<HashMap<u8, DynamicPacketDef>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

pub fn set_text_packet_id(id: u8) {
    TEXT_PACKET_ID.store(id, Ordering::Relaxed);
}

pub fn text_packet_id() -> u8 {
    TEXT_PACKET_ID.load(Ordering::Relaxed)
}

pub fn clear_static_registry() {
    if let Ok(mut guard) = STATIC_REGISTRY.write() {
        guard.clear();
    }
}

pub fn register_static_quat(id: u8) {
    if let Ok(mut guard) = STATIC_REGISTRY.write() {
        guard.insert(id, StaticPacketKind::Quat);
    }
}

pub fn register_static_temperature(id: u8) {
    if let Ok(mut guard) = STATIC_REGISTRY.write() {
        guard.insert(id, StaticPacketKind::Temperature);
    }
}

pub fn clear_dynamic_registry() {
    if let Ok(mut guard) = DYNAMIC_REGISTRY.write() {
        guard.clear();
    }
}

pub fn register_dynamic(id: u8, def: DynamicPacketDef) -> Result<(), ProtocolError> {
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
        if field.offset + field.size > normalized.byte_size {
            return Err(ProtocolError::DynamicFieldOutOfRange { name: field.name });
        }
        normalized.fields.push(DynamicFieldDef {
            name: field.name,
            c_type,
            offset: field.offset,
            size: field.size,
        });
    }

    if let Ok(mut guard) = DYNAMIC_REGISTRY.write() {
        guard.insert(id, normalized);
    }

    Ok(())
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

pub fn parse_packet(id: u8, payload: &[u8]) -> Result<PacketData, ProtocolError> {
    if id == text_packet_id() {
        return Ok(PacketData::Text(parse_text(payload)));
    }

    if let Some(decoded) = parse_dynamic_packet(id, payload)? {
        return Ok(PacketData::Dynamic(decoded));
    }

    if let Some(kind) = STATIC_REGISTRY
        .read()
        .ok()
        .and_then(|guard| guard.get(&id).cloned())
    {
        return match kind {
            StaticPacketKind::Quat => {
                if payload.len() != 16 {
                    Err(ProtocolError::PayloadSizeMismatch {
                        id,
                        got: payload.len(),
                        expected: 16,
                    })
                } else {
                    Ok(PacketData::Quat(QuatPacket {
                        w: f32::from_bits(u32::from_le_bytes(
                            payload[0..4].try_into().expect("quat slice"),
                        )),
                        x: f32::from_bits(u32::from_le_bytes(
                            payload[4..8].try_into().expect("quat slice"),
                        )),
                        y: f32::from_bits(u32::from_le_bytes(
                            payload[8..12].try_into().expect("quat slice"),
                        )),
                        z: f32::from_bits(u32::from_le_bytes(
                            payload[12..16].try_into().expect("quat slice"),
                        )),
                    }))
                }
            }
            StaticPacketKind::Temperature => {
                if payload.len() != 4 {
                    Err(ProtocolError::PayloadSizeMismatch {
                        id,
                        got: payload.len(),
                        expected: 4,
                    })
                } else {
                    Ok(PacketData::Temperature(TemperaturePacket {
                        celsius: f32::from_bits(u32::from_le_bytes(
                            payload[0..4].try_into().expect("temp slice"),
                        )),
                    }))
                }
            }
        };
    }

    Ok(PacketData::Raw(RawPacket {
        id: format!("0x{:02x}", id),
        payload_hex: hex::encode(payload),
    }))
}

fn parse_dynamic_packet(
    id: u8,
    payload: &[u8],
) -> Result<Option<Map<String, Value>>, ProtocolError> {
    let Some(def) = DYNAMIC_REGISTRY
        .read()
        .ok()
        .and_then(|guard| guard.get(&id).cloned())
    else {
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
        let end = field.offset + field.size;
        let slice = &payload[start..end];
        let value = decode_dynamic_value(&field.c_type, slice)?;
        out.insert(field.name.clone(), value);
    }

    Ok(Some(out))
}

fn decode_dynamic_value(c_type: &str, data: &[u8]) -> Result<Value, ProtocolError> {
    let value = match c_type {
        "float" => Value::from(f32::from_bits(u32::from_le_bytes(
            data.try_into().expect("float slice"),
        )) as f64),
        "double" => Value::from(f64::from_bits(u64::from_le_bytes(
            data.try_into().expect("double slice"),
        ))),
        "int8_t" => Value::from(i8::from_le_bytes([data[0]]) as i64),
        "uint8_t" => Value::from(u8::from_le_bytes([data[0]]) as u64),
        "int16_t" => Value::from(i16::from_le_bytes(data.try_into().expect("int16 slice")) as i64),
        "uint16_t" => {
            Value::from(u16::from_le_bytes(data.try_into().expect("uint16 slice")) as u64)
        }
        "int32_t" => Value::from(i32::from_le_bytes(data.try_into().expect("int32 slice")) as i64),
        "uint32_t" => {
            Value::from(u32::from_le_bytes(data.try_into().expect("uint32 slice")) as u64)
        }
        "int64_t" => Value::from(i64::from_le_bytes(data.try_into().expect("int64 slice"))),
        "uint64_t" => Value::from(u64::from_le_bytes(data.try_into().expect("uint64 slice"))),
        "bool" | "_bool" => Value::from(data[0] != 0),
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

    let mut out = Vec::with_capacity(frame.len());
    let mut index = 0usize;
    while index < frame.len() {
        let code = frame[index];
        if code == 0 {
            return Err(ProtocolError::InvalidCobsCode);
        }
        index += 1;

        let count = code as usize - 1;
        if index + count > frame.len() {
            return Err(ProtocolError::TruncatedFrame);
        }

        out.extend_from_slice(&frame[index..index + count]);
        index += count;

        if code != 0xFF && index < frame.len() {
            out.push(0);
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_text_stops_at_null() {
        assert_eq!(parse_text(b"abc\0def"), "abc");
    }

    #[test]
    fn cobs_simple() {
        assert_eq!(
            cobs_decode(&[0x03, 0x11, 0x22]).expect("decode"),
            vec![0x11, 0x22]
        );
    }
}
