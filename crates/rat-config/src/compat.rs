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

    if value.get("rttd").and_then(toml::Value::as_table).is_some() {
        deprecated_keys.push("[rttd] (renamed to [ratd])");
    }

    if let Some(ratd) = value.get("ratd").and_then(toml::Value::as_table) {
        if ratd.contains_key("server") {
            deprecated_keys.push("[ratd.server]");
        }
        if ratd.contains_key("foxglove") {
            deprecated_keys.push("[ratd.foxglove]");
        }
        if ratd
            .get("source")
            .and_then(toml::Value::as_table)
            .map(|source| source.contains_key("preferred_backend"))
            .unwrap_or(false)
        {
            deprecated_keys.push("ratd.source.preferred_backend");
        }
        if ratd
            .get("source")
            .and_then(toml::Value::as_table)
            .map(|source| source.contains_key("backend"))
            .unwrap_or(false)
        {
            deprecated_keys.push("[ratd.source.backend]");
        }
        if ratd
            .get("behavior")
            .and_then(toml::Value::as_table)
            .map(|behavior| behavior.contains_key("auto_sync_on_start"))
            .unwrap_or(false)
        {
            deprecated_keys.push("ratd.behavior.auto_sync_on_start");
        }
        if ratd
            .get("behavior")
            .and_then(toml::Value::as_table)
            .map(|behavior| behavior.contains_key("auto_sync_on_reset"))
            .unwrap_or(false)
        {
            deprecated_keys.push("ratd.behavior.auto_sync_on_reset");
        }
        if ratd
            .get("behavior")
            .and_then(toml::Value::as_table)
            .map(|behavior| behavior.contains_key("sync_debounce_ms"))
            .unwrap_or(false)
        {
            deprecated_keys.push("ratd.behavior.sync_debounce_ms");
        }
    }

    if deprecated_keys.is_empty() {
        return Ok(());
    }

    Err(ConfigError::Validation(format!(
        "deprecated config keys removed in v0.2.0: {}. Migrate ratd keys via docs/migrations/0.2.0-breaking.md, move path filters into .rttdignore, and remove generation.toml_name because rat_gen.toml is no longer generated",
        deprecated_keys.join(", ")
    )))
}
