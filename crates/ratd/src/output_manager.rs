use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use rat_bridge_foxglove::{run_bridge, BridgeConfig};
use rat_config::{PacketDef, RatitudeConfig};
use rat_core::{spawn_jsonl_writer, Hub, SinkFailure};
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
        failure_tx: &broadcast::Sender<SinkFailure>,
    ) -> Result<()>;
    fn shutdown(&mut self);
    fn is_healthy(&self, _desired: &OutputState, _context: Option<&SinkContext>) -> bool {
        true
    }
    fn force_reconcile(&mut self) {
        self.shutdown();
    }
}

struct RegisteredSink {
    key: &'static str,
    sink: Box<dyn PacketSink>,
}

struct JsonlSink {
    task: Option<JoinHandle<()>>,
    last_state: Option<JsonlRuntimeState>,
    #[cfg(test)]
    restart_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct JsonlRuntimeState {
    enabled: bool,
    path: Option<String>,
    runtime_generation: Option<u64>,
}

impl JsonlSink {
    fn new() -> Self {
        Self {
            task: None,
            last_state: None,
            #[cfg(test)]
            restart_count: 0,
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
        failure_tx: &broadcast::Sender<SinkFailure>,
    ) -> Result<()> {
        let next_state = JsonlRuntimeState {
            enabled: desired.jsonl_enabled,
            path: desired.jsonl_path.clone(),
            runtime_generation: context.map(|ctx| ctx.key.runtime_generation),
        };
        if self.last_state.as_ref() == Some(&next_state) {
            return Ok(());
        }

        if self.task.is_some() {
            #[cfg(test)]
            {
                self.restart_count = self.restart_count.saturating_add(1);
            }
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
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .with_context(|| format!("failed to open jsonl file {path}"))?,
            )
        } else {
            Box::new(io::stdout())
        };
        let writer = Arc::new(Mutex::new(writer));
        let sink_key = self.key();
        self.task = Some(spawn_jsonl_writer(
            context.hub.subscribe(),
            writer,
            failure_tx.clone(),
            sink_key,
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

    fn is_healthy(&self, desired: &OutputState, context: Option<&SinkContext>) -> bool {
        let should_run = desired.jsonl_enabled && context.is_some();
        if !should_run {
            return true;
        }
        self.task.as_ref().is_some_and(|task| !task.is_finished())
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
        failure_tx: &broadcast::Sender<SinkFailure>,
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
        let sink_key = self.key();
        let task = tokio::spawn(async move {
            if let Err(err) = run_bridge(bridge_cfg, packets, hub, bridge_shutdown).await {
                let _ = failure_tx.send(SinkFailure {
                    sink_key,
                    reason: err.to_string(),
                });
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

    fn is_healthy(&self, desired: &OutputState, context: Option<&SinkContext>) -> bool {
        let should_run = desired.foxglove_enabled && context.is_some();
        if !should_run {
            return true;
        }
        self.task.as_ref().is_some_and(|task| !task.is_finished())
    }
}

pub struct OutputManager {
    desired: OutputState,
    context: Option<SinkContext>,
    sinks: Vec<RegisteredSink>,
    failure_tx: broadcast::Sender<SinkFailure>,
    unhealthy_sinks: BTreeSet<&'static str>,
}

impl OutputManager {
    pub fn from_config(cfg: &RatitudeConfig) -> Result<Self> {
        let desired = output_state_from_config(cfg);
        let (failure_tx, _failure_rx) = broadcast::channel::<SinkFailure>(64);

        let sinks = build_registered_sinks(default_sinks())?;

        Ok(Self {
            desired,
            context: None,
            sinks,
            failure_tx,
            unhealthy_sinks: BTreeSet::new(),
        })
    }

    #[cfg(test)]
    fn with_sinks_for_test(desired: OutputState, sinks: Vec<Box<dyn PacketSink>>) -> Result<Self> {
        let (failure_tx, _failure_rx) = broadcast::channel::<SinkFailure>(64);
        let registered = build_registered_sinks(sinks)?;
        Ok(Self {
            desired,
            context: None,
            sinks: registered,
            failure_tx,
            unhealthy_sinks: BTreeSet::new(),
        })
    }

    pub fn snapshot(&self) -> OutputState {
        self.desired.clone()
    }

    pub fn reload_from_config(&mut self, cfg: &RatitudeConfig) -> Result<()> {
        self.desired = output_state_from_config(cfg);
        self.reconcile_all_strict()
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
            self.context = Some(SinkContext { hub, packets, key });
        }

        self.reconcile_all_non_fatal();
        Ok(())
    }

    pub async fn shutdown(&mut self) {
        self.context = None;
        self.unhealthy_sinks.clear();
        for entry in &mut self.sinks {
            entry.sink.shutdown();
        }
    }

    pub fn subscribe_failures(&self) -> broadcast::Receiver<SinkFailure> {
        self.failure_tx.subscribe()
    }

    pub fn unhealthy_sink_keys(&self) -> Vec<&'static str> {
        self.unhealthy_sinks.iter().copied().collect()
    }

    pub fn refresh_unhealthy_sinks(&mut self) {
        let desired = &self.desired;
        let context = self.context.as_ref();
        for entry in &self.sinks {
            if entry.sink.is_healthy(desired, context) {
                self.unhealthy_sinks.remove(entry.key);
            } else {
                self.unhealthy_sinks.insert(entry.key);
            }
        }
    }

    pub fn mark_sink_unhealthy(&mut self, sink_key: &'static str) -> bool {
        let exists = self.sinks.iter().any(|entry| entry.key == sink_key);
        if exists {
            self.unhealthy_sinks.insert(sink_key);
        }
        exists
    }

    pub fn recover_sink_after_failure(&mut self, sink_key: &str) -> Result<()> {
        let Some(entry) = self.sinks.iter_mut().find(|entry| entry.key == sink_key) else {
            return Err(anyhow!("unknown sink key in OutputManager: {}", sink_key));
        };
        entry.sink.force_reconcile();
        reconcile_sink(
            entry,
            &self.desired,
            self.context.as_ref(),
            &self.failure_tx,
            &mut self.unhealthy_sinks,
        )
    }

    fn reconcile_all_strict(&mut self) -> Result<()> {
        for entry in &mut self.sinks {
            reconcile_sink(
                entry,
                &self.desired,
                self.context.as_ref(),
                &self.failure_tx,
                &mut self.unhealthy_sinks,
            )?;
        }
        Ok(())
    }

    fn reconcile_all_non_fatal(&mut self) {
        for entry in &mut self.sinks {
            if let Err(err) = reconcile_sink(
                entry,
                &self.desired,
                self.context.as_ref(),
                &self.failure_tx,
                &mut self.unhealthy_sinks,
            ) {
                let _ = self.failure_tx.send(SinkFailure {
                    sink_key: entry.key,
                    reason: format!("output sink apply failed: {err}"),
                });
            }
        }
    }
}

fn reconcile_sink(
    entry: &mut RegisteredSink,
    desired: &OutputState,
    context: Option<&SinkContext>,
    failure_tx: &broadcast::Sender<SinkFailure>,
    unhealthy_sinks: &mut BTreeSet<&'static str>,
) -> Result<()> {
    match entry.sink.sync(desired, context, failure_tx) {
        Ok(()) => {
            unhealthy_sinks.remove(entry.key);
            Ok(())
        }
        Err(err) => {
            unhealthy_sinks.insert(entry.key);
            Err(err)
        }
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

fn default_sinks() -> Vec<Box<dyn PacketSink>> {
    vec![Box::new(JsonlSink::new()), Box::new(FoxgloveSink::new())]
}

fn build_registered_sinks(sinks: Vec<Box<dyn PacketSink>>) -> Result<Vec<RegisteredSink>> {
    let mut registered = Vec::new();
    for sink in sinks {
        register_sink(&mut registered, sink)?;
    }
    Ok(registered)
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
mod tests;
