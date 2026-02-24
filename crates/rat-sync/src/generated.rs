use rat_config::{FieldDef, PacketDef, PacketType};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct GeneratedMeta {
    pub project: String,
    pub schema_hash: String,
}

impl Default for GeneratedMeta {
    fn default() -> Self {
        Self {
            project: String::new(),
            schema_hash: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GeneratedPacketDef {
    pub id: u16,
    pub signature_hash: String,
    pub struct_name: String,
    #[serde(rename = "type")]
    pub packet_type: PacketType,
    pub packed: bool,
    pub byte_size: usize,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub fields: Vec<FieldDef>,
}

impl GeneratedPacketDef {
    pub fn to_packet_def(&self) -> PacketDef {
        PacketDef {
            id: self.id,
            struct_name: self.struct_name.clone(),
            packet_type: self.packet_type,
            packed: self.packed,
            byte_size: self.byte_size,
            source: self.source.clone(),
            fields: self.fields.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct GeneratedConfig {
    pub meta: GeneratedMeta,
    pub packets: Vec<GeneratedPacketDef>,
}

impl GeneratedConfig {
    pub fn to_packet_defs(&self) -> Vec<PacketDef> {
        self.packets
            .iter()
            .map(GeneratedPacketDef::to_packet_def)
            .collect()
    }
}
