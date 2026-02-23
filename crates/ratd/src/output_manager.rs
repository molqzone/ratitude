use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use rat_bridge_foxglove::{run_bridge, BridgeConfig};
use rat_config::{PacketDef, RatitudeConfig};
use rat_core::{spawn_jsonl_writer, Hub};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;
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
    fn poll_failure(&mut self) -> Option<anyhow::Error>;
}

struct JsonlSink {
    task: Option<JoinHandle<()>>,
    failure_rx: Option<mpsc::UnboundedReceiver<String>>,
}

impl JsonlSink {
    fn new() -> Self {
        Self {
            task: None,
            failure_rx: None,
        }
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
        let (failure_tx, failure_rx) = mpsc::unbounded_channel::<String>();
        self.task = Some(spawn_jsonl_writer(
            context.hub.subscribe(),
            writer,
            failure_tx,
        ));
        self.failure_rx = Some(failure_rx);

        Ok(())
    }

    fn shutdown(&mut self) {
        self.failure_rx = None;
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }

    fn poll_failure(&mut self) -> Option<anyhow::Error> {
        let Some(failure_rx) = self.failure_rx.as_mut() else {
            return None;
        };

        match failure_rx.try_recv() {
            Ok(reason) => Some(anyhow!("jsonl sink stopped: {reason}")),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(anyhow!("jsonl sink stopped unexpectedly")),
        }
    }
}

struct FoxgloveSink {
    task: Option<JoinHandle<()>>,
    shutdown: Option<CancellationToken>,
    failure_rx: Option<mpsc::UnboundedReceiver<String>>,
}

impl FoxgloveSink {
    fn new() -> Self {
        Self {
            task: None,
            shutdown: None,
            failure_rx: None,
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
        let packets = context.packets.clone();
        let hub = context.hub.clone();
        let bridge_shutdown = shutdown.clone();
        let (failure_tx, failure_rx) = mpsc::unbounded_channel::<String>();
        let task = tokio::spawn(async move {
            if let Err(err) = run_bridge(bridge_cfg, packets, hub, bridge_shutdown).await {
                let _ = failure_tx.send(err.to_string());
            }
        });
        self.task = Some(task);
        self.shutdown = Some(shutdown);
        self.failure_rx = Some(failure_rx);

        Ok(())
    }

    fn shutdown(&mut self) {
        self.failure_rx = None;
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.cancel();
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }

    fn poll_failure(&mut self) -> Option<anyhow::Error> {
        let Some(failure_rx) = self.failure_rx.as_mut() else {
            return None;
        };

        match failure_rx.try_recv() {
            Ok(reason) => Some(anyhow!("foxglove bridge stopped: {reason}")),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(anyhow!("foxglove bridge stopped unexpectedly"))
            }
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
        let jsonl_path = cfg.ratd.outputs.jsonl.path.trim();
        let desired = OutputState {
            jsonl_enabled: cfg.ratd.outputs.jsonl.enabled,
            jsonl_path: if jsonl_path.is_empty() {
                None
            } else {
                Some(jsonl_path.to_string())
            },
            foxglove_enabled: cfg.ratd.outputs.foxglove.enabled,
            foxglove_ws_addr: cfg.ratd.outputs.foxglove.ws_addr.clone(),
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

    #[cfg(test)]
    fn with_sinks_for_test(desired: OutputState, sinks: Vec<Box<dyn PacketSink>>) -> Self {
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
        self.reconcile_sink(SinkKey::Jsonl)
    }

    pub fn set_foxglove(&mut self, enabled: bool, ws_addr: Option<String>) -> Result<()> {
        self.desired.foxglove_enabled = enabled;
        if let Some(ws_addr) = ws_addr {
            if !ws_addr.trim().is_empty() {
                self.desired.foxglove_ws_addr = ws_addr;
            }
        }
        self.reconcile_sink(SinkKey::Foxglove)
    }

    pub async fn apply(&mut self, hub: Hub, packets: Vec<PacketDef>) -> Result<()> {
        self.context = Some(SinkContext { hub, packets });
        self.reconcile_all()
    }

    pub async fn shutdown(&mut self) {
        self.context = None;
        for sink in &mut self.sinks {
            sink.shutdown();
        }
    }

    pub fn poll_failure(&mut self) -> Option<anyhow::Error> {
        for sink in &mut self.sinks {
            if let Some(err) = sink.poll_failure() {
                return Some(err);
            }
        }
        None
    }

    fn reconcile_all(&mut self) -> Result<()> {
        for sink in &mut self.sinks {
            sink.sync(&self.desired, self.context.as_ref())?;
        }
        Ok(())
    }

    fn reconcile_sink(&mut self, key: SinkKey) -> Result<()> {
        if let Some(sink) = self.sinks.iter_mut().find(|sink| sink.key() == key) {
            sink.sync(&self.desired, self.context.as_ref())?;
        }
        Ok(())
    }
}

fn validate_unique_sink_keys(sinks: &[Box<dyn PacketSink>]) {
    let mut seen = BTreeSet::new();
    for sink in sinks {
        let inserted = seen.insert(sink.key());
        assert!(inserted, "duplicate sink key in OutputManager");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailOnceSink {
        failed: bool,
    }

    impl PacketSink for FailOnceSink {
        fn key(&self) -> SinkKey {
            SinkKey::Jsonl
        }

        fn sync(&mut self, _desired: &OutputState, _context: Option<&SinkContext>) -> Result<()> {
            Ok(())
        }

        fn shutdown(&mut self) {}

        fn poll_failure(&mut self) -> Option<anyhow::Error> {
            if self.failed {
                return None;
            }
            self.failed = true;
            Some(anyhow!("sink failed"))
        }
    }

    #[test]
    fn poll_failure_returns_sink_error_once() {
        let mut manager = OutputManager::with_sinks_for_test(
            OutputState {
                jsonl_enabled: false,
                jsonl_path: None,
                foxglove_enabled: false,
                foxglove_ws_addr: "127.0.0.1:8765".to_string(),
            },
            vec![Box::new(FailOnceSink { failed: false })],
        );

        let first = manager.poll_failure().expect("first failure");
        assert!(first.to_string().contains("sink failed"));
        assert!(manager.poll_failure().is_none());
    }
}
