use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

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
    #[serde(skip)]
    config_path: PathBuf,
    #[serde(skip)]
    scan_root_path: PathBuf,
    #[serde(skip)]
    generated_toml_path: PathBuf,
    #[serde(skip)]
    generated_header_path: PathBuf,
}

impl Default for RatitudeConfig {
    fn default() -> Self {
        Self {
            project: ProjectConfig::default(),
            artifacts: ArtifactsConfig::default(),
            generation: GenerationConfig::default(),
            rttd: RttdConfig::default(),
            packets: Vec::new(),
            config_path: PathBuf::new(),
            scan_root_path: PathBuf::new(),
            generated_toml_path: PathBuf::new(),
            generated_header_path: PathBuf::new(),
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendType {
    None,
    Openocd,
    Jlink,
}

impl Default for BackendType {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BackendConfig {
    #[serde(rename = "type")]
    pub backend_type: BackendType,
    pub auto_start: bool,
    pub startup_timeout_ms: u64,
    pub openocd: OpenOcdBackendConfig,
    pub jlink: JlinkBackendConfig,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            backend_type: BackendType::None,
            auto_start: false,
            startup_timeout_ms: 5_000,
            openocd: OpenOcdBackendConfig::default(),
            jlink: JlinkBackendConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OpenOcdBackendConfig {
    pub elf: String,
    pub symbol: String,
    pub interface: String,
    pub target: String,
    pub transport: String,
    pub speed: u32,
    pub polling: u32,
    pub disable_debug_ports: bool,
}

impl Default for OpenOcdBackendConfig {
    fn default() -> Self {
        Self {
            elf: String::new(),
            symbol: "_SEGGER_RTT".to_string(),
            interface: "interface/cmsis-dap.cfg".to_string(),
            target: "target/stm32f4x.cfg".to_string(),
            transport: "swd".to_string(),
            speed: 8_000,
            polling: 1,
            disable_debug_ports: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct JlinkBackendConfig {
    pub device: String,
    pub interface: String,
    pub speed: u32,
    pub serial: String,
    pub ip: String,
}

impl Default for JlinkBackendConfig {
    fn default() -> Self {
        Self {
            device: "STM32F407ZG".to_string(),
            interface: "SWD".to_string(),
            speed: 4_000,
            serial: String::new(),
            ip: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RttdSourceConfig {
    pub auto_scan: bool,
    pub scan_timeout_ms: u64,
    pub last_selected_addr: String,
    pub backend: BackendConfig,
}

impl Default for RttdSourceConfig {
    fn default() -> Self {
        Self {
            auto_scan: true,
            scan_timeout_ms: 300,
            last_selected_addr: "127.0.0.1:19021".to_string(),
            backend: BackendConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RttdBehaviorConfig {
    pub auto_sync_on_start: bool,
    pub auto_sync_on_reset: bool,
    pub sync_debounce_ms: u64,
    pub reconnect: String,
    pub buf: usize,
    pub reader_buf: usize,
}

impl Default for RttdBehaviorConfig {
    fn default() -> Self {
        Self {
            auto_sync_on_start: true,
            auto_sync_on_reset: true,
            sync_debounce_ms: 500,
            reconnect: "1s".to_string(),
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

pub fn load_or_default(path: impl AsRef<Path>) -> Result<(RatitudeConfig, bool), ConfigError> {
    let path = normalize_config_path(path.as_ref());
    match fs::read_to_string(&path) {
        Ok(raw) => {
            reject_deprecated_config_keys(&raw)?;
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

pub fn load_generated_or_default(
    path: impl AsRef<Path>,
) -> Result<(GeneratedConfig, bool), ConfigError> {
    let path = path.as_ref();
    match fs::read_to_string(path) {
        Ok(raw) => {
            let cfg: GeneratedConfig = toml::from_str(&raw).map_err(ConfigError::Parse)?;
            Ok((cfg, true))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok((GeneratedConfig::default(), false))
        }
        Err(err) => Err(ConfigError::Read(err)),
    }
}

pub fn save_generated(path: impl AsRef<Path>, cfg: &GeneratedConfig) -> Result<(), ConfigError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(ConfigError::Mkdir)?;
    }
    let out = toml::to_string_pretty(cfg).map_err(ConfigError::Serialize)?;
    fs::write(path, out).map_err(ConfigError::Write)
}

impl RatitudeConfig {
    pub fn save(&mut self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        let path = normalize_config_path(path.as_ref());
        self.normalize(&path);
        self.validate()?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(ConfigError::Mkdir)?;
        }

        let out = toml::to_string_pretty(&self).map_err(ConfigError::Serialize)?;
        fs::write(&path, out).map_err(ConfigError::Write)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.project.name.trim().is_empty() {
            return Err(ConfigError::Validation(
                "project.name must not be empty".to_string(),
            ));
        }
        if self.project.scan_root.trim().is_empty() {
            return Err(ConfigError::Validation(
                "project.scan_root must not be empty".to_string(),
            ));
        }
        if self.generation.toml_name.trim().is_empty() {
            return Err(ConfigError::Validation(
                "generation.toml_name must not be empty".to_string(),
            ));
        }
        if self.generation.header_name.trim().is_empty() {
            return Err(ConfigError::Validation(
                "generation.header_name must not be empty".to_string(),
            ));
        }
        if self.rttd.text_id > 0xFF {
            return Err(ConfigError::Validation(format!(
                "rttd.text_id out of range: 0x{:X}",
                self.rttd.text_id
            )));
        }

        if self.rttd.source.scan_timeout_ms == 0 {
            return Err(ConfigError::Validation(
                "rttd.source.scan_timeout_ms must be > 0".to_string(),
            ));
        }
        if self.rttd.source.last_selected_addr.trim().is_empty() {
            return Err(ConfigError::Validation(
                "rttd.source.last_selected_addr must not be empty".to_string(),
            ));
        }

        if self.rttd.behavior.sync_debounce_ms == 0 {
            return Err(ConfigError::Validation(
                "rttd.behavior.sync_debounce_ms must be > 0".to_string(),
            ));
        }
        if self.rttd.behavior.buf == 0 {
            return Err(ConfigError::Validation(
                "rttd.behavior.buf must be > 0".to_string(),
            ));
        }
        if self.rttd.behavior.reader_buf == 0 {
            return Err(ConfigError::Validation(
                "rttd.behavior.reader_buf must be > 0".to_string(),
            ));
        }

        if self.rttd.source.backend.startup_timeout_ms == 0 {
            return Err(ConfigError::Validation(
                "rttd.source.backend.startup_timeout_ms must be > 0".to_string(),
            ));
        }
        if self.rttd.source.backend.openocd.speed == 0 {
            return Err(ConfigError::Validation(
                "rttd.source.backend.openocd.speed must be > 0".to_string(),
            ));
        }
        if self.rttd.source.backend.openocd.polling == 0 {
            return Err(ConfigError::Validation(
                "rttd.source.backend.openocd.polling must be > 0".to_string(),
            ));
        }
        if self.rttd.source.backend.jlink.device.trim().is_empty() {
            return Err(ConfigError::Validation(
                "rttd.source.backend.jlink.device must not be empty".to_string(),
            ));
        }
        if self.rttd.source.backend.jlink.interface.trim().is_empty() {
            return Err(ConfigError::Validation(
                "rttd.source.backend.jlink.interface must not be empty".to_string(),
            ));
        }
        if self.rttd.source.backend.jlink.speed == 0 {
            return Err(ConfigError::Validation(
                "rttd.source.backend.jlink.speed must be > 0".to_string(),
            ));
        }
        if self.rttd.outputs.foxglove.ws_addr.trim().is_empty() {
            return Err(ConfigError::Validation(
                "rttd.outputs.foxglove.ws_addr must not be empty".to_string(),
            ));
        }

        let mut seen = BTreeSet::new();
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

    pub fn generated_toml_path(&self) -> &Path {
        &self.generated_toml_path
    }

    pub fn generated_header_path(&self) -> &Path {
        &self.generated_header_path
    }

    pub fn resolve_relative_path(&self, raw: impl AsRef<Path>) -> PathBuf {
        let path = raw.as_ref();
        if path.as_os_str().is_empty() {
            return PathBuf::new();
        }
        if path.is_absolute() {
            return path.to_path_buf();
        }
        self.config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(path)
    }

    pub fn normalize(&mut self, path: &Path) {
        if self.project.name.trim().is_empty() {
            self.project.name = ProjectConfig::default().name;
        }
        if self.project.scan_root.trim().is_empty() {
            self.project.scan_root = ProjectConfig::default().scan_root;
        }
        if self.project.extensions.is_empty() {
            self.project.extensions = ProjectConfig::default().extensions;
        }
        if self.generation.out_dir.trim().is_empty() {
            self.generation.out_dir = GenerationConfig::default().out_dir;
        }
        if self.generation.toml_name.trim().is_empty() {
            self.generation.toml_name = GenerationConfig::default().toml_name;
        }
        if self.generation.header_name.trim().is_empty() {
            self.generation.header_name = GenerationConfig::default().header_name;
        }

        if self.rttd.source.last_selected_addr.trim().is_empty() {
            self.rttd.source.last_selected_addr = RttdSourceConfig::default().last_selected_addr;
        }
        if self.rttd.behavior.reconnect.trim().is_empty() {
            self.rttd.behavior.reconnect = RttdBehaviorConfig::default().reconnect;
        }
        if self.rttd.outputs.foxglove.ws_addr.trim().is_empty() {
            self.rttd.outputs.foxglove.ws_addr = FoxgloveOutputConfig::default().ws_addr;
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
        self.scan_root_path = normalize_path_fallback(scan_root);

        let mut out_dir = PathBuf::from(&self.generation.out_dir);
        if !out_dir.is_absolute() {
            out_dir = base_dir.join(out_dir);
        }
        let out_dir = normalize_path_fallback(out_dir);

        self.generated_toml_path = out_dir.join(&self.generation.toml_name);
        self.generated_header_path = out_dir.join(&self.generation.header_name);
    }
}

fn normalize_config_path(path: &Path) -> PathBuf {
    if path.as_os_str().is_empty() {
        PathBuf::from(DEFAULT_CONFIG_PATH)
    } else {
        path.to_path_buf()
    }
}

fn normalize_path_fallback(path: PathBuf) -> PathBuf {
    if let Ok(abs) = path.canonicalize() {
        abs
    } else {
        path.components().collect()
    }
}

fn reject_deprecated_config_keys(raw: &str) -> Result<(), ConfigError> {
    let value: toml::Value = toml::from_str(raw).map_err(ConfigError::Parse)?;

    let mut deprecated_keys = Vec::new();

    if value
        .get("project")
        .and_then(toml::Value::as_table)
        .map(|project| project.contains_key("ignore_dirs"))
        .unwrap_or(false)
    {
        deprecated_keys.push("project.ignore_dirs");
    }

    if let Some(rttd) = value.get("rttd").and_then(toml::Value::as_table) {
        if rttd.contains_key("server") {
            deprecated_keys.push("[rttd.server]");
        }
        if rttd.contains_key("foxglove") {
            deprecated_keys.push("[rttd.foxglove]");
        }
        if rttd
            .get("source")
            .and_then(toml::Value::as_table)
            .map(|source| source.contains_key("preferred_backend"))
            .unwrap_or(false)
        {
            deprecated_keys.push("rttd.source.preferred_backend");
        }
        if rttd
            .get("source")
            .and_then(toml::Value::as_table)
            .and_then(|source| source.get("backend"))
            .and_then(toml::Value::as_table)
            .and_then(|backend| backend.get("jlink"))
            .and_then(toml::Value::as_table)
            .map(|jlink| jlink.contains_key("rtt_telnet_port"))
            .unwrap_or(false)
        {
            deprecated_keys.push("rttd.source.backend.jlink.rtt_telnet_port");
        }
    }

    if deprecated_keys.is_empty() {
        return Ok(());
    }

    Err(ConfigError::Validation(format!(
        "deprecated config keys removed in v0.2.0: {}. Migrate rttd keys via docs/migrations/0.2.0-breaking.md and move path filters into .rttdignore",
        deprecated_keys.join(", ")
    )))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}_{unique}"));
        fs::create_dir_all(&dir).expect("mkdir temp dir");
        dir
    }

    #[test]
    fn default_path_uses_rat_toml() {
        assert_eq!(DEFAULT_CONFIG_PATH, "rat.toml");
    }

    #[test]
    fn resolve_relative_path_uses_config_dir() {
        let mut cfg = RatitudeConfig::default();
        let path = PathBuf::from("tmp/rat.toml");
        cfg.normalize(&path);

        let resolved = cfg.resolve_relative_path("demo.jpg");
        assert!(resolved.ends_with(Path::new("tmp").join("demo.jpg")));

        let absolute = std::env::temp_dir().join("demo.jpg");
        assert_eq!(cfg.resolve_relative_path(&absolute), absolute);
    }

    #[test]
    fn normalize_sets_generated_paths() {
        let mut cfg = RatitudeConfig::default();
        cfg.generation.out_dir = "generated".to_string();
        cfg.generation.toml_name = "rat_gen.toml".to_string();
        cfg.generation.header_name = "rat_gen.h".to_string();
        cfg.normalize(Path::new("firmware/example/stm32f4_rtt/rat.toml"));

        assert!(cfg
            .generated_toml_path()
            .ends_with("generated/rat_gen.toml"));
        assert!(cfg.generated_header_path().ends_with("generated/rat_gen.h"));
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = unique_temp_dir("ratitude_cfg_roundtrip");
        let path = dir.join("rat.toml");

        let mut cfg = RatitudeConfig::default();
        cfg.project.name = "demo".to_string();
        cfg.project.scan_root = "Core".to_string();
        cfg.artifacts.elf = "build/app.elf".to_string();
        cfg.generation.out_dir = "generated".to_string();
        cfg.rttd.outputs.jsonl.enabled = false;
        cfg.rttd.outputs.foxglove.enabled = true;
        cfg.save(&path).expect("save config");

        let (loaded, exists) = load_or_default(&path).expect("load config");
        assert!(exists);
        assert_eq!(loaded.project.name, "demo");
        assert_eq!(loaded.artifacts.elf, "build/app.elf");
        assert!(loaded
            .generated_toml_path()
            .ends_with("generated/rat_gen.toml"));
        assert!(!loaded.rttd.outputs.jsonl.enabled);
        assert!(loaded.rttd.outputs.foxglove.enabled);

        let _ = fs::remove_file(path);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn generated_config_round_trip() {
        let dir = unique_temp_dir("ratitude_gen_roundtrip");
        let path = dir.join("rat_gen.toml");
        let mut cfg = GeneratedConfig::default();
        cfg.meta.project = "demo".to_string();
        cfg.meta.fingerprint = "0x00000000AABBCCDD".to_string();
        cfg.packets.push(GeneratedPacketDef {
            id: 1,
            signature_hash: "0x1122".to_string(),
            struct_name: "AttitudePacket".to_string(),
            packet_type: "quat".to_string(),
            packed: true,
            byte_size: 16,
            source: "Core/Src/main.c".to_string(),
            fields: vec![FieldDef {
                name: "w".to_string(),
                c_type: "float".to_string(),
                offset: 0,
                size: 4,
            }],
        });

        save_generated(&path, &cfg).expect("save generated config");
        let (loaded, exists) = load_generated_or_default(&path).expect("load generated config");
        assert!(exists);
        assert_eq!(loaded, cfg);

        let _ = fs::remove_file(path);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn validate_rejects_zero_reader_buffer() {
        let mut cfg = RatitudeConfig::default();
        cfg.rttd.behavior.reader_buf = 0;
        let err = cfg.validate().expect_err("validation should fail");
        assert!(err
            .to_string()
            .contains("rttd.behavior.reader_buf must be > 0"));
    }

    #[test]
    fn validate_rejects_zero_scan_timeout() {
        let mut cfg = RatitudeConfig::default();
        cfg.rttd.source.scan_timeout_ms = 0;
        let err = cfg.validate().expect_err("validation should fail");
        assert!(err
            .to_string()
            .contains("rttd.source.scan_timeout_ms must be > 0"));
    }

    #[test]
    fn legacy_rttd_sections_are_rejected() {
        let dir = unique_temp_dir("ratitude_cfg_legacy_sections");
        let path = dir.join("rat.toml");
        let raw = r#"
[project]
name = "demo"
scan_root = "Core"

[generation]
out_dir = "."
toml_name = "rat_gen.toml"
header_name = "rat_gen.h"

[rttd]
text_id = 255

[rttd.server]
addr = "127.0.0.1:19021"

[rttd.foxglove]
ws_addr = "127.0.0.1:8765"
"#;
        fs::write(&path, raw).expect("write config");

        let err = load_or_default(&path).expect_err("legacy sections should fail");
        let msg = err.to_string();
        assert!(msg.contains("deprecated config keys removed in v0.2.0"));
        assert!(msg.contains("[rttd.server]"));
        assert!(msg.contains("[rttd.foxglove]"));
        assert!(msg.contains("docs/migrations/0.2.0-breaking.md"));

        let _ = fs::remove_file(path);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn preferred_backend_is_rejected_with_migration_hint() {
        let dir = unique_temp_dir("ratitude_cfg_preferred_backend");
        let path = dir.join("rat.toml");
        let raw = r#"
[project]
name = "demo"
scan_root = "Core"

[generation]
out_dir = "."
toml_name = "rat_gen.toml"
header_name = "rat_gen.h"

[rttd]
text_id = 255

[rttd.source]
auto_scan = true
scan_timeout_ms = 300
last_selected_addr = "127.0.0.1:19021"
preferred_backend = "openocd"
"#;
        fs::write(&path, raw).expect("write config");

        let err = load_or_default(&path).expect_err("preferred_backend should fail");
        let msg = err.to_string();
        assert!(msg.contains("deprecated config keys removed in v0.2.0"));
        assert!(msg.contains("rttd.source.preferred_backend"));
        assert!(msg.contains("docs/migrations/0.2.0-breaking.md"));

        let _ = fs::remove_file(path);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn ignore_dirs_is_rejected_with_rttdignore_migration_hint() {
        let dir = unique_temp_dir("ratitude_cfg_ignore_dirs");
        let path = dir.join("rat.toml");
        let raw = r#"
[project]
name = "demo"
scan_root = "Core"
ignore_dirs = ["build", ".git"]

[generation]
out_dir = "."
toml_name = "rat_gen.toml"
header_name = "rat_gen.h"
"#;
        fs::write(&path, raw).expect("write config");

        let err = load_or_default(&path).expect_err("ignore_dirs should fail");
        let msg = err.to_string();
        assert!(msg.contains("deprecated config keys removed in v0.2.0"));
        assert!(msg.contains("project.ignore_dirs"));
        assert!(msg.contains(".rttdignore"));

        let _ = fs::remove_file(path);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn jlink_rtt_telnet_port_is_rejected_with_source_addr_hint() {
        let dir = unique_temp_dir("ratitude_cfg_jlink_rtt_port");
        let path = dir.join("rat.toml");
        let raw = r#"
[project]
name = "demo"
scan_root = "Core"

[generation]
out_dir = "."
toml_name = "rat_gen.toml"
header_name = "rat_gen.h"

[rttd]
text_id = 255

[rttd.source]
auto_scan = true
scan_timeout_ms = 300
last_selected_addr = "127.0.0.1:19021"

[rttd.source.backend]
type = "jlink"
auto_start = false
startup_timeout_ms = 5000

[rttd.source.backend.jlink]
device = "STM32F407ZG"
interface = "SWD"
speed = 4000
rtt_telnet_port = 19021
"#;
        fs::write(&path, raw).expect("write config");

        let err = load_or_default(&path).expect_err("rtt_telnet_port should fail");
        let msg = err.to_string();
        assert!(msg.contains("deprecated config keys removed in v0.2.0"));
        assert!(msg.contains("rttd.source.backend.jlink.rtt_telnet_port"));

        let _ = fs::remove_file(path);
        let _ = fs::remove_dir_all(dir);
    }
}
