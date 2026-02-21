use std::collections::BTreeSet;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SinkKey {
    Jsonl,
    Foxglove,
}

trait PacketSink {
    fn key(&self) -> SinkKey;
    fn sync(&mut self, desired: &OutputState, context: Option<&SinkContext>) -> Result<()>;
    fn shutdown(&mut self);
}

struct JsonlSink {
    task: Option<JoinHandle<()>>,
}

impl JsonlSink {
    fn new() -> Self {
        Self { task: None }
    }
}

impl PacketSink for JsonlSink {
    fn key(&self) -> SinkKey {
        SinkKey::Jsonl
    }

    fn sync(&mut self, desired: &OutputState, context: Option<&SinkContext>) -> Result<()> {
        self.shutdown();

        if !desired.jsonl_enabled {
            return Ok(());
        }

        let Some(context) = context else {
            return Ok(());
        };

        let writer: Box<dyn Write + Send> = if let Some(path) = &desired.jsonl_path {
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
    task: Option<JoinHandle<Result<()>>>,
    shutdown: Option<CancellationToken>,
}

impl FoxgloveSink {
    fn new() -> Self {
        Self {
            task: None,
            shutdown: None,
        }
    }
}

impl PacketSink for FoxgloveSink {
    fn key(&self) -> SinkKey {
        SinkKey::Foxglove
    }

    fn sync(&mut self, desired: &OutputState, context: Option<&SinkContext>) -> Result<()> {
        self.shutdown();

        if !desired.foxglove_enabled {
            return Ok(());
        }

        let Some(context) = context else {
            return Ok(());
        };

        let shutdown = CancellationToken::new();
        let bridge_cfg = BridgeConfig {
            ws_addr: desired.foxglove_ws_addr.clone(),
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
    sinks: Vec<Box<dyn PacketSink>>,
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

        let sinks: Vec<Box<dyn PacketSink>> =
            vec![Box::new(JsonlSink::new()), Box::new(FoxgloveSink::new())];
        validate_unique_sink_keys(&sinks);

        Self {
            desired,
            context: None,
            sinks,
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
        self.reconcile()
    }

    pub fn set_foxglove(&mut self, enabled: bool, ws_addr: Option<String>) -> Result<()> {
        self.desired.foxglove_enabled = enabled;
        if let Some(ws_addr) = ws_addr {
            if !ws_addr.trim().is_empty() {
                self.desired.foxglove_ws_addr = ws_addr;
            }
        }
        self.reconcile()
    }

    pub async fn apply(&mut self, hub: Hub, packets: Vec<PacketDef>) -> Result<()> {
        self.context = Some(SinkContext { hub, packets });
        self.reconcile()
    }

    pub async fn shutdown(&mut self) {
        self.context = None;
        for sink in &mut self.sinks {
            sink.shutdown();
        }
    }

    fn reconcile(&mut self) -> Result<()> {
        for sink in &mut self.sinks {
            sink.sync(&self.desired, self.context.as_ref())?;
        }
        Ok(())
    }
}

fn validate_unique_sink_keys(sinks: &[Box<dyn PacketSink>]) {
    let mut seen = BTreeSet::new();
    for sink in sinks {
        let inserted = seen.insert(sink.key());
        debug_assert!(inserted, "duplicate sink key in OutputManager");
    }
}
