use std::path::{Path, PathBuf};

use glob::Pattern;
use rat_config::{
    self, load_generated_or_default, save_generated, ConfigError, FieldDef, GeneratedMeta,
    RatitudeConfig,
};
use thiserror::Error;

#[cfg(test)]
use rat_config::GeneratedPacketDef;

mod ast;
mod discover;
mod header;
mod ids;
mod layout;
mod parser;

use discover::discover_packets;
use header::write_generated_header;
use ids::{allocate_packet_ids, compute_fingerprint};
use layout::{collect_layout_blockers, collect_layout_warnings};

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use ast::*;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use discover::*;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use header::*;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use ids::*;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use layout::*;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use parser::*;

const RAT_ID_MIN: u16 = 1;
const RAT_ID_MAX: u16 = 0xFE;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("failed to read source file {path}: {source}")]
    ReadSource {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse C source {path}")]
    ParseSource { path: PathBuf },
    #[error("sync validation failed: {0}")]
    Validation(String),
    #[error("failed to write generated header {path}: {source}")]
    WriteHeader {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct SyncResult {
    pub config: RatitudeConfig,
    pub changed: bool,
    pub layout_warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct DiscoveredPacket {
    signature_hash: u64,
    struct_name: String,
    packet_type: String,
    packed: bool,
    byte_size: usize,
    source: String,
    fields: Vec<FieldDef>,
}

#[derive(Debug, Clone)]
struct TagMatch {
    end_byte: usize,
    packet_type: String,
    line: usize,
}

#[derive(Debug, Clone)]
struct StructDef {
    start_byte: usize,
    name: String,
    packed: bool,
    byte_size: usize,
    fields: Vec<FieldDef>,
}

#[derive(Debug, Clone)]
struct FieldSpec {
    name: String,
    c_type: String,
    size: usize,
}

#[derive(Debug, Clone)]
struct RttdIgnoreMatcher {
    root: PathBuf,
    patterns: Vec<Pattern>,
}

pub fn sync_packets(
    config_path: impl AsRef<Path>,
    scan_root_override: Option<&Path>,
) -> Result<SyncResult, SyncError> {
    let config_path = config_path.as_ref();
    let (mut cfg, _) = rat_config::load_or_default(config_path)?;

    let discovered = discover_packets(&cfg, scan_root_override)?;
    let layout_blockers = collect_layout_blockers(&discovered);
    if !layout_blockers.is_empty() {
        return Err(SyncError::Validation(format!(
            "layout validation failed:\n- {}",
            layout_blockers.join("\n- ")
        )));
    }
    let layout_warnings = collect_layout_warnings(&discovered);

    let generated_path = cfg.generated_toml_path().to_path_buf();
    let generated_header_path = cfg.generated_header_path().to_path_buf();

    let (old_generated, old_exists) = load_generated_or_default(&generated_path)?;
    let packets = allocate_packet_ids(&discovered, &old_generated.packets)?;
    let fingerprint = compute_fingerprint(&packets);

    let generated = rat_config::GeneratedConfig {
        meta: GeneratedMeta {
            project: cfg.project.name.clone(),
            fingerprint: format!("0x{:016X}", fingerprint),
        },
        packets,
    };

    let changed = !old_exists || old_generated != generated;
    if changed {
        save_generated(&generated_path, &generated)?;
    }
    if changed || !generated_header_path.exists() {
        write_generated_header(&generated_header_path, &generated)?;
    }

    cfg.packets = generated.to_packet_defs();
    cfg.validate()?;

    Ok(SyncResult {
        config: cfg,
        changed,
        layout_warnings,
    })
}

#[cfg(test)]
mod tests;
