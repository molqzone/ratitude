use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use tokio::sync::Mutex;

use crate::sync_executor::{FsSyncExecutor, SyncExecutionResult, SyncExecutor};

#[derive(Debug, Clone)]
pub struct SyncOutcome {
    pub changed: bool,
    pub packets: usize,
    pub warnings: Vec<String>,
    pub skipped: bool,
    pub reason: String,
}

#[derive(Debug)]
struct SyncState {
    last_success: Option<Instant>,
    in_flight: bool,
}

#[derive(Clone)]
pub struct SyncController {
    executor: Arc<dyn SyncExecutor>,
    debounce: Duration,
    state: Arc<Mutex<SyncState>>,
}

impl SyncController {
    pub fn new(config_path: String, debounce_ms: u64) -> Self {
        let executor: Arc<dyn SyncExecutor> = Arc::new(FsSyncExecutor::new(config_path));
        Self::new_with_executor(executor, debounce_ms)
    }

    pub fn new_with_executor(executor: Arc<dyn SyncExecutor>, debounce_ms: u64) -> Self {
        Self {
            executor,
            debounce: Duration::from_millis(debounce_ms.max(1)),
            state: Arc::new(Mutex::new(SyncState {
                last_success: None,
                in_flight: false,
            })),
        }
    }

    pub async fn trigger(&self, reason: &str) -> Result<SyncOutcome> {
        {
            let mut guard = self.state.lock().await;
            if guard.in_flight {
                return Ok(SyncOutcome {
                    changed: false,
                    packets: 0,
                    warnings: Vec::new(),
                    skipped: true,
                    reason: format!("{reason}: skipped (already running)"),
                });
            }
            if let Some(last) = guard.last_success {
                if last.elapsed() < self.debounce {
                    return Ok(SyncOutcome {
                        changed: false,
                        packets: 0,
                        warnings: Vec::new(),
                        skipped: true,
                        reason: format!("{reason}: skipped by debounce"),
                    });
                }
            }
            guard.in_flight = true;
        }

        let executor = Arc::clone(&self.executor);
        let join = tokio::task::spawn_blocking(move || executor.execute());
        let result = match join.await {
            Ok(Ok(result)) => result,
            Ok(Err(err)) => {
                let mut guard = self.state.lock().await;
                guard.in_flight = false;
                return Err(err.into());
            }
            Err(err) => {
                let mut guard = self.state.lock().await;
                guard.in_flight = false;
                return Err(anyhow!("sync executor join failed: {err}"));
            }
        };

        {
            let mut guard = self.state.lock().await;
            guard.in_flight = false;
            guard.last_success = Some(Instant::now());
        }

        Ok(SyncOutcome {
            changed: result.changed,
            packets: result.packets,
            warnings: result.warnings,
            skipped: false,
            reason: reason.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::sync_executor::SyncExecutor;

    struct StaticSuccessExecutor {
        result: SyncExecutionResult,
    }

    impl SyncExecutor for StaticSuccessExecutor {
        fn execute(&self) -> Result<SyncExecutionResult> {
            Ok(self.result.clone())
        }
    }

    struct StaticFailureExecutor;

    impl SyncExecutor for StaticFailureExecutor {
        fn execute(&self) -> Result<SyncExecutionResult> {
            Err(anyhow!("mock sync failure"))
        }
    }

    #[tokio::test]
    async fn debounce_prevents_back_to_back_trigger() {
        let controller = SyncController::new("/tmp/non-existent-rat.toml".to_string(), 5_000);
        {
            let mut guard = controller.state.lock().await;
            guard.last_success = Some(Instant::now());
        }

        let outcome = controller.trigger("test").await.expect("skip path");
        assert!(outcome.skipped);
        assert!(outcome.reason.contains("debounce"));
    }

    #[tokio::test]
    async fn mock_executor_success_maps_outcome_fields() {
        let executor: Arc<dyn SyncExecutor> = Arc::new(StaticSuccessExecutor {
            result: SyncExecutionResult {
                changed: true,
                packets: 7,
                warnings: vec!["layout warning".to_string()],
            },
        });
        let controller = SyncController::new_with_executor(executor, 5_000);

        let outcome = controller.trigger("manual").await.expect("sync success");
        assert!(!outcome.skipped);
        assert!(outcome.changed);
        assert_eq!(outcome.packets, 7);
        assert_eq!(outcome.warnings, vec!["layout warning".to_string()]);
        assert_eq!(outcome.reason, "manual");
    }

    #[tokio::test]
    async fn failed_execution_does_not_update_last_success() {
        let controller =
            SyncController::new_with_executor(Arc::new(StaticFailureExecutor), 60_000);

        let first = controller.trigger("first").await;
        assert!(first.is_err());

        let second = controller.trigger("second").await;
        assert!(
            second.is_err(),
            "second attempt should retry instead of debounce-skip"
        );

        let guard = controller.state.lock().await;
        assert!(!guard.in_flight);
        assert!(
            guard.last_success.is_none(),
            "failed sync should not update debounce timestamp"
        );
    }
}
