use std::fs;
use std::path::Path;

use crate::{ConfigError, GeneratedConfig};

pub fn load_generated_or_default(
    path: impl AsRef<Path>,
) -> Result<(GeneratedConfig, bool), ConfigError> {
    let path = path.as_ref();
    match fs::read_to_string(path) {
        Ok(raw) => {
            let cfg: GeneratedConfig = toml::from_str(&raw).map_err(ConfigError::Parse)?;
            Ok((cfg, true))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok((GeneratedConfig::default(), false))
        }
        Err(err) => Err(ConfigError::Read(err)),
    }
}

pub fn save_generated(path: impl AsRef<Path>, cfg: &GeneratedConfig) -> Result<(), ConfigError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(ConfigError::Mkdir)?;
    }
    let out = toml::to_string_pretty(cfg).map_err(ConfigError::Serialize)?;
    fs::write(path, out).map_err(ConfigError::Write)
}
