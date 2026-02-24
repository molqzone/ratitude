use std::fs::File;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use rat_bridge_foxglove::{run_bridge, BridgeConfig};
use rat_config::{PacketDef, RatitudeConfig};
use rat_core::{spawn_jsonl_writer, Hub};
use tokio::sync::broadcast;
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
    key: SinkContextKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SinkContextKey {
    runtime_generation: u64,
    schema_hash: u64,
}

trait PacketSink {
    fn key(&self) -> &'static str;
    fn sync(
        &mut self,
        desired: &OutputState,
        context: Option<&SinkContext>,
        failure_tx: &broadcast::Sender<String>,
    ) -> Result<()>;
    fn shutdown(&mut self);
}

struct RegisteredSink {
    key: &'static str,
    sink: Box<dyn PacketSink>,
}

struct JsonlSink {
    task: Option<JoinHandle<()>>,
    last_state: Option<JsonlRuntimeState>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct JsonlRuntimeState {
    enabled: bool,
    path: Option<String>,
    context_key: Option<SinkContextKey>,
}

impl JsonlSink {
    fn new() -> Self {
        Self {
            task: None,
            last_state: None,
        }
    }
}

impl PacketSink for JsonlSink {
    fn key(&self) -> &'static str {
        "jsonl"
    }

    fn sync(
        &mut self,
        desired: &OutputState,
        context: Option<&SinkContext>,
        failure_tx: &broadcast::Sender<String>,
    ) -> Result<()> {
        let next_state = JsonlRuntimeState {
            enabled: desired.jsonl_enabled,
            path: desired.jsonl_path.clone(),
            context_key: context.map(|ctx| ctx.key),
        };
        if self.last_state.as_ref() == Some(&next_state) {
            return Ok(());
        }

        self.shutdown();

        if !next_state.enabled {
            self.last_state = Some(next_state);
            return Ok(());
        }

        let Some(context) = context else {
            self.last_state = Some(next_state);
            return Ok(());
        };

        let writer: Box<dyn Write + Send> = if let Some(path) = &next_state.path {
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
        self.last_state = Some(next_state);

        Ok(())
    }

    fn shutdown(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.last_state = None;
    }
}

struct FoxgloveSink {
    task: Option<JoinHandle<()>>,
    shutdown: Option<CancellationToken>,
    last_state: Option<FoxgloveRuntimeState>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FoxgloveRuntimeState {
    enabled: bool,
    ws_addr: String,
    context_key: Option<SinkContextKey>,
}

impl FoxgloveSink {
    fn new() -> Self {
        Self {
            task: None,
            shutdown: None,
            last_state: None,
        }
    }
}

impl PacketSink for FoxgloveSink {
    fn key(&self) -> &'static str {
        "foxglove"
    }

    fn sync(
        &mut self,
        desired: &OutputState,
        context: Option<&SinkContext>,
        failure_tx: &broadcast::Sender<String>,
    ) -> Result<()> {
        let next_state = FoxgloveRuntimeState {
            enabled: desired.foxglove_enabled,
            ws_addr: desired.foxglove_ws_addr.clone(),
            context_key: context.map(|ctx| ctx.key),
        };
        if self.last_state.as_ref() == Some(&next_state) {
            return Ok(());
        }

        self.shutdown();

        if !next_state.enabled {
            self.last_state = Some(next_state);
            return Ok(());
        }

        let Some(context) = context else {
            self.last_state = Some(next_state);
            return Ok(());
        };

        let shutdown = CancellationToken::new();
        let bridge_cfg = BridgeConfig {
            ws_addr: next_state.ws_addr.clone(),
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
        self.last_state = Some(next_state);

        Ok(())
    }

    fn shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.cancel();
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.last_state = None;
    }
}

pub struct OutputManager {
    desired: OutputState,
    context: Option<SinkContext>,
    sinks: Vec<RegisteredSink>,
    failure_tx: broadcast::Sender<String>,
}

impl OutputManager {
    pub fn from_config(cfg: &RatitudeConfig) -> Result<Self> {
        let desired = output_state_from_config(cfg);
        let (failure_tx, _failure_rx) = broadcast::channel::<String>(64);

        let mut sinks = Vec::new();
        register_sink(&mut sinks, Box::new(JsonlSink::new()))?;
        register_sink(&mut sinks, Box::new(FoxgloveSink::new()))?;

        Ok(Self {
            desired,
            context: None,
            sinks,
            failure_tx,
        })
    }

    #[cfg(test)]
    fn with_sinks_for_test(desired: OutputState, sinks: Vec<Box<dyn PacketSink>>) -> Result<Self> {
        let (failure_tx, _failure_rx) = broadcast::channel::<String>(64);
        let mut registered = Vec::new();
        for sink in sinks {
            register_sink(&mut registered, sink)?;
        }
        Ok(Self {
            desired,
            context: None,
            sinks: registered,
            failure_tx,
        })
    }

    pub fn snapshot(&self) -> OutputState {
        self.desired.clone()
    }

    pub fn reload_from_config(&mut self, cfg: &RatitudeConfig) -> Result<()> {
        self.desired = output_state_from_config(cfg);
        self.reconcile_all()
    }

    pub async fn apply(
        &mut self,
        hub: Hub,
        runtime_generation: u64,
        schema_hash: u64,
        packets: Vec<PacketDef>,
    ) -> Result<()> {
        let key = SinkContextKey {
            runtime_generation,
            schema_hash,
        };

        if self.context.as_ref().map(|ctx| ctx.key) != Some(key) {
            self.context = Some(SinkContext {
                hub,
                packets,
                key,
            });
        }

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

fn register_sink(sinks: &mut Vec<RegisteredSink>, sink: Box<dyn PacketSink>) -> Result<()> {
    let key = sink.key();
    let duplicate = sinks.iter().any(|entry| entry.key == key);
    if duplicate {
        return Err(anyhow!("duplicate sink key in OutputManager: {}", key));
    }
    sinks.push(RegisteredSink { key, sink });
    Ok(())
}

#[cfg(test)]
mod tests {
    use tokio::sync::broadcast::error::TryRecvError;

    use super::*;

    struct FailOnceSink {
        sent: bool,
    }

    struct NoopSink;

    impl PacketSink for FailOnceSink {
        fn key(&self) -> &'static str {
            "jsonl"
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

    impl PacketSink for NoopSink {
        fn key(&self) -> &'static str {
            "jsonl"
        }

        fn sync(
            &mut self,
            _desired: &OutputState,
            _context: Option<&SinkContext>,
            _failure_tx: &broadcast::Sender<String>,
        ) -> Result<()> {
            Ok(())
        }

        fn shutdown(&mut self) {}
    }

    #[test]
    fn failure_subscription_receives_sink_error_once() {
        let mut manager = OutputManager::with_sinks_for_test(
            OutputState {
                jsonl_enabled: false,
                jsonl_path: None,
                foxglove_enabled: false,
                foxglove_ws_addr: "127.0.0.1:8765".to_string(),
            },
            vec![Box::new(FailOnceSink { sent: false })],
        )
        .expect("build output manager");
        let mut failures = manager.subscribe_failures();

        let cfg = RatitudeConfig::default();
        manager.reload_from_config(&cfg).expect("reload");

        let first = failures.try_recv().expect("first failure");
        assert!(first.contains("sink failed"));
        assert!(matches!(failures.try_recv(), Err(TryRecvError::Empty)));
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
        )
        .expect("build output manager");
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

    #[test]
    fn with_sinks_for_test_rejects_duplicate_sink_keys() {
        let result = OutputManager::with_sinks_for_test(
            OutputState {
                jsonl_enabled: false,
                jsonl_path: None,
                foxglove_enabled: false,
                foxglove_ws_addr: "127.0.0.1:8765".to_string(),
            },
            vec![
                Box::new(FailOnceSink { sent: true }),
                Box::new(FailOnceSink { sent: true }),
            ],
        );
        match result {
            Ok(_) => panic!("duplicate sink keys should fail"),
            Err(err) => assert!(err.to_string().contains("duplicate sink key")),
        }
    }

    #[tokio::test]
    async fn apply_keeps_context_key_for_same_runtime_generation_and_hash() {
        let mut manager = OutputManager::with_sinks_for_test(
            OutputState {
                jsonl_enabled: false,
                jsonl_path: None,
                foxglove_enabled: false,
                foxglove_ws_addr: "127.0.0.1:8765".to_string(),
            },
            vec![Box::new(NoopSink)],
        )
        .expect("build output manager");
        let hub = Hub::new(8);

        manager
            .apply(hub.clone(), 1, 0xABCD_EF01_u64, Vec::new())
            .await
            .expect("first apply");
        let first_key = manager.context.as_ref().map(|ctx| ctx.key);

        manager
            .apply(hub, 1, 0xABCD_EF01_u64, Vec::new())
            .await
            .expect("second apply");
        assert_eq!(manager.context.as_ref().map(|ctx| ctx.key), first_key);
    }

    #[tokio::test]
    async fn apply_updates_context_key_when_runtime_generation_changes() {
        let mut manager = OutputManager::with_sinks_for_test(
            OutputState {
                jsonl_enabled: false,
                jsonl_path: None,
                foxglove_enabled: false,
                foxglove_ws_addr: "127.0.0.1:8765".to_string(),
            },
            vec![Box::new(NoopSink)],
        )
        .expect("build output manager");
        let hub = Hub::new(8);

        manager
            .apply(hub.clone(), 1, 0x1111_u64, Vec::new())
            .await
            .expect("first apply");
        let first_key = manager.context.as_ref().map(|ctx| ctx.key);

        manager
            .apply(hub, 2, 0x1111_u64, Vec::new())
            .await
            .expect("second apply");
        assert_ne!(manager.context.as_ref().map(|ctx| ctx.key), first_key);
    }
}
