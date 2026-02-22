use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use rat_config::RatitudeConfig;
use rat_core::{IngestRuntimeConfig, ListenerOptions};

#[derive(Debug)]
pub(crate) struct RuntimeSpec {
    pub(crate) ingest_config: IngestRuntimeConfig,
}

pub(crate) fn build_runtime_spec(
    cfg: &RatitudeConfig,
    addr: &str,
    unknown_window: Duration,
    unknown_threshold: u32,
) -> Result<RuntimeSpec> {
    let text_id = parse_text_id(cfg.rttd.text_id)?;
    let reconnect = parse_duration(&cfg.rttd.behavior.reconnect)?;
    let schema_timeout = parse_duration(&cfg.rttd.behavior.schema_timeout)?;

    Ok(RuntimeSpec {
        ingest_config: IngestRuntimeConfig {
            addr: addr.to_string(),
            listener: ListenerOptions {
                reconnect,
                reconnect_max: Duration::from_secs(30),
                dial_timeout: Duration::from_secs(5),
                reader_buf_bytes: cfg.rttd.behavior.reader_buf,
            },
            hub_buffer: cfg.rttd.behavior.buf,
            text_packet_id: text_id,
            schema_timeout,
            unknown_window,
            unknown_threshold,
        },
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_runtime_spec_reads_schema_timeout() {
        let mut cfg = RatitudeConfig::default();
        cfg.rttd.behavior.schema_timeout = "2s".to_string();

        let spec = build_runtime_spec(&cfg, "127.0.0.1:19021", Duration::from_secs(5), 20)
            .expect("build runtime spec");

        assert_eq!(spec.ingest_config.schema_timeout, Duration::from_secs(2));
    }

    #[test]
    fn build_runtime_spec_rejects_invalid_schema_timeout() {
        let mut cfg = RatitudeConfig::default();
        cfg.rttd.behavior.schema_timeout = "-".to_string();

        let err = build_runtime_spec(&cfg, "127.0.0.1:19021", Duration::from_secs(5), 20)
            .expect_err("invalid timeout should fail");
        assert!(err.to_string().contains("invalid duration"));
    }
}
