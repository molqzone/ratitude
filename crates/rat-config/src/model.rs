use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_CONFIG_PATH: &str = "rat.toml";
pub const DEFAULT_GENERATED_TOML_NAME: &str = "rat_gen.toml";
pub const DEFAULT_GENERATED_HEADER_NAME: &str = "rat_gen.h";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config: {0}")]
    Read(std::io::Error),
    #[error("failed to parse config: {0}")]
    Parse(toml::de::Error),
    #[error("failed to serialize config: {0}")]
    Serialize(toml::ser::Error),
    #[error("failed to create config directory: {0}")]
    Mkdir(std::io::Error),
    #[error("failed to write config: {0}")]
    Write(std::io::Error),
    #[error("config validation failed: {0}")]
    Validation(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RatitudeConfig {
    pub project: ProjectConfig,
    pub artifacts: ArtifactsConfig,
    pub generation: GenerationConfig,
    pub rttd: RttdConfig,
    #[serde(skip)]
    pub packets: Vec<PacketDef>,
}

impl Default for RatitudeConfig {
    fn default() -> Self {
        Self {
            project: ProjectConfig::default(),
            artifacts: ArtifactsConfig::default(),
            generation: GenerationConfig::default(),
            rttd: RttdConfig::default(),
            packets: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProjectConfig {
    pub name: String,
    pub scan_root: String,
    pub recursive: bool,
    pub extensions: Vec<String>,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            name: "stm32f4_rtt".to_string(),
            scan_root: ".".to_string(),
            recursive: true,
            extensions: vec![".h".to_string(), ".c".to_string()],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ArtifactsConfig {
    pub elf: String,
    pub hex: String,
    pub bin: String,
}

impl Default for ArtifactsConfig {
    fn default() -> Self {
        Self {
            elf: String::new(),
            hex: String::new(),
            bin: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GenerationConfig {
    pub out_dir: String,
    pub toml_name: String,
    pub header_name: String,
}

impl Default for GenerationConfig {
    fn default() -> Self {
        Self {
            out_dir: ".".to_string(),
            toml_name: DEFAULT_GENERATED_TOML_NAME.to_string(),
            header_name: DEFAULT_GENERATED_HEADER_NAME.to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RttdConfig {
    pub text_id: u16,
    pub source: RttdSourceConfig,
    pub behavior: RttdBehaviorConfig,
    pub outputs: RttdOutputsConfig,
}

impl Default for RttdConfig {
    fn default() -> Self {
        Self {
            text_id: 0xFF,
            source: RttdSourceConfig::default(),
            behavior: RttdBehaviorConfig::default(),
            outputs: RttdOutputsConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RttdSourceConfig {
    pub auto_scan: bool,
    pub scan_timeout_ms: u64,
    pub last_selected_addr: String,
}

impl Default for RttdSourceConfig {
    fn default() -> Self {
        Self {
            auto_scan: true,
            scan_timeout_ms: 300,
            last_selected_addr: "127.0.0.1:19021".to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RttdBehaviorConfig {
    pub reconnect: String,
    pub schema_timeout: String,
    pub buf: usize,
    pub reader_buf: usize,
}

impl Default for RttdBehaviorConfig {
    fn default() -> Self {
        Self {
            reconnect: "1s".to_string(),
            schema_timeout: "3s".to_string(),
            buf: 256,
            reader_buf: 65_536,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RttdOutputsConfig {
    pub jsonl: JsonlOutputConfig,
    pub foxglove: FoxgloveOutputConfig,
}

impl Default for RttdOutputsConfig {
    fn default() -> Self {
        Self {
            jsonl: JsonlOutputConfig::default(),
            foxglove: FoxgloveOutputConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct JsonlOutputConfig {
    pub enabled: bool,
    pub path: String,
}

impl Default for JsonlOutputConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            path: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct FoxgloveOutputConfig {
    pub enabled: bool,
    pub ws_addr: String,
}

impl Default for FoxgloveOutputConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ws_addr: "127.0.0.1:8765".to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PacketDef {
    pub id: u16,
    pub struct_name: String,
    #[serde(rename = "type")]
    pub packet_type: String,
    pub packed: bool,
    pub byte_size: usize,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub fields: Vec<FieldDef>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FieldDef {
    pub name: String,
    pub c_type: String,
    pub offset: usize,
    pub size: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GeneratedMeta {
    pub project: String,
    pub fingerprint: String,
}

impl Default for GeneratedMeta {
    fn default() -> Self {
        Self {
            project: String::new(),
            fingerprint: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GeneratedPacketDef {
    pub id: u16,
    pub signature_hash: String,
    pub struct_name: String,
    #[serde(rename = "type")]
    pub packet_type: String,
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
            packet_type: self.packet_type.clone(),
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
