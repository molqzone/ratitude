use rat_config::{FieldDef, PacketType, RatitudeConfig};

use crate::generated::{GeneratedConfig, GeneratedPacketDef};

#[derive(Debug, Clone)]
pub struct DiscoveredPacket {
    pub signature_hash: u64,
    pub struct_name: String,
    pub packet_type: PacketType,
    pub packed: bool,
    pub byte_size: usize,
    pub source: String,
    pub fields: Vec<FieldDef>,
}

#[derive(Debug, Clone)]
pub(crate) struct TagMatch {
    pub(crate) end_byte: usize,
    pub(crate) packet_type: PacketType,
    pub(crate) line: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct StructDef {
    pub(crate) start_byte: usize,
    pub(crate) name: String,
    pub(crate) packed: bool,
    pub(crate) byte_size: usize,
    pub(crate) fields: Vec<FieldDef>,
}

#[derive(Debug, Clone)]
pub(crate) struct FieldSpec {
    pub(crate) name: String,
    pub(crate) c_type: String,
    pub(crate) size: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedTaggedFile {
    pub(crate) tags: Vec<TagMatch>,
    pub(crate) structs: Vec<StructDef>,
}

#[derive(Debug, Clone)]
pub struct SyncPipelineInput {
    pub project_name: String,
    pub discovered_packets: Vec<DiscoveredPacket>,
}

#[derive(Debug, Clone)]
pub struct SyncPipelineOutput {
    pub generated: GeneratedConfig,
    pub layout_warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SyncFsResult {
    pub config: RatitudeConfig,
    pub generated: GeneratedConfig,
    pub packet_defs: Vec<GeneratedPacketDef>,
    pub layout_warnings: Vec<String>,
}
