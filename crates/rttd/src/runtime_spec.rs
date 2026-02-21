use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use rat_config::{load_generated_or_default, resolve_config_paths, PacketDef, RatitudeConfig};
use rat_core::{IngestRuntimeConfig, ListenerOptions, RuntimeFieldDef, RuntimePacketDef};

#[derive(Debug, Clone)]
pub(crate) struct RuntimeMaterial {
    pub(crate) packets: Vec<PacketDef>,
    pub(crate) expected_fingerprint: u64,
}

pub(crate) struct RuntimeSpec {
    pub(crate) ingest_config: IngestRuntimeConfig,
    pub(crate) packets: Vec<PacketDef>,
}

pub(crate) fn load_runtime_material(
    cfg: &RatitudeConfig,
    config_path: &str,
) -> Result<RuntimeMaterial> {
    let generated_path = resolve_config_paths(cfg, config_path)
        .generated_toml_path()
        .to_path_buf();
    let (generated, exists) = load_generated_or_default(&generated_path)?;
    if !exists {
        return Err(anyhow!(
            "rat_gen.toml not found at {}; run sync before starting daemon",
            generated_path.display()
        ));
    }
    if generated.packets.is_empty() {
        return Err(anyhow!("rat_gen.toml has no packets"));
    }
    let expected_fingerprint = parse_generated_fingerprint(&generated.meta.fingerprint)
        .with_context(|| format!("invalid fingerprint in {}", generated_path.display()))?;
    let packets = generated.to_packet_defs();

    let mut validated_cfg = cfg.clone();
    validated_cfg.packets = packets.clone();
    validated_cfg.validate()?;

    Ok(RuntimeMaterial {
        packets,
        expected_fingerprint,
    })
}

pub(crate) fn build_runtime_spec(
    cfg: &RatitudeConfig,
    material: &RuntimeMaterial,
    addr: &str,
    unknown_window: Duration,
    unknown_threshold: u32,
) -> Result<RuntimeSpec> {
    let text_id = parse_text_id(cfg.rttd.text_id)?;
    let reconnect = parse_duration(&cfg.rttd.behavior.reconnect)?;
    let buf = cfg.rttd.behavior.buf;
    let reader_buf = cfg.rttd.behavior.reader_buf;
    let packets = material.packets.clone();

    Ok(RuntimeSpec {
        ingest_config: IngestRuntimeConfig {
            addr: addr.to_string(),
            listener: ListenerOptions {
                reconnect,
                reconnect_max: Duration::from_secs(30),
                dial_timeout: Duration::from_secs(5),
                reader_buf_bytes: reader_buf,
            },
            hub_buffer: buf,
            text_packet_id: text_id,
            expected_fingerprint: material.expected_fingerprint,
            packets: map_runtime_packets(&packets),
            unknown_window,
            unknown_threshold,
        },
        packets,
    })
}

fn map_runtime_packets(packets: &[PacketDef]) -> Vec<RuntimePacketDef> {
    packets
        .iter()
        .map(|packet| RuntimePacketDef {
            id: packet.id,
            struct_name: packet.struct_name.clone(),
            packed: packet.packed,
            byte_size: packet.byte_size,
            fields: packet
                .fields
                .iter()
                .map(|field| RuntimeFieldDef {
                    name: field.name.clone(),
                    c_type: field.c_type.clone(),
                    offset: field.offset,
                    size: field.size,
                })
                .collect(),
        })
        .collect()
}

fn parse_text_id(value: u16) -> Result<u8> {
    if value > 0xFF {
        return Err(anyhow!("text id out of range: 0x{:X}", value));
    }
    Ok(value as u8)
}

fn parse_duration(raw: &str) -> Result<Duration> {
    humantime::parse_duration(raw).with_context(|| format!("invalid duration: {}", raw))
}

pub(crate) fn parse_generated_fingerprint(raw: &str) -> Result<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("generated fingerprint is empty"));
    }
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    u64::from_str_radix(hex, 16)
        .with_context(|| format!("invalid generated fingerprint value: {}", raw))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rat_config::{
        save_generated, ConfigStore, FieldDef, GeneratedConfig, GeneratedMeta, GeneratedPacketDef,
        PacketDef, RatitudeConfig,
    };

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

    fn save_default_config(path: &PathBuf) -> RatitudeConfig {
        let mut cfg = RatitudeConfig::default();
        cfg.project.name = "runtime_spec_test".to_string();
        ConfigStore::new(path).save(&cfg).expect("save config");
        cfg
    }

    fn sample_generated_packet() -> GeneratedPacketDef {
        GeneratedPacketDef {
            id: 0x10,
            signature_hash: "0x1234".to_string(),
            struct_name: "RatRuntime".to_string(),
            packet_type: "plot".to_string(),
            packed: true,
            byte_size: 4,
            source: "src/main.c".to_string(),
            fields: vec![FieldDef {
                name: "value".to_string(),
                c_type: "uint32_t".to_string(),
                offset: 0,
                size: 4,
            }],
        }
    }

    #[test]
    fn parse_generated_fingerprint_rejects_empty() {
        assert!(parse_generated_fingerprint(" ").is_err());
    }

    #[test]
    fn parse_generated_fingerprint_supports_prefixed_hex() {
        let parsed = parse_generated_fingerprint("0xAA").expect("parse");
        assert_eq!(parsed, 0xAA);
    }

    #[test]
    fn load_runtime_material_fails_when_generated_missing() {
        let dir = unique_temp_dir("rttd_runtime_material_missing");
        let config_path = dir.join("rat.toml");
        let cfg = save_default_config(&config_path);

        let err = load_runtime_material(&cfg, &config_path.to_string_lossy())
            .expect_err("missing generated should fail");
        assert!(err.to_string().contains("rat_gen.toml not found"));

        let _ = fs::remove_file(&config_path);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_runtime_material_fails_when_packets_empty() {
        let dir = unique_temp_dir("rttd_runtime_material_empty_packets");
        let config_path = dir.join("rat.toml");
        let cfg = save_default_config(&config_path);
        let generated_path = resolve_config_paths(&cfg, &config_path)
            .generated_toml_path()
            .to_path_buf();

        save_generated(
            &generated_path,
            &GeneratedConfig {
                meta: GeneratedMeta {
                    project: "runtime_spec_test".to_string(),
                    fingerprint: "0x1".to_string(),
                },
                packets: Vec::new(),
            },
        )
        .expect("save generated");

        let err = load_runtime_material(&cfg, &config_path.to_string_lossy())
            .expect_err("empty packets should fail");
        assert!(err.to_string().contains("rat_gen.toml has no packets"));

        let _ = fs::remove_file(&generated_path);
        let _ = fs::remove_file(&config_path);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_runtime_material_does_not_mutate_cfg_packets() {
        let dir = unique_temp_dir("rttd_runtime_material_no_mutation");
        let config_path = dir.join("rat.toml");
        let mut cfg = save_default_config(&config_path);
        cfg.packets = vec![PacketDef {
            id: 0x20,
            struct_name: "PreExisting".to_string(),
            packet_type: "plot".to_string(),
            packed: true,
            byte_size: 4,
            source: "src/old.c".to_string(),
            fields: vec![FieldDef {
                name: "old".to_string(),
                c_type: "uint32_t".to_string(),
                offset: 0,
                size: 4,
            }],
        }];

        let generated_path = resolve_config_paths(&cfg, &config_path)
            .generated_toml_path()
            .to_path_buf();
        save_generated(
            &generated_path,
            &GeneratedConfig {
                meta: GeneratedMeta {
                    project: "runtime_spec_test".to_string(),
                    fingerprint: "0x2".to_string(),
                },
                packets: vec![sample_generated_packet()],
            },
        )
        .expect("save generated");

        let material = load_runtime_material(&cfg, &config_path.to_string_lossy())
            .expect("load material");
        assert_eq!(cfg.packets.len(), 1);
        assert_eq!(cfg.packets[0].struct_name, "PreExisting");
        assert_eq!(material.packets.len(), 1);
        assert_eq!(material.packets[0].struct_name, "RatRuntime");

        let _ = fs::remove_file(&generated_path);
        let _ = fs::remove_file(&config_path);
        let _ = fs::remove_dir_all(dir);
    }
}
