use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_CONFIG_PATH: &str = "firmware/example/stm32f4_rtt/ratitude.toml";

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
    pub rttd: RttdConfig,
    pub packets: Vec<PacketDef>,
    #[serde(skip)]
    config_path: PathBuf,
    #[serde(skip)]
    scan_root_path: PathBuf,
}

impl Default for RatitudeConfig {
    fn default() -> Self {
        Self {
            project: ProjectConfig::default(),
            rttd: RttdConfig::default(),
            packets: Vec::new(),
            config_path: PathBuf::new(),
            scan_root_path: PathBuf::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProjectConfig {
    pub name: String,
    pub source_dir: Option<String>,
    pub scan_root: String,
    pub recursive: bool,
    pub extensions: Vec<String>,
    pub ignore_dirs: Vec<String>,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            name: "stm32f4_rtt".to_string(),
            source_dir: None,
            scan_root: ".".to_string(),
            recursive: true,
            extensions: vec![".h".to_string(), ".c".to_string()],
            ignore_dirs: vec![
                "Drivers".to_string(),
                ".git".to_string(),
                "build".to_string(),
            ],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RttdConfig {
    pub text_id: u16,
    pub server: ServerConfig,
    pub foxglove: FoxgloveConfig,
}

impl Default for RttdConfig {
    fn default() -> Self {
        Self {
            text_id: 0xFF,
            server: ServerConfig::default(),
            foxglove: FoxgloveConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ServerConfig {
    pub addr: String,
    pub reconnect: String,
    pub buf: usize,
    pub reader_buf: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            addr: "127.0.0.1:19021".to_string(),
            reconnect: "1s".to_string(),
            buf: 256,
            reader_buf: 65_536,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FoxgloveConfig {
    pub ws_addr: String,
    pub topic: String,
    pub schema_name: String,
    pub quat_id: u16,
    pub temp_id: u16,
    pub marker_topic: String,
    pub parent_frame: String,
    pub frame_id: String,
    pub image_path: String,
    pub image_frame: String,
    pub image_format: String,
    pub log_topic: String,
    pub log_name: String,
}

impl Default for FoxgloveConfig {
    fn default() -> Self {
        Self {
            ws_addr: "127.0.0.1:8765".to_string(),
            topic: "ratitude/packet".to_string(),
            schema_name: "ratitude.Packet".to_string(),
            quat_id: 0x10,
            temp_id: 0x20,
            marker_topic: "/visualization_marker".to_string(),
            parent_frame: "world".to_string(),
            frame_id: "base_link".to_string(),
            image_path: "D:/Repos/ratitude/demo.jpg".to_string(),
            image_frame: "camera".to_string(),
            image_format: "jpeg".to_string(),
            log_topic: "/ratitude/log".to_string(),
            log_name: "ratitude".to_string(),
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foxglove: Option<BTreeMap<String, toml::Value>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FieldDef {
    pub name: String,
    pub c_type: String,
    pub offset: usize,
    pub size: usize,
}

pub fn load_or_default(path: impl AsRef<Path>) -> Result<(RatitudeConfig, bool), ConfigError> {
    let path = normalize_config_path(path.as_ref());
    match fs::read_to_string(&path) {
        Ok(raw) => {
            let mut cfg: RatitudeConfig = toml::from_str(&raw).map_err(ConfigError::Parse)?;
            cfg.normalize(&path);
            cfg.validate()?;
            Ok((cfg, true))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let mut cfg = RatitudeConfig::default();
            cfg.normalize(&path);
            cfg.validate()?;
            Ok((cfg, false))
        }
        Err(err) => Err(ConfigError::Read(err)),
    }
}

pub fn load(path: impl AsRef<Path>) -> Result<RatitudeConfig, ConfigError> {
    let (cfg, exists) = load_or_default(path)?;
    if exists {
        Ok(cfg)
    } else {
        Err(ConfigError::Validation(
            "config file does not exist".to_string(),
        ))
    }
}

impl RatitudeConfig {
    pub fn save(&mut self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        let path = normalize_config_path(path.as_ref());
        self.normalize(&path);
        self.validate()?;
        self.packets.sort_by_key(|packet| packet.id);

        let out = toml::to_string_pretty(&self).map_err(ConfigError::Serialize)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(ConfigError::Mkdir)?;
        }
        fs::write(&path, out).map_err(ConfigError::Write)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.rttd.text_id > 0xFF {
            return Err(ConfigError::Validation(format!(
                "rttd.text_id out of range: 0x{:X}",
                self.rttd.text_id
            )));
        }
        if self.rttd.foxglove.quat_id > 0xFF {
            return Err(ConfigError::Validation(format!(
                "rttd.foxglove.quat_id out of range: 0x{:X}",
                self.rttd.foxglove.quat_id
            )));
        }
        if self.rttd.foxglove.temp_id > 0xFF {
            return Err(ConfigError::Validation(format!(
                "rttd.foxglove.temp_id out of range: 0x{:X}",
                self.rttd.foxglove.temp_id
            )));
        }

        let mut seen = std::collections::BTreeSet::new();
        for packet in &self.packets {
            if packet.id > 0xFF {
                return Err(ConfigError::Validation(format!(
                    "packet id out of range: 0x{:X}",
                    packet.id
                )));
            }
            if !seen.insert(packet.id) {
                return Err(ConfigError::Validation(format!(
                    "duplicate packet id: 0x{:02X}",
                    packet.id
                )));
            }
            if packet.struct_name.trim().is_empty() {
                return Err(ConfigError::Validation(format!(
                    "packet 0x{:02X} has empty struct_name",
                    packet.id
                )));
            }
            for field in &packet.fields {
                if field.name.trim().is_empty() {
                    return Err(ConfigError::Validation(format!(
                        "packet 0x{:02X} has field with empty name",
                        packet.id
                    )));
                }
                if field.size == 0 {
                    return Err(ConfigError::Validation(format!(
                        "packet 0x{:02X} field {} has invalid size",
                        packet.id, field.name
                    )));
                }
            }
        }

        Ok(())
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn scan_root_path(&self) -> &Path {
        &self.scan_root_path
    }

    pub fn normalize(&mut self, path: &Path) {
        if self.project.name.trim().is_empty() {
            self.project.name = ProjectConfig::default().name;
        }
        if self.project.scan_root.trim().is_empty() {
            if let Some(source_dir) = self.project.source_dir.as_ref() {
                self.project.scan_root = source_dir.clone();
            } else {
                self.project.scan_root = ProjectConfig::default().scan_root;
            }
        }
        self.project.source_dir = None;

        if self.project.extensions.is_empty() {
            self.project.extensions = ProjectConfig::default().extensions;
        }
        if self.project.ignore_dirs.is_empty() {
            self.project.ignore_dirs = ProjectConfig::default().ignore_dirs;
        }

        self.project.extensions = self
            .project
            .extensions
            .iter()
            .filter_map(|ext| {
                let trimmed = ext.trim();
                if trimmed.is_empty() {
                    None
                } else if trimmed.starts_with('.') {
                    Some(trimmed.to_ascii_lowercase())
                } else {
                    Some(format!(".{}", trimmed.to_ascii_lowercase()))
                }
            })
            .collect();

        self.config_path = normalize_config_path(path);
        let base_dir = self
            .config_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        let mut scan_root = PathBuf::from(&self.project.scan_root);
        if !scan_root.is_absolute() {
            scan_root = base_dir.join(scan_root);
        }
        if let Ok(abs) = scan_root.canonicalize() {
            scan_root = abs;
        } else {
            scan_root = scan_root.components().collect();
        }
        self.scan_root_path = scan_root;
    }
}

fn normalize_config_path(path: &Path) -> PathBuf {
    if path.as_os_str().is_empty() {
        PathBuf::from(DEFAULT_CONFIG_PATH)
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scan_root_is_relative() {
        let mut cfg = RatitudeConfig::default();
        let path = PathBuf::from("tmp/ratitude.toml");
        cfg.normalize(&path);
        assert!(cfg.scan_root_path.ends_with("tmp"));
    }
}
