use std::path::PathBuf;

use rat_protocol::PacketType;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_CONFIG_PATH: &str = "rat.toml";
pub const DEFAULT_GENERATED_HEADER_NAME: &str = "rat_gen.h";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file does not exist: {}", .0.display())]
    NotFound(PathBuf),
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RatitudeConfig {
    pub project: ProjectConfig,
    pub artifacts: ArtifactsConfig,
    pub generation: GenerationConfig,
    pub ratd: RatdConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ArtifactsConfig {
    pub elf: String,
    pub hex: String,
    pub bin: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct GenerationConfig {
    pub out_dir: String,
    pub header_name: String,
}

impl Default for GenerationConfig {
    fn default() -> Self {
        Self {
            out_dir: ".".to_string(),
            header_name: DEFAULT_GENERATED_HEADER_NAME.to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RatdConfig {
    pub text_id: u16,
    pub source: RatdSourceConfig,
    pub behavior: RatdBehaviorConfig,
    pub outputs: RatdOutputsConfig,
}

impl Default for RatdConfig {
    fn default() -> Self {
        Self {
            text_id: 0xFF,
            source: RatdSourceConfig::default(),
            behavior: RatdBehaviorConfig::default(),
            outputs: RatdOutputsConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RatdSourceConfig {
    pub auto_scan: bool,
    pub scan_timeout_ms: u64,
    pub last_selected_addr: String,
    pub seed_addrs: Vec<String>,
}

impl Default for RatdSourceConfig {
    fn default() -> Self {
        Self {
            auto_scan: true,
            scan_timeout_ms: 300,
            last_selected_addr: "127.0.0.1:19021".to_string(),
            seed_addrs: vec![
                "127.0.0.1:19021".to_string(),
                "127.0.0.1:2331".to_string(),
                "127.0.0.1:9090".to_string(),
            ],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RatdBehaviorConfig {
    pub reconnect: String,
    pub schema_timeout: String,
    pub buf: usize,
    pub reader_buf: usize,
}

impl Default for RatdBehaviorConfig {
    fn default() -> Self {
        Self {
            reconnect: "1s".to_string(),
            schema_timeout: "3s".to_string(),
            buf: 256,
            reader_buf: 65_536,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RatdOutputsConfig {
    pub jsonl: JsonlOutputConfig,
    pub foxglove: FoxgloveOutputConfig,
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
    pub packet_type: PacketType,
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
