use std::fs::File;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use rat_bridge_foxglove::{run_bridge, BridgeConfig};
use rat_config::{PacketDef, RatitudeConfig};
use rat_core::{spawn_jsonl_writer, Hub};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SinkKey {
    Jsonl,
    Foxglove,
}

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
    fn key(&self) -> SinkKey;
    fn sync(
        &mut self,
        desired: &OutputState,
        context: Option<&SinkContext>,
        failure_tx: &broadcast::Sender<String>,
    ) -> Result<()>;
    fn shutdown(&mut self);
}

struct RegisteredSink {
    key: SinkKey,
    sink: Box<dyn PacketSink>,
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

    fn sync(
        &mut self,
        desired: &OutputState,
        context: Option<&SinkContext>,
        failure_tx: &broadcast::Sender<String>,
    ) -> Result<()> {
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
        self.task = Some(spawn_jsonl_writer(
            context.hub.subscribe(),
            writer,
            failure_tx.clone(),
        ));

        Ok(())
    }

    fn shutdown(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

struct FoxgloveSink {
    task: Option<JoinHandle<()>>,
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

    fn sync(
        &mut self,
        desired: &OutputState,
        context: Option<&SinkContext>,
        failure_tx: &broadcast::Sender<String>,
    ) -> Result<()> {
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
        let failure_tx = failure_tx.clone();
        let task = tokio::spawn(async move {
            if let Err(err) = run_bridge(bridge_cfg, packets, hub, bridge_shutdown).await {
                let _ = failure_tx.send(err.to_string());
            }
        });
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
    sinks: Vec<RegisteredSink>,
    failure_tx: broadcast::Sender<String>,
    failure_rx: broadcast::Receiver<String>,
}

impl OutputManager {
    pub fn from_config(cfg: &RatitudeConfig) -> Self {
        let desired = output_state_from_config(cfg);
        let (failure_tx, failure_rx) = broadcast::channel::<String>(64);

        let mut sinks = Vec::new();
        register_sink(&mut sinks, Box::new(JsonlSink::new()));
        register_sink(&mut sinks, Box::new(FoxgloveSink::new()));

        Self {
            desired,
            context: None,
            sinks,
            failure_tx,
            failure_rx,
        }
    }

    #[cfg(test)]
    fn with_sinks_for_test(desired: OutputState, sinks: Vec<Box<dyn PacketSink>>) -> Self {
        let (failure_tx, failure_rx) = broadcast::channel::<String>(64);
        let mut registered = Vec::new();
        for sink in sinks {
            register_sink(&mut registered, sink);
        }
        Self {
            desired,
            context: None,
            sinks: registered,
            failure_tx,
            failure_rx,
        }
    }

    pub fn snapshot(&self) -> OutputState {
        self.desired.clone()
    }

    pub fn reload_from_config(&mut self, cfg: &RatitudeConfig) -> Result<()> {
        self.desired = output_state_from_config(cfg);
        self.reconcile_all()
    }

    pub async fn apply(&mut self, hub: Hub, packets: Vec<PacketDef>) -> Result<()> {
        self.context = Some(SinkContext { hub, packets });
        self.reconcile_all()
    }

    pub async fn shutdown(&mut self) {
        self.context = None;
        for entry in &mut self.sinks {
            entry.sink.shutdown();
        }
    }

    pub fn subscribe_failures(&self) -> broadcast::Receiver<String> {
        self.failure_tx.subscribe()
    }

    pub fn poll_failure(&mut self) -> Option<anyhow::Error> {
        match self.failure_rx.try_recv() {
            Ok(reason) => Some(anyhow!("output sink failure: {reason}")),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Closed) => Some(anyhow!("output sink failure channel closed")),
            Err(TryRecvError::Lagged(skipped)) => Some(anyhow!(
                "output sink failure channel lagged (skipped {skipped} messages)"
            )),
        }
    }

    fn reconcile_all(&mut self) -> Result<()> {
        for entry in &mut self.sinks {
            entry
                .sink
                .sync(&self.desired, self.context.as_ref(), &self.failure_tx)?;
        }
        Ok(())
    }
}

fn output_state_from_config(cfg: &RatitudeConfig) -> OutputState {
    let jsonl_path = cfg.ratd.outputs.jsonl.path.trim();
    OutputState {
        jsonl_enabled: cfg.ratd.outputs.jsonl.enabled,
        jsonl_path: if jsonl_path.is_empty() {
            None
        } else {
            Some(jsonl_path.to_string())
        },
        foxglove_enabled: cfg.ratd.outputs.foxglove.enabled,
        foxglove_ws_addr: cfg.ratd.outputs.foxglove.ws_addr.clone(),
    }
}

fn register_sink(sinks: &mut Vec<RegisteredSink>, sink: Box<dyn PacketSink>) {
    let key = sink.key();
    let duplicate = sinks.iter().any(|entry| entry.key == key);
    assert!(!duplicate, "duplicate sink key in OutputManager: {:?}", key);
    sinks.push(RegisteredSink { key, sink });
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailOnceSink {
        sent: bool,
    }

    impl PacketSink for FailOnceSink {
        fn key(&self) -> SinkKey {
            SinkKey::Jsonl
        }

        fn sync(
            &mut self,
            _desired: &OutputState,
            _context: Option<&SinkContext>,
            failure_tx: &broadcast::Sender<String>,
        ) -> Result<()> {
            if !self.sent {
                self.sent = true;
                let _ = failure_tx.send("sink failed".to_string());
            }
            Ok(())
        }

        fn shutdown(&mut self) {}
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
            vec![Box::new(FailOnceSink { sent: false })],
        );

        let cfg = RatitudeConfig::default();
        manager.reload_from_config(&cfg).expect("reload");

        let first = manager.poll_failure().expect("first failure");
        assert!(first.to_string().contains("sink failed"));
        assert!(manager.poll_failure().is_none());
    }

    #[test]
    fn reload_from_config_replaces_desired_state() {
        let mut manager = OutputManager::with_sinks_for_test(
            OutputState {
                jsonl_enabled: false,
                jsonl_path: None,
                foxglove_enabled: false,
                foxglove_ws_addr: "127.0.0.1:8765".to_string(),
            },
            vec![Box::new(FailOnceSink { sent: true })],
        );
        let mut cfg = RatitudeConfig::default();
        cfg.ratd.outputs.jsonl.enabled = true;
        cfg.ratd.outputs.jsonl.path = "out.jsonl".to_string();
        cfg.ratd.outputs.foxglove.enabled = true;
        cfg.ratd.outputs.foxglove.ws_addr = "127.0.0.1:9000".to_string();

        manager.reload_from_config(&cfg).expect("reload");
        let snapshot = manager.snapshot();
        assert!(snapshot.jsonl_enabled);
        assert_eq!(snapshot.jsonl_path.as_deref(), Some("out.jsonl"));
        assert!(snapshot.foxglove_enabled);
        assert_eq!(snapshot.foxglove_ws_addr, "127.0.0.1:9000");
    }
}
