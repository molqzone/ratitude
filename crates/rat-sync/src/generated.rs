use rat_config::PacketDef;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct GeneratedMeta {
    pub project: String,
    pub schema_hash: String,
}

pub type GeneratedPacketDef = PacketDef;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct GeneratedConfig {
    pub meta: GeneratedMeta,
    pub packets: Vec<GeneratedPacketDef>,
}

impl GeneratedConfig {
    pub fn to_packet_defs(&self) -> Vec<PacketDef> {
        self.packets.clone()
    }
}
