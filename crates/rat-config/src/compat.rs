use crate::ConfigError;

pub(crate) fn reject_deprecated_config_keys(raw: &str) -> Result<(), ConfigError> {
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

    if value
        .get("generation")
        .and_then(toml::Value::as_table)
        .map(|generation| generation.contains_key("toml_name"))
        .unwrap_or(false)
    {
        deprecated_keys.push("generation.toml_name");
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
            .map(|source| source.contains_key("backend"))
            .unwrap_or(false)
        {
            deprecated_keys.push("[rttd.source.backend]");
        }
        if rttd
            .get("behavior")
            .and_then(toml::Value::as_table)
            .map(|behavior| behavior.contains_key("auto_sync_on_start"))
            .unwrap_or(false)
        {
            deprecated_keys.push("rttd.behavior.auto_sync_on_start");
        }
        if rttd
            .get("behavior")
            .and_then(toml::Value::as_table)
            .map(|behavior| behavior.contains_key("auto_sync_on_reset"))
            .unwrap_or(false)
        {
            deprecated_keys.push("rttd.behavior.auto_sync_on_reset");
        }
        if rttd
            .get("behavior")
            .and_then(toml::Value::as_table)
            .map(|behavior| behavior.contains_key("sync_debounce_ms"))
            .unwrap_or(false)
        {
            deprecated_keys.push("rttd.behavior.sync_debounce_ms");
        }
    }

    if deprecated_keys.is_empty() {
        return Ok(());
    }

    Err(ConfigError::Validation(format!(
        "deprecated config keys removed in v0.2.0: {}. Migrate rttd keys via docs/migrations/0.2.0-breaking.md, move path filters into .rttdignore, and remove generation.toml_name because rat_gen.toml is no longer generated",
        deprecated_keys.join(", ")
    )))
}
