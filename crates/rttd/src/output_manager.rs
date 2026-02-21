use std::fs::File;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rat_bridge_foxglove::{run_bridge, BridgeConfig};
use rat_config::{PacketDef, RatitudeConfig};
use rat_core::{spawn_jsonl_writer, Hub};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct OutputState {
    pub jsonl_enabled: bool,
    pub jsonl_path: Option<String>,
    pub foxglove_enabled: bool,
    pub foxglove_ws_addr: String,
}

pub struct OutputManager {
    desired: OutputState,
    jsonl_task: Option<JoinHandle<()>>,
    foxglove_task: Option<JoinHandle<Result<()>>>,
    foxglove_shutdown: Option<CancellationToken>,
}

impl OutputManager {
    pub fn from_config(cfg: &RatitudeConfig) -> Self {
        let jsonl_path = cfg.rttd.outputs.jsonl.path.trim();
        Self {
            desired: OutputState {
                jsonl_enabled: cfg.rttd.outputs.jsonl.enabled,
                jsonl_path: if jsonl_path.is_empty() {
                    None
                } else {
                    Some(jsonl_path.to_string())
                },
                foxglove_enabled: cfg.rttd.outputs.foxglove.enabled,
                foxglove_ws_addr: cfg.rttd.outputs.foxglove.ws_addr.clone(),
            },
            jsonl_task: None,
            foxglove_task: None,
            foxglove_shutdown: None,
        }
    }

    pub fn snapshot(&self) -> OutputState {
        self.desired.clone()
    }

    pub fn set_jsonl(&mut self, enabled: bool, path: Option<String>) {
        self.desired.jsonl_enabled = enabled;
        if let Some(path) = path {
            self.desired.jsonl_path = if path.trim().is_empty() {
                None
            } else {
                Some(path)
            };
        }
    }

    pub fn set_foxglove(&mut self, enabled: bool, ws_addr: Option<String>) {
        self.desired.foxglove_enabled = enabled;
        if let Some(ws_addr) = ws_addr {
            if !ws_addr.trim().is_empty() {
                self.desired.foxglove_ws_addr = ws_addr;
            }
        }
    }

    pub async fn apply(&mut self, hub: Hub, packets: Vec<PacketDef>) -> Result<()> {
        self.shutdown().await;

        if self.desired.jsonl_enabled {
            let writer: Box<dyn Write + Send> = if let Some(path) = &self.desired.jsonl_path {
                Box::new(
                    File::create(path)
                        .with_context(|| format!("failed to open jsonl file {path}"))?,
                )
            } else {
                Box::new(io::stdout())
            };
            let writer = Arc::new(Mutex::new(writer));
            self.jsonl_task = Some(spawn_jsonl_writer(hub.subscribe(), writer));
        }

        if self.desired.foxglove_enabled {
            let shutdown = CancellationToken::new();
            let bridge_cfg = BridgeConfig {
                ws_addr: self.desired.foxglove_ws_addr.clone(),
            };
            let task = tokio::spawn(run_bridge(bridge_cfg, packets, hub, shutdown.clone()));
            self.foxglove_task = Some(task);
            self.foxglove_shutdown = Some(shutdown);
        }

        Ok(())
    }

    pub async fn shutdown(&mut self) {
        if let Some(task) = self.jsonl_task.take() {
            task.abort();
            let _ = task.await;
        }

        if let Some(shutdown) = self.foxglove_shutdown.take() {
            shutdown.cancel();
        }
        if let Some(task) = self.foxglove_task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}
