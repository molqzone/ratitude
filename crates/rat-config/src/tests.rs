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
    cfg.generation.toml_name = "rat_gen.toml".to_string();
    cfg.generation.header_name = "rat_gen.h".to_string();
    cfg.normalize();
    let paths = resolve_config_paths(&cfg, Path::new("firmware/example/stm32f4_rtt/rat.toml"));

    assert!(paths
        .generated_toml_path()
        .ends_with("generated/rat_gen.toml"));
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
    cfg.rttd.outputs.jsonl.enabled = false;
    cfg.rttd.outputs.foxglove.enabled = true;
    ConfigStore::new(&path).save(&cfg).expect("save config");

    let (loaded, exists) = load_or_default(&path).expect("load config");
    assert!(exists);
    assert_eq!(loaded.project.name, "demo");
    assert_eq!(loaded.artifacts.elf, "build/app.elf");
    let paths = resolve_config_paths(&loaded, &path);
    assert!(paths
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
fn source_backend_section_is_rejected_with_migration_hint() {
    let dir = unique_temp_dir("ratitude_cfg_source_backend");
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
type = "none"
auto_start = false
startup_timeout_ms = 5000
"#;
    fs::write(&path, raw).expect("write config");

    let err = load_or_default(&path).expect_err("source backend section should fail");
    let msg = err.to_string();
    assert!(msg.contains("deprecated config keys removed in v0.2.0"));
    assert!(msg.contains("[rttd.source.backend]"));

    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn removed_sync_behavior_keys_are_rejected_with_migration_hint() {
    let dir = unique_temp_dir("ratitude_cfg_removed_sync_behavior");
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

[rttd.behavior]
auto_sync_on_start = true
auto_sync_on_reset = true
sync_debounce_ms = 500
reconnect = "1s"
schema_timeout = "3s"
buf = 256
reader_buf = 65536
"#;
    fs::write(&path, raw).expect("write config");

    let err = load_or_default(&path).expect_err("removed sync behavior keys should fail");
    let msg = err.to_string();
    assert!(msg.contains("deprecated config keys removed in v0.2.0"));
    assert!(msg.contains("rttd.behavior.auto_sync_on_start"));
    assert!(msg.contains("rttd.behavior.auto_sync_on_reset"));
    assert!(msg.contains("rttd.behavior.sync_debounce_ms"));

    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(dir);
}
