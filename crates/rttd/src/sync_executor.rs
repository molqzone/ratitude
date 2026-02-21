use anyhow::Result;
use rat_sync::sync_packets_fs;

#[derive(Debug, Clone)]
pub struct SyncExecutionResult {
    pub changed: bool,
    pub packets: usize,
    pub warnings: Vec<String>,
}

pub trait SyncExecutor: Send + Sync + 'static {
    fn execute(&self) -> Result<SyncExecutionResult>;
}

#[derive(Debug, Clone)]
pub struct FsSyncExecutor {
    config_path: String,
}

impl FsSyncExecutor {
    pub fn new(config_path: String) -> Self {
        Self { config_path }
    }
}

impl SyncExecutor for FsSyncExecutor {
    fn execute(&self) -> Result<SyncExecutionResult> {
        let result = sync_packets_fs(&self.config_path, None)?;
        Ok(SyncExecutionResult {
            changed: result.changed,
            packets: result.config.packets.len(),
            warnings: result.layout_warnings,
        })
    }
}
