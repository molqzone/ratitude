use std::path::PathBuf;

use rat_config::ConfigError;
use thiserror::Error;

#[cfg(test)]
use rat_config::GeneratedPacketDef;

mod assembler;
mod ast;
mod discover;
mod fs_adapter;
mod header;
mod ids;
mod layout;
mod model;
mod parser;
mod parser_runner;
mod pipeline;
mod scanner;

pub use fs_adapter::sync_packets_fs;
pub use model::{DiscoveredPacket, SyncFsResult, SyncPipelineInput, SyncPipelineOutput};
pub use pipeline::run_sync_pipeline;

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use assembler::*;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use ast::*;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use discover::*;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use fs_adapter::*;
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
pub(crate) use model::*;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use parser::*;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use parser_runner::*;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use pipeline::*;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use scanner::*;

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
