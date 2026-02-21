use std::collections::BTreeSet;

use crate::{
    ConfigError, FoxgloveOutputConfig, GenerationConfig, ProjectConfig, RatitudeConfig,
    RttdBehaviorConfig, RttdSourceConfig,
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
    }
}
