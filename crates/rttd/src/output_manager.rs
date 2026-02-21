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

#[derive(Clone)]
struct SinkContext {
    hub: Hub,
    packets: Vec<PacketDef>,
}

trait PacketSink {
    fn sync(&mut self, context: Option<&SinkContext>) -> Result<()>;
    fn shutdown(&mut self);
}

struct JsonlSink {
    enabled: bool,
    path: Option<String>,
    task: Option<JoinHandle<()>>,
}

impl JsonlSink {
    fn new() -> Self {
        Self {
            enabled: false,
            path: None,
            task: None,
        }
    }

    fn set_desired(&mut self, enabled: bool, path: Option<String>) {
        self.enabled = enabled;
        if let Some(path) = path {
            self.path = if path.trim().is_empty() {
                None
            } else {
                Some(path)
            };
        }
    }
}

impl PacketSink for JsonlSink {
    fn sync(&mut self, context: Option<&SinkContext>) -> Result<()> {
        self.shutdown();

        if !self.enabled {
            return Ok(());
        }

        let Some(context) = context else {
            return Ok(());
        };

        let writer: Box<dyn Write + Send> = if let Some(path) = &self.path {
            Box::new(
                File::create(path).with_context(|| format!("failed to open jsonl file {path}"))?,
            )
        } else {
            Box::new(io::stdout())
        };
        let writer = Arc::new(Mutex::new(writer));
        self.task = Some(spawn_jsonl_writer(context.hub.subscribe(), writer));

        Ok(())
    }

    fn shutdown(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

struct FoxgloveSink {
    enabled: bool,
    ws_addr: String,
    task: Option<JoinHandle<Result<()>>>,
    shutdown: Option<CancellationToken>,
}

impl FoxgloveSink {
    fn new(ws_addr: String) -> Self {
        Self {
            enabled: false,
            ws_addr,
            task: None,
            shutdown: None,
        }
    }

    fn set_desired(&mut self, enabled: bool, ws_addr: Option<String>) {
        self.enabled = enabled;
        if let Some(ws_addr) = ws_addr {
            if !ws_addr.trim().is_empty() {
                self.ws_addr = ws_addr;
            }
        }
    }
}

impl PacketSink for FoxgloveSink {
    fn sync(&mut self, context: Option<&SinkContext>) -> Result<()> {
        self.shutdown();

        if !self.enabled {
            return Ok(());
        }

        let Some(context) = context else {
            return Ok(());
        };

        let shutdown = CancellationToken::new();
        let bridge_cfg = BridgeConfig {
            ws_addr: self.ws_addr.clone(),
        };
        let task = tokio::spawn(run_bridge(
            bridge_cfg,
            context.packets.clone(),
            context.hub.clone(),
            shutdown.clone(),
        ));
        self.task = Some(task);
        self.shutdown = Some(shutdown);

        Ok(())
    }

    fn shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.cancel();
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub struct OutputManager {
    desired: OutputState,
    context: Option<SinkContext>,
    jsonl_sink: JsonlSink,
    foxglove_sink: FoxgloveSink,
}

impl OutputManager {
    pub fn from_config(cfg: &RatitudeConfig) -> Self {
        let jsonl_path = cfg.rttd.outputs.jsonl.path.trim();
        let desired = OutputState {
            jsonl_enabled: cfg.rttd.outputs.jsonl.enabled,
            jsonl_path: if jsonl_path.is_empty() {
                None
            } else {
                Some(jsonl_path.to_string())
            },
            foxglove_enabled: cfg.rttd.outputs.foxglove.enabled,
            foxglove_ws_addr: cfg.rttd.outputs.foxglove.ws_addr.clone(),
        };

        let mut jsonl_sink = JsonlSink::new();
        jsonl_sink.set_desired(desired.jsonl_enabled, desired.jsonl_path.clone());

        let mut foxglove_sink = FoxgloveSink::new(desired.foxglove_ws_addr.clone());
        foxglove_sink.set_desired(
            desired.foxglove_enabled,
            Some(desired.foxglove_ws_addr.clone()),
        );

        Self {
            desired,
            context: None,
            jsonl_sink,
            foxglove_sink,
        }
    }

    pub fn snapshot(&self) -> OutputState {
        self.desired.clone()
    }

    pub fn set_jsonl(&mut self, enabled: bool, path: Option<String>) -> Result<()> {
        self.desired.jsonl_enabled = enabled;
        if let Some(path) = path {
            self.desired.jsonl_path = if path.trim().is_empty() {
                None
            } else {
                Some(path)
            };
        }
        self.jsonl_sink
            .set_desired(self.desired.jsonl_enabled, self.desired.jsonl_path.clone());
        self.reconcile()
    }

    pub fn set_foxglove(&mut self, enabled: bool, ws_addr: Option<String>) -> Result<()> {
        self.desired.foxglove_enabled = enabled;
        if let Some(ws_addr) = ws_addr {
            if !ws_addr.trim().is_empty() {
                self.desired.foxglove_ws_addr = ws_addr;
            }
        }
        self.foxglove_sink.set_desired(
            self.desired.foxglove_enabled,
            Some(self.desired.foxglove_ws_addr.clone()),
        );
        self.reconcile()
    }

    pub async fn apply(&mut self, hub: Hub, packets: Vec<PacketDef>) -> Result<()> {
        self.context = Some(SinkContext { hub, packets });
        self.reconcile()
    }

    pub async fn shutdown(&mut self) {
        self.context = None;
        self.jsonl_sink.shutdown();
        self.foxglove_sink.shutdown();
    }

    fn reconcile(&mut self) -> Result<()> {
        self.jsonl_sink.sync(self.context.as_ref())?;
        self.foxglove_sink.sync(self.context.as_ref())?;
        Ok(())
    }
}
