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
    }

    if deprecated_keys.is_empty() {
        return Ok(());
    }

    Err(ConfigError::Validation(format!(
        "deprecated config keys removed in v0.2.0: {}. Migrate rttd keys via docs/migrations/0.2.0-breaking.md and move path filters into .rttdignore",
        deprecated_keys.join(", ")
    )))
}
