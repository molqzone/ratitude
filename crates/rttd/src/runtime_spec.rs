use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use rat_config::{load_generated_or_default, resolve_config_paths, PacketDef, RatitudeConfig};
use rat_core::{IngestRuntimeConfig, ListenerOptions, RuntimeFieldDef, RuntimePacketDef};

pub(crate) struct RuntimeSpec {
    pub(crate) ingest_config: IngestRuntimeConfig,
    pub(crate) packets: Vec<PacketDef>,
}

pub(crate) fn build_runtime_spec(
    cfg: &mut RatitudeConfig,
    config_path: &str,
    addr: &str,
    unknown_window: Duration,
    unknown_threshold: u32,
) -> Result<RuntimeSpec> {
    let expected_fingerprint = load_generated_packets(cfg, config_path)?;
    let text_id = parse_text_id(cfg.rttd.text_id)?;
    let reconnect = parse_duration(&cfg.rttd.behavior.reconnect)?;
    let buf = cfg.rttd.behavior.buf;
    let reader_buf = cfg.rttd.behavior.reader_buf;
    let packets = cfg.packets.clone();

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
            expected_fingerprint,
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

fn load_generated_packets(cfg: &mut RatitudeConfig, config_path: &str) -> Result<u64> {
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

    cfg.packets = generated.to_packet_defs();
    cfg.validate()?;
    Ok(expected_fingerprint)
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
    use super::*;

    #[test]
    fn parse_generated_fingerprint_rejects_empty() {
        assert!(parse_generated_fingerprint(" ").is_err());
    }

    #[test]
    fn parse_generated_fingerprint_supports_prefixed_hex() {
        let parsed = parse_generated_fingerprint("0xAA").expect("parse");
        assert_eq!(parsed, 0xAA);
    }
}
