use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use rat_sync::sync_packets;
use tokio::sync::Mutex;

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
    last_trigger: Option<Instant>,
    in_flight: bool,
}

#[derive(Clone)]
pub struct SyncController {
    config_path: String,
    debounce: Duration,
    state: Arc<Mutex<SyncState>>,
}

impl SyncController {
    pub fn new(config_path: String, debounce_ms: u64) -> Self {
        Self {
            config_path,
            debounce: Duration::from_millis(debounce_ms.max(1)),
            state: Arc::new(Mutex::new(SyncState {
                last_trigger: None,
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
            if let Some(last) = guard.last_trigger {
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
            guard.last_trigger = Some(Instant::now());
        }

        let config_path = self.config_path.clone();
        let join = tokio::task::spawn_blocking(move || sync_packets(&config_path, None));
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
                return Err(err.into());
            }
        };

        {
            let mut guard = self.state.lock().await;
            guard.in_flight = false;
        }

        Ok(SyncOutcome {
            changed: result.changed,
            packets: result.config.packets.len(),
            warnings: result.layout_warnings,
            skipped: false,
            reason: reason.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn debounce_prevents_back_to_back_trigger() {
        let controller = SyncController::new("/tmp/non-existent-rat.toml".to_string(), 5_000);
        {
            let mut guard = controller.state.lock().await;
            guard.last_trigger = Some(Instant::now());
        }

        let outcome = controller.trigger("test").await.expect("skip path");
        assert!(outcome.skipped);
        assert!(outcome.reason.contains("debounce"));
    }
}
