use std::fs;
use std::path::{Path, PathBuf};

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

    fn read_and_validate(&self) -> Result<RatitudeConfig, ConfigError> {
        match fs::read_to_string(&self.config_path) {
            Ok(raw) => {
                let mut cfg: RatitudeConfig = toml::from_str(&raw).map_err(ConfigError::Parse)?;
                cfg.normalize();
                cfg.validate()?;
                Ok(cfg)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Err(ConfigError::NotFound(self.config_path.clone()))
            }
            Err(err) => Err(ConfigError::Read(err)),
        }
    }

    pub fn load(&self) -> Result<RatitudeConfig, ConfigError> {
        self.read_and_validate()
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

pub fn load(path: impl AsRef<Path>) -> Result<RatitudeConfig, ConfigError> {
    ConfigStore::new(path).load()
}
