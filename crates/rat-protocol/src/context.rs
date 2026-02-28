use std::collections::{HashMap, HashSet};

use nom::number::complete::{
    le_f32, le_f64, le_i16, le_i32, le_i64, le_i8, le_u16, le_u32, le_u64, le_u8,
};
use nom::IResult;
use serde_json::{Map, Value};

use crate::{
    c_type_size, normalize_c_type, DynamicFieldDef, DynamicPacketDef, PacketData, ProtocolError,
};

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

    pub fn register_dynamic(&mut self, def: DynamicPacketDef) -> Result<(), ProtocolError> {
        if def.byte_size == 0 {
            return Err(ProtocolError::InvalidDynamicByteSize(def.byte_size));
        }
        if def.fields.is_empty() {
            return Err(ProtocolError::MissingDynamicFields);
        }

        let packet_id = def.id;
        let mut seen_field_names = HashSet::with_capacity(def.fields.len());

        let mut normalized = DynamicPacketDef {
            id: packet_id,
            struct_name: def.struct_name,
            packed: def.packed,
            byte_size: def.byte_size,
            fields: Vec::with_capacity(def.fields.len()),
        };

        for field in def.fields {
            if !seen_field_names.insert(field.name.clone()) {
                return Err(ProtocolError::DuplicateDynamicFieldName(field.name));
            }
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

        self.dynamic_registry.insert(packet_id, normalized);

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
