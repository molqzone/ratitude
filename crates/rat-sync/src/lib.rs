use std::path::PathBuf;

use rat_config::ConfigError;
use thiserror::Error;

mod assembler;
mod ast;
mod discover;
mod fs_adapter;
mod generated;
mod header;
mod ids;
mod layout;
mod model;
mod parser;
mod parser_runner;
mod pipeline;
mod scanner;
mod schema;

pub use fs_adapter::sync_packets_fs;
pub use generated::{GeneratedConfig, GeneratedMeta, GeneratedPacketDef};
pub use model::{DiscoveredPacket, SyncFsResult, SyncPipelineInput, SyncPipelineOutput};
pub use pipeline::run_sync_pipeline;

pub(crate) const RAT_ID_MIN: u16 = 1;
pub(crate) const RAT_ID_MAX: u16 = 0xFE;

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
    #[error("failed to read generated header {path}: {source}")]
    ReadGeneratedHeader {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("sync validation failed: {0}")]
    Validation(String),
    #[error("failed to write generated header {path}: {source}")]
    WriteHeader {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests;
