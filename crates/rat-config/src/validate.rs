use std::time::Duration;

use crate::{
    ConfigError, FoxgloveOutputConfig, GenerationConfig, ProjectConfig, RatdBehaviorConfig,
    RatdSourceConfig, RatitudeConfig,
};

impl RatitudeConfig {
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
        if self.generation.header_name.trim().is_empty() {
            return Err(ConfigError::Validation(
                "generation.header_name must not be empty".to_string(),
            ));
        }
        if self.ratd.text_id == 0 {
            return Err(ConfigError::Validation(
                "ratd.text_id 0x0 is reserved for runtime control packet".to_string(),
            ));
        }
        if self.ratd.text_id > 0xFF {
            return Err(ConfigError::Validation(format!(
                "ratd.text_id out of range: 0x{:X}",
                self.ratd.text_id
            )));
        }

        if self.ratd.source.scan_timeout_ms == 0 {
            return Err(ConfigError::Validation(
                "ratd.source.scan_timeout_ms must be > 0".to_string(),
            ));
        }
        if self.ratd.source.last_selected_addr.trim().is_empty() {
            return Err(ConfigError::Validation(
                "ratd.source.last_selected_addr must not be empty".to_string(),
            ));
        }
        if self.ratd.source.auto_scan && self.ratd.source.seed_addrs.is_empty() {
            return Err(ConfigError::Validation(
                "ratd.source.seed_addrs must not be empty when auto_scan=true".to_string(),
            ));
        }

        if self.ratd.behavior.buf == 0 {
            return Err(ConfigError::Validation(
                "ratd.behavior.buf must be > 0".to_string(),
            ));
        }
        if self.ratd.behavior.reader_buf == 0 {
            return Err(ConfigError::Validation(
                "ratd.behavior.reader_buf must be > 0".to_string(),
            ));
        }
        self.ratd.behavior.reconnect_duration()?;
        self.ratd.behavior.schema_timeout_duration()?;

        if self.ratd.outputs.foxglove.ws_addr.trim().is_empty() {
            return Err(ConfigError::Validation(
                "ratd.outputs.foxglove.ws_addr must not be empty".to_string(),
            ));
        }
        validate_foxglove_ws_addr(&self.ratd.outputs.foxglove.ws_addr)?;
        Ok(())
    }

    pub fn normalize(&mut self) {
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
        if self.generation.header_name.trim().is_empty() {
            self.generation.header_name = GenerationConfig::default().header_name;
        }

        if self.ratd.source.last_selected_addr.trim().is_empty() {
            self.ratd.source.last_selected_addr = RatdSourceConfig::default().last_selected_addr;
        }
        if self.ratd.source.seed_addrs.is_empty() {
            self.ratd.source.seed_addrs = RatdSourceConfig::default().seed_addrs;
        }
        if self.ratd.behavior.reconnect.trim().is_empty() {
            self.ratd.behavior.reconnect = RatdBehaviorConfig::default().reconnect;
        }
        if self.ratd.behavior.schema_timeout.trim().is_empty() {
            self.ratd.behavior.schema_timeout = RatdBehaviorConfig::default().schema_timeout;
        }
        if self.ratd.outputs.foxglove.ws_addr.trim().is_empty() {
            self.ratd.outputs.foxglove.ws_addr = FoxgloveOutputConfig::default().ws_addr;
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

        self.ratd.source.seed_addrs = self
            .ratd
            .source
            .seed_addrs
            .iter()
            .map(|addr| addr.trim())
            .filter(|addr| !addr.is_empty())
            .map(ToString::to_string)
            .collect();
    }
}

impl RatdBehaviorConfig {
    pub fn reconnect_duration(&self) -> Result<Duration, ConfigError> {
        let reconnect = parse_duration_value("ratd.behavior.reconnect", &self.reconnect)?;
        if reconnect.is_zero() {
            return Err(ConfigError::Validation(
                "ratd.behavior.reconnect must be > 0".to_string(),
            ));
        }
        Ok(reconnect)
    }

    pub fn schema_timeout_duration(&self) -> Result<Duration, ConfigError> {
        let schema_timeout =
            parse_duration_value("ratd.behavior.schema_timeout", &self.schema_timeout)?;
        if schema_timeout.is_zero() {
            return Err(ConfigError::Validation(
                "ratd.behavior.schema_timeout must be > 0".to_string(),
            ));
        }
        Ok(schema_timeout)
    }
}

fn parse_duration_value(field: &str, raw: &str) -> Result<Duration, ConfigError> {
    humantime::parse_duration(raw).map_err(|err| {
        ConfigError::Validation(format!("{field} must be a valid duration string ({err})"))
    })
}

fn validate_foxglove_ws_addr(raw_addr: &str) -> Result<(), ConfigError> {
    let normalized = raw_addr.trim();
    if let Some(rest) = normalized.strip_prefix('[') {
        let (host, suffix) = rest.split_once(']').ok_or_else(|| {
            ConfigError::Validation(format!(
                "ratd.outputs.foxglove.ws_addr must be host:port or [ipv6]:port: {raw_addr}"
            ))
        })?;
        if host.is_empty() {
            return Err(ConfigError::Validation(format!(
                "ratd.outputs.foxglove.ws_addr must be host:port or [ipv6]:port: {raw_addr}"
            )));
        }
        let port_raw = suffix.strip_prefix(':').ok_or_else(|| {
            ConfigError::Validation(format!(
                "ratd.outputs.foxglove.ws_addr must be host:port or [ipv6]:port: {raw_addr}"
            ))
        })?;
        parse_ws_port(port_raw, raw_addr)?;
        return Ok(());
    }

    let (host, port_raw) = normalized.rsplit_once(':').ok_or_else(|| {
        ConfigError::Validation(format!(
            "ratd.outputs.foxglove.ws_addr must be host:port or [ipv6]:port: {raw_addr}"
        ))
    })?;
    if host.is_empty() || host.contains(':') {
        return Err(ConfigError::Validation(format!(
            "ratd.outputs.foxglove.ws_addr must be host:port or [ipv6]:port: {raw_addr}"
        )));
    }
    parse_ws_port(port_raw, raw_addr)
}

fn parse_ws_port(raw_port: &str, raw_addr: &str) -> Result<(), ConfigError> {
    raw_port.parse::<u16>().map(|_| ()).map_err(|err| {
        ConfigError::Validation(format!(
            "ratd.outputs.foxglove.ws_addr has invalid port in {raw_addr}: {err}"
        ))
    })
}
