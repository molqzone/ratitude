use anyhow::{anyhow, Context, Result};
use rat_config::RatitudeConfig;
use rat_core::RuntimeSignal;
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

    let run_result: Result<()> = loop {
        tokio::select! {
            ctrl = tokio::signal::ctrl_c() => {
                match process_ctrl_c(ctrl) {
                    Ok(LoopControl::Continue) => {}
                    Ok(LoopControl::Quit) => break Ok(()),
                    Err(err) => break Err(err),
                }
            }
            command = command_rx.recv(), if console_attached => {
                match process_console_event(command, &mut state, &mut output_manager, &mut runtime).await {
                    Ok(console_state) => {
                        console_attached = console_state.keep_attached;
                        if matches!(console_state.loop_control, LoopControl::Quit) {
                            break Ok(());
                        }
                    }
                    Err(err) => break Err(err),
                }
            }
            maybe_signal = runtime.recv_signal() => {
                match process_runtime_signal(
                    maybe_signal,
                    &mut state,
                    &mut output_manager,
                    &runtime,
                )
                .await
                {
                    Ok(LoopControl::Continue) => {}
                    Ok(LoopControl::Quit) => break Ok(()),
                    Err(err) => break Err(err),
                }
            }
            sink_failure = output_failure_rx.recv() => {
                match process_output_failure(sink_failure) {
                    Ok(LoopControl::Continue) => {}
                    Ok(LoopControl::Quit) => break Ok(()),
                    Err(err) => break Err(err),
                }
            }
        }
    };

    console_shutdown.cancel();
    output_manager.shutdown().await;
    runtime.shutdown().await;
    run_result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopControl {
    Continue,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ConsoleLoopState {
    loop_control: LoopControl,
    keep_attached: bool,
}

impl ConsoleLoopState {
    const fn continue_with(keep_attached: bool) -> Self {
        Self {
            loop_control: LoopControl::Continue,
            keep_attached,
        }
    }

    const fn quit() -> Self {
        Self {
            loop_control: LoopControl::Quit,
            keep_attached: true,
        }
    }
}

fn process_ctrl_c(ctrl: std::io::Result<()>) -> Result<LoopControl> {
    match ctrl.context("failed to wait ctrl-c") {
        Ok(()) => {
            info!("received ctrl-c, stopping daemon");
            Ok(LoopControl::Quit)
        }
        Err(err) => Err(err),
    }
}

async fn process_console_event(
    command: Option<crate::console::ConsoleCommand>,
    state: &mut DaemonState,
    output_manager: &mut OutputManager,
    runtime: &mut rat_core::IngestRuntime,
) -> Result<ConsoleLoopState> {
    let Some(command) = command else {
        return Ok(process_console_channel_closed());
    };

    let action = handle_console_command(command, state, output_manager).await?;
    if action.restart_runtime {
        restart_runtime(runtime, state, output_manager).await?;
    }
    if action.should_quit {
        return Ok(ConsoleLoopState::quit());
    }

    Ok(ConsoleLoopState::continue_with(true))
}

fn process_console_channel_closed() -> ConsoleLoopState {
    warn!("console input stream closed; daemon continues without interactive command channel");
    ConsoleLoopState::continue_with(false)
}

async fn process_runtime_signal(
    signal: Option<RuntimeSignal>,
    state: &mut DaemonState,
    output_manager: &mut OutputManager,
    runtime: &rat_core::IngestRuntime,
) -> Result<LoopControl> {
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
            Ok(LoopControl::Continue)
        }
        Some(RuntimeSignal::Fatal(err)) => Err(anyhow!(err.to_string())),
        None => Err(anyhow!("ingest runtime signal channel closed")),
    }
}

fn process_output_failure(
    sink_failure: std::result::Result<String, tokio::sync::broadcast::error::RecvError>,
) -> Result<LoopControl> {
    match sink_failure {
        Ok(reason) => {
            warn!(reason = %reason, "output sink failed; daemon keeps running");
            Ok(LoopControl::Continue)
        }
        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
            warn!("output sink failure channel closed; daemon keeps running");
            Ok(LoopControl::Continue)
        }
        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
            warn!("output sink failure channel lagged (skipped {skipped} messages)");
            Ok(LoopControl::Continue)
        }
    }
}

#[cfg(test)]
mod tests;
