use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use rat_config::RatitudeConfig;
use rat_core::{RuntimeSignal, SinkFailure};
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::cli::Cli;
use crate::command_loop::handle_console_command;
use crate::config_io::load_config;
use crate::console::spawn_console_reader;
use crate::output_manager::OutputManager;
use crate::runtime_lifecycle::{apply_schema_ready, restart_runtime, start_runtime};
use crate::runtime_schema::RuntimeSchemaState;
use crate::source_scan::render_candidates;
use crate::source_state::{build_source_domain, SourceDomainState};

const OUTPUT_RECOVERY_COOLDOWN: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub(crate) struct RuntimeDomainState {
    schema: RuntimeSchemaState,
    generation: u64,
}

impl RuntimeDomainState {
    fn new(schema: RuntimeSchemaState) -> Self {
        Self {
            schema,
            generation: 0,
        }
    }

    pub(crate) fn schema(&self) -> &RuntimeSchemaState {
        &self.schema
    }

    pub(crate) fn schema_mut(&mut self) -> &mut RuntimeSchemaState {
        &mut self.schema
    }

    pub(crate) fn clear_schema(&mut self) {
        self.schema.clear();
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn advance_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}

#[derive(Debug, Clone)]
pub struct DaemonState {
    config_path: String,
    config: RatitudeConfig,
    source: SourceDomainState,
    runtime: RuntimeDomainState,
}

impl DaemonState {
    pub(crate) fn new(
        config_path: String,
        config: RatitudeConfig,
        source: SourceDomainState,
    ) -> Self {
        Self {
            config_path,
            config,
            source,
            runtime: RuntimeDomainState::new(RuntimeSchemaState::default()),
        }
    }

    pub(crate) fn config_path(&self) -> &str {
        &self.config_path
    }

    pub(crate) fn config(&self) -> &RatitudeConfig {
        &self.config
    }

    pub(crate) fn replace_config(&mut self, config: RatitudeConfig) {
        self.config = config;
    }

    pub(crate) fn source(&self) -> &SourceDomainState {
        &self.source
    }

    pub(crate) fn source_mut(&mut self) -> &mut SourceDomainState {
        &mut self.source
    }

    pub(crate) fn runtime(&self) -> &RuntimeDomainState {
        &self.runtime
    }

    pub(crate) fn runtime_mut(&mut self) -> &mut RuntimeDomainState {
        &mut self.runtime
    }
}

pub async fn run_daemon(cli: Cli) -> Result<()> {
    let cfg = load_config(&cli.config).await?;
    let source = build_source_domain(&cfg.ratd.source).await?;
    let mut state = DaemonState::new(cli.config.clone(), cfg, source);
    render_candidates(state.source().candidates());
    let mut output_manager = OutputManager::from_config(state.config())?;
    let mut runtime = start_runtime(&mut state).await?;
    let mut output_failure_rx = output_manager.subscribe_failures();

    println!(
        "ratd daemon started at source {}",
        state.source().active_addr()
    );
    println!("type `$help` to show available commands");

    let console_shutdown = CancellationToken::new();
    let mut command_rx = spawn_console_reader(console_shutdown.clone());
    let mut console_attached = true;
    let mut output_failure_attached = true;
    let mut recovery_backoff = SinkRecoveryBackoff::new(OUTPUT_RECOVERY_COOLDOWN);
    let mut sink_recovery_tick = tokio::time::interval(OUTPUT_RECOVERY_COOLDOWN);
    sink_recovery_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    sink_recovery_tick.tick().await;

    let run_result: Result<()> = loop {
        tokio::select! {
            ctrl = tokio::signal::ctrl_c() => {
                match process_ctrl_c(ctrl) {
                    Ok(()) => break Ok(()),
                    Err(err) => break Err(err),
                }
            }
            command = command_rx.recv(), if console_attached => {
                match process_console_event(command, &mut state, &mut output_manager, &mut runtime).await {
                    Ok(console_event) => {
                        console_attached = console_event.keep_attached;
                        if console_event.should_quit {
                            break Ok(());
                        }
                    }
                    Err(err) => break Err(err),
                }
            }
            maybe_signal = runtime.recv_signal() => {
                if let Err(err) = process_runtime_signal(
                    maybe_signal,
                    &mut state,
                    &mut output_manager,
                    &runtime,
                )
                .await
                {
                    break Err(err);
                }
            }
            sink_failure = output_failure_rx.recv(), if output_failure_attached => {
                match process_output_failure(sink_failure, &mut output_manager, &mut recovery_backoff) {
                    Ok(keep_attached) => output_failure_attached = keep_attached,
                    Err(err) => break Err(err),
                }
            }
            _ = sink_recovery_tick.tick() => {
                process_periodic_sink_recovery(&mut output_manager, &mut recovery_backoff);
            }
        }
    };

    console_shutdown.cancel();
    output_manager.shutdown().await;
    runtime.shutdown().await;
    run_result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ConsoleEvent {
    keep_attached: bool,
    should_quit: bool,
}

impl ConsoleEvent {
    const fn continue_with(keep_attached: bool) -> Self {
        Self {
            keep_attached,
            should_quit: false,
        }
    }

    const fn quit() -> Self {
        Self {
            keep_attached: true,
            should_quit: true,
        }
    }
}

fn process_ctrl_c(ctrl: std::io::Result<()>) -> Result<()> {
    match ctrl.context("failed to wait ctrl-c") {
        Ok(()) => {
            info!("received ctrl-c, stopping daemon");
            Ok(())
        }
        Err(err) => Err(err),
    }
}

async fn process_console_event(
    command: Option<crate::console::ConsoleCommand>,
    state: &mut DaemonState,
    output_manager: &mut OutputManager,
    runtime: &mut rat_core::IngestRuntime,
) -> Result<ConsoleEvent> {
    let Some(command) = command else {
        return Ok(process_console_channel_closed());
    };

    let action = handle_console_command(command, state, output_manager).await?;
    if action.restart_runtime {
        restart_runtime(runtime, state, output_manager).await?;
    }
    if action.should_quit {
        return Ok(ConsoleEvent::quit());
    }

    Ok(ConsoleEvent::continue_with(true))
}

fn process_console_channel_closed() -> ConsoleEvent {
    warn!("console input stream closed; daemon continues without interactive command channel");
    ConsoleEvent::continue_with(false)
}

async fn process_runtime_signal(
    signal: Option<RuntimeSignal>,
    state: &mut DaemonState,
    output_manager: &mut OutputManager,
    runtime: &rat_core::IngestRuntime,
) -> Result<()> {
    match signal {
        Some(RuntimeSignal::SchemaReady {
            schema_hash,
            packets,
        }) => {
            apply_schema_ready(state, output_manager, runtime, schema_hash, packets).await?;
            println!(
                "runtime schema ready: packets={}, hash=0x{:016X}",
                state.runtime().schema().packet_count(),
                state
                    .runtime()
                    .schema()
                    .schema_hash()
                    .unwrap_or(schema_hash)
            );
            Ok(())
        }
        Some(RuntimeSignal::Fatal(err)) => Err(anyhow!(err.to_string())),
        None => Err(anyhow!("ingest runtime signal channel closed")),
    }
}

fn process_output_failure(
    sink_failure: std::result::Result<SinkFailure, tokio::sync::broadcast::error::RecvError>,
    output_manager: &mut OutputManager,
    recovery_backoff: &mut SinkRecoveryBackoff,
) -> Result<bool> {
    match sink_failure {
        Ok(failure) => {
            warn!(
                sink = failure.sink_key,
                reason = %failure.reason,
                "output sink failed; daemon keeps running"
            );
            attempt_sink_recovery(
                output_manager,
                recovery_backoff,
                failure.sink_key,
                Instant::now(),
            );
            Ok(true)
        }
        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
            warn!("output sink failure channel closed; daemon keeps running");
            Ok(false)
        }
        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
            warn!("output sink failure channel lagged (skipped {skipped} messages)");
            let now = Instant::now();
            for sink_key in output_manager.unhealthy_sink_keys() {
                attempt_sink_recovery(output_manager, recovery_backoff, sink_key, now);
            }
            Ok(true)
        }
    }
}

fn process_periodic_sink_recovery(
    output_manager: &mut OutputManager,
    recovery_backoff: &mut SinkRecoveryBackoff,
) {
    let now = Instant::now();
    for sink_key in output_manager.unhealthy_sink_keys() {
        attempt_sink_recovery(output_manager, recovery_backoff, sink_key, now);
    }
}

fn attempt_sink_recovery(
    output_manager: &mut OutputManager,
    recovery_backoff: &mut SinkRecoveryBackoff,
    sink_key: &'static str,
    now: Instant,
) {
    if !recovery_backoff.should_attempt(sink_key, now) {
        return;
    }
    if let Err(err) = output_manager.recover_sink_after_failure(sink_key) {
        warn!(
            sink = sink_key,
            error = %err,
            "failed to recover output sink after failure"
        );
    }
}

#[derive(Debug)]
struct SinkRecoveryBackoff {
    cooldown: Duration,
    next_retry_at: HashMap<&'static str, Instant>,
}

impl SinkRecoveryBackoff {
    fn new(cooldown: Duration) -> Self {
        Self {
            cooldown,
            next_retry_at: HashMap::new(),
        }
    }

    fn should_attempt(&mut self, sink_key: &'static str, now: Instant) -> bool {
        if let Some(next_retry) = self.next_retry_at.get(&sink_key) {
            if now < *next_retry {
                return false;
            }
        }
        self.next_retry_at.insert(sink_key, now + self.cooldown);
        true
    }
}

#[cfg(test)]
mod tests;
