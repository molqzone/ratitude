use std::fs;
use std::path::{Path, PathBuf};

use crate::compat::reject_removed_config_keys;
use crate::paths::normalize_config_path;
use crate::{resolve_config_paths, ConfigError, ConfigPaths, RatitudeConfig};

#[derive(Clone, Debug)]
pub struct ConfigStore {
    config_path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            config_path: normalize_config_path(path.as_ref()),
        }
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn load_or_default(&self) -> Result<(RatitudeConfig, bool), ConfigError> {
        match fs::read_to_string(&self.config_path) {
            Ok(raw) => {
                reject_removed_config_keys(&raw)?;
                let mut cfg: RatitudeConfig = toml::from_str(&raw).map_err(ConfigError::Parse)?;
                cfg.normalize();
                cfg.validate()?;
                Ok((cfg, true))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let mut cfg = RatitudeConfig::default();
                cfg.normalize();
                cfg.validate()?;
                Ok((cfg, false))
            }
            Err(err) => Err(ConfigError::Read(err)),
        }
    }

    pub fn load(&self) -> Result<RatitudeConfig, ConfigError> {
        let (cfg, exists) = self.load_or_default()?;
        if exists {
            Ok(cfg)
        } else {
            Err(ConfigError::Validation(
                "config file does not exist".to_string(),
            ))
        }
    }

    pub fn save(&self, cfg: &RatitudeConfig) -> Result<(), ConfigError> {
        let mut normalized = cfg.clone();
        normalized.normalize();
        normalized.validate()?;

        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent).map_err(ConfigError::Mkdir)?;
        }

        let out = toml::to_string_pretty(&normalized).map_err(ConfigError::Serialize)?;
        fs::write(&self.config_path, out).map_err(ConfigError::Write)
    }

    pub fn paths_for(&self, cfg: &RatitudeConfig) -> ConfigPaths {
        resolve_config_paths(cfg, &self.config_path)
    }
}

pub fn load_or_default(path: impl AsRef<Path>) -> Result<(RatitudeConfig, bool), ConfigError> {
    ConfigStore::new(path).load_or_default()
}

pub fn load(path: impl AsRef<Path>) -> Result<RatitudeConfig, ConfigError> {
    ConfigStore::new(path).load()
}
