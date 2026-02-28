use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::*;

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
    cfg.normalize();
    let store = ConfigStore::new(&path);
    let paths = store.paths_for(&cfg);

    let resolved = paths.resolve_relative_path("demo.jpg");
    assert!(resolved.ends_with(Path::new("tmp").join("demo.jpg")));

    let absolute = std::env::temp_dir().join("demo.jpg");
    assert_eq!(paths.resolve_relative_path(&absolute), absolute);
}

#[test]
fn normalize_sets_generated_paths() {
    let mut cfg = RatitudeConfig::default();
    cfg.generation.out_dir = "generated".to_string();
    cfg.generation.header_name = "rat_gen.h".to_string();
    cfg.normalize();
    let paths = resolve_config_paths(&cfg, Path::new("firmware/example/stm32f4_rtt/rat.toml"));

    assert!(paths
        .generated_header_path()
        .ends_with("generated/rat_gen.h"));
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
    cfg.ratd.outputs.jsonl.enabled = false;
    cfg.ratd.outputs.foxglove.enabled = true;
    ConfigStore::new(&path).save(&cfg).expect("save config");

    let loaded = load(&path).expect("load config");
    assert_eq!(loaded.project.name, "demo");
    assert_eq!(loaded.artifacts.elf, "build/app.elf");
    let paths = resolve_config_paths(&loaded, &path);
    assert!(paths
        .generated_header_path()
        .ends_with("generated/rat_gen.h"));
    assert!(!loaded.ratd.outputs.jsonl.enabled);
    assert!(loaded.ratd.outputs.foxglove.enabled);

    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn validate_rejects_zero_reader_buffer() {
    let mut cfg = RatitudeConfig::default();
    cfg.ratd.behavior.reader_buf = 0;
    let err = cfg.validate().expect_err("validation should fail");
    assert!(err
        .to_string()
        .contains("ratd.behavior.reader_buf must be > 0"));
}

#[test]
fn validate_rejects_zero_scan_timeout() {
    let mut cfg = RatitudeConfig::default();
    cfg.ratd.source.scan_timeout_ms = 0;
    let err = cfg.validate().expect_err("validation should fail");
    assert!(err
        .to_string()
        .contains("ratd.source.scan_timeout_ms must be > 0"));
}

#[test]
fn validate_rejects_zero_reconnect_duration() {
    let mut cfg = RatitudeConfig::default();
    cfg.ratd.behavior.reconnect = "0s".to_string();
    let err = cfg.validate().expect_err("validation should fail");
    assert!(err
        .to_string()
        .contains("ratd.behavior.reconnect must be > 0"));
}

#[test]
fn validate_rejects_zero_schema_timeout_duration() {
    let mut cfg = RatitudeConfig::default();
    cfg.ratd.behavior.schema_timeout = "0s".to_string();
    let err = cfg.validate().expect_err("validation should fail");
    assert!(err
        .to_string()
        .contains("ratd.behavior.schema_timeout must be > 0"));
}

#[test]
fn validate_rejects_empty_seed_addrs_when_auto_scan_enabled() {
    let mut cfg = RatitudeConfig::default();
    cfg.ratd.source.auto_scan = true;
    cfg.ratd.source.seed_addrs.clear();
    let err = cfg.validate().expect_err("validation should fail");
    assert!(err
        .to_string()
        .contains("ratd.source.seed_addrs must not be empty"));
}

#[test]
fn validate_rejects_reserved_text_id() {
    let mut cfg = RatitudeConfig::default();
    cfg.ratd.text_id = 0;
    let err = cfg.validate().expect_err("validation should fail");
    assert!(err
        .to_string()
        .contains("ratd.text_id 0x0 is reserved for runtime control packet"));
}

#[test]
fn validate_rejects_invalid_foxglove_ws_addr() {
    let mut cfg = RatitudeConfig::default();
    cfg.ratd.outputs.foxglove.ws_addr = "::1:8765".to_string();
    let err = cfg.validate().expect_err("validation should fail");
    assert!(err.to_string().contains("host:port or [ipv6]:port"));
}

#[test]
fn parse_foxglove_ws_addr_supports_hostname_and_ipv4() {
    let (host, port) = parse_foxglove_ws_addr("localhost:8765").expect("parse host:port");
    assert_eq!(host, "localhost");
    assert_eq!(port, 8765);
}

#[test]
fn parse_foxglove_ws_addr_supports_bracketed_ipv6() {
    let (host, port) = parse_foxglove_ws_addr("[::1]:8765").expect("parse ipv6");
    assert_eq!(host, "::1");
    assert_eq!(port, 8765);
}

#[test]
fn parse_foxglove_ws_addr_rejects_port_zero() {
    let err = parse_foxglove_ws_addr("127.0.0.1:0").expect_err("port 0 should be invalid");
    assert!(err.to_string().contains("port must be > 0"));
}

#[test]
fn parse_foxglove_ws_addr_rejects_whitespace_in_host() {
    let err = parse_foxglove_ws_addr("localhost :8765")
        .expect_err("whitespace around host should be invalid");
    assert!(err.to_string().contains("host:port or [ipv6]:port"));

    let err = parse_foxglove_ws_addr("[:: 1]:8765")
        .expect_err("whitespace inside ipv6 host should be invalid");
    assert!(err.to_string().contains("host:port or [ipv6]:port"));
}

#[test]
fn normalize_restores_seed_addrs_when_empty() {
    let mut cfg = RatitudeConfig::default();
    cfg.ratd.source.seed_addrs.clear();
    cfg.normalize();
    assert!(!cfg.ratd.source.seed_addrs.is_empty());
}

#[test]
fn removed_ratd_sections_are_rejected() {
    let dir = unique_temp_dir("ratitude_cfg_removed_sections");
    let path = dir.join("rat.toml");
    let raw = r#"
[project]
name = "demo"
scan_root = "Core"

[generation]
out_dir = "."
header_name = "rat_gen.h"

[ratd]
text_id = 255

[ratd.server]
addr = "127.0.0.1:19021"

[ratd.foxglove]
ws_addr = "127.0.0.1:8765"
"#;
    fs::write(&path, raw).expect("write config");

    let err = load(&path).expect_err("removed sections should fail");
    let msg = err.to_string();
    assert!(msg.contains("unknown field"));
    assert!(msg.contains("server"));

    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn preferred_backend_is_rejected() {
    let dir = unique_temp_dir("ratitude_cfg_preferred_backend");
    let path = dir.join("rat.toml");
    let raw = r#"
[project]
name = "demo"
scan_root = "Core"

[generation]
out_dir = "."
header_name = "rat_gen.h"

[ratd]
text_id = 255

[ratd.source]
auto_scan = true
scan_timeout_ms = 300
last_selected_addr = "127.0.0.1:19021"
preferred_backend = "openocd"
"#;
    fs::write(&path, raw).expect("write config");

    let err = load(&path).expect_err("preferred_backend should fail");
    let msg = err.to_string();
    assert!(msg.contains("unknown field"));
    assert!(msg.contains("preferred_backend"));

    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn ignore_dirs_is_rejected() {
    let dir = unique_temp_dir("ratitude_cfg_ignore_dirs");
    let path = dir.join("rat.toml");
    let raw = r#"
[project]
name = "demo"
scan_root = "Core"
ignore_dirs = ["build", ".git"]

[generation]
out_dir = "."
header_name = "rat_gen.h"
"#;
    fs::write(&path, raw).expect("write config");

    let err = load(&path).expect_err("ignore_dirs should fail");
    let msg = err.to_string();
    assert!(msg.contains("unknown field"));
    assert!(msg.contains("ignore_dirs"));

    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn source_backend_section_is_rejected() {
    let dir = unique_temp_dir("ratitude_cfg_source_backend");
    let path = dir.join("rat.toml");
    let raw = r#"
[project]
name = "demo"
scan_root = "Core"

[generation]
out_dir = "."
header_name = "rat_gen.h"

[ratd]
text_id = 255

[ratd.source]
auto_scan = true
scan_timeout_ms = 300
last_selected_addr = "127.0.0.1:19021"

[ratd.source.backend]
type = "none"
auto_start = false
startup_timeout_ms = 5000
"#;
    fs::write(&path, raw).expect("write config");

    let err = load(&path).expect_err("source backend section should fail");
    let msg = err.to_string();
    assert!(msg.contains("unknown field"));
    assert!(msg.contains("backend"));

    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn removed_sync_behavior_keys_are_rejected() {
    let dir = unique_temp_dir("ratitude_cfg_removed_sync_behavior");
    let path = dir.join("rat.toml");
    let raw = r#"
[project]
name = "demo"
scan_root = "Core"

[generation]
out_dir = "."
header_name = "rat_gen.h"

[ratd]
text_id = 255

[ratd.behavior]
auto_sync_on_start = true
auto_sync_on_reset = true
sync_debounce_ms = 500
reconnect = "1s"
schema_timeout = "3s"
buf = 256
reader_buf = 65536
"#;
    fs::write(&path, raw).expect("write config");

    let err = load(&path).expect_err("removed sync behavior keys should fail");
    let msg = err.to_string();
    assert!(msg.contains("unknown field"));
    assert!(msg.contains("auto_sync_on_start"));

    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn generation_toml_name_is_rejected() {
    let dir = unique_temp_dir("ratitude_cfg_generation_toml_name");
    let path = dir.join("rat.toml");
    let raw = r#"
[project]
name = "demo"
scan_root = "Core"

[generation]
out_dir = "."
toml_name = "rat_gen.toml"
header_name = "rat_gen.h"
"#;
    fs::write(&path, raw).expect("write config");

    let err = load(&path).expect_err("generation.toml_name should fail");
    let msg = err.to_string();
    assert!(msg.contains("unknown field"));
    assert!(msg.contains("toml_name"));

    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn load_missing_file_returns_not_found_error() {
    let dir = unique_temp_dir("ratitude_cfg_missing_not_found");
    let path = dir.join("rat.toml");
    assert!(!path.exists());

    let err = load(&path).expect_err("missing config should fail");
    assert!(matches!(err, ConfigError::NotFound(_)));
    assert!(err.to_string().contains("config file does not exist"));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn packet_def_rejects_unknown_fields() {
    let raw = r#"
id = 33
type = "plot"
struct_name = "DemoPacket"
packed = true
byte_size = 4
source = "src/main.c"
signature_hash = "0x1122334455667788"
"#;
    let err = toml::from_str::<PacketDef>(raw).expect_err("unknown field should fail");
    let msg = err.to_string();
    assert!(msg.contains("unknown field"));
    assert!(msg.contains("signature_hash"));
}
