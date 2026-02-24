use rat_protocol::hash_schema_bytes;
use rat_protocol::PacketType;
use serde::Deserialize;

use super::{RuntimeError, RuntimePacketDef};

const MAX_SCHEMA_BYTES: usize = 64 * 1024;

pub(crate) struct SchemaReadyPayload {
    pub(crate) schema_hash: u64,
    pub(crate) packets: Vec<RuntimePacketDef>,
}

#[derive(Debug)]
pub(crate) struct SchemaAssembly {
    total_len: usize,
    expected_hash: u64,
    bytes: Vec<u8>,
}

impl SchemaAssembly {
    pub(crate) fn new(total_len: usize, expected_hash: u64) -> Result<Self, RuntimeError> {
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

    pub(crate) fn append(&mut self, offset: usize, chunk: &[u8]) -> Result<(), RuntimeError> {
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

    pub(crate) fn bytes_len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn total_len(&self) -> usize {
        self.total_len
    }

    pub(crate) fn finalize(self, commit_hash: u64) -> Result<SchemaReadyPayload, RuntimeError> {
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeSchemaDocument {
    #[serde(default)]
    packets: Vec<RuntimeSchemaPacket>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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

    doc.packets
        .into_iter()
        .map(|packet| {
            let packet_type = normalize_runtime_packet_type(&packet.packet_type)?;
            Ok(RuntimePacketDef {
                id: packet.id,
                struct_name: packet.struct_name,
                packet_type,
                packed: packet.packed,
                byte_size: packet.byte_size,
                source: "runtime-schema".to_string(),
                fields: packet
                    .fields
                    .into_iter()
                    .map(|field| rat_config::FieldDef {
                        name: field.name,
                        c_type: field.c_type,
                        offset: field.offset,
                        size: field.size,
                    })
                    .collect(),
            })
        })
        .collect()
}

fn normalize_runtime_packet_type(raw: &str) -> Result<PacketType, RuntimeError> {
    PacketType::parse(raw).ok_or_else(|| RuntimeError::SchemaParseFailed {
        reason: format!("unsupported packet type in runtime schema: {raw}"),
    })
}
