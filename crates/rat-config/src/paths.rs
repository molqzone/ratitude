use std::path::{Path, PathBuf};

use crate::{RatitudeConfig, DEFAULT_CONFIG_PATH};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigPaths {
    config_path: PathBuf,
    scan_root_path: PathBuf,
    generated_header_path: PathBuf,
}

impl ConfigPaths {
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn scan_root_path(&self) -> &Path {
        &self.scan_root_path
    }

    pub fn generated_header_path(&self) -> &Path {
        &self.generated_header_path
    }

    pub fn resolve_relative_path(&self, raw: impl AsRef<Path>) -> PathBuf {
        let path = raw.as_ref();
        if path.as_os_str().is_empty() {
            return PathBuf::new();
        }
        if path.is_absolute() {
            return path.to_path_buf();
        }
        self.config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(path)
    }
}

pub fn resolve_config_paths(cfg: &RatitudeConfig, config_path: impl AsRef<Path>) -> ConfigPaths {
    let config_path = normalize_config_path(config_path.as_ref());
    let base_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let mut scan_root = PathBuf::from(&cfg.project.scan_root);
    if !scan_root.is_absolute() {
        scan_root = base_dir.join(scan_root);
    }
    let scan_root_path = normalize_path_fallback(scan_root);

    let mut out_dir = PathBuf::from(&cfg.generation.out_dir);
    if !out_dir.is_absolute() {
        out_dir = base_dir.join(out_dir);
    }
    let out_dir = normalize_path_fallback(out_dir);

    ConfigPaths {
        config_path,
        scan_root_path,
        generated_header_path: out_dir.join(&cfg.generation.header_name),
    }
}

pub(crate) fn normalize_config_path(path: &Path) -> PathBuf {
    if path.as_os_str().is_empty() {
        PathBuf::from(DEFAULT_CONFIG_PATH)
    } else {
        path.to_path_buf()
    }
}

fn normalize_path_fallback(path: PathBuf) -> PathBuf {
    if let Ok(abs) = path.canonicalize() {
        abs
    } else {
        path.components().collect()
    }
}
