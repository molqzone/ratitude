use std::time::Duration;

use anyhow::{anyhow, Result};
use rat_config::{PacketDef, RatitudeConfig};
use rat_core::{start_ingest_runtime, IngestRuntime};
use tracing::info;

use crate::daemon::DaemonState;
use crate::output_manager::OutputManager;
use crate::runtime_spec::build_runtime_spec;

const UNKNOWN_PACKET_WINDOW: Duration = Duration::from_secs(5);
const UNKNOWN_PACKET_THRESHOLD: u32 = 20;

pub(crate) async fn start_runtime(state: &mut DaemonState) -> Result<IngestRuntime> {
    let runtime = start_runtime_from_config(state.config(), state.source().active_addr()).await?;
    mark_runtime_started(state);
    Ok(runtime)
}

pub(crate) async fn restart_runtime(
    runtime: &mut IngestRuntime,
    state: &mut DaemonState,
    output_manager: &mut OutputManager,
) -> Result<()> {
    output_manager.shutdown().await;
    runtime.shutdown().await;
    *runtime = start_runtime_from_config(state.config(), state.source().active_addr()).await?;
    mark_runtime_started(state);
    Ok(())
}

pub(crate) async fn apply_schema_ready(
    state: &mut DaemonState,
    output_manager: &mut OutputManager,
    runtime: &IngestRuntime,
    schema_hash: u64,
    packets: Vec<PacketDef>,
) -> Result<()> {
    state
        .runtime_mut()
        .schema_mut()
        .replace(schema_hash, packets);
    output_manager
        .apply(
            runtime.hub(),
            state.runtime().generation(),
            schema_hash,
            state.runtime().schema().packets().to_vec(),
        )
        .await
}

async fn start_runtime_from_config(cfg: &RatitudeConfig, addr: &str) -> Result<IngestRuntime> {
    let spec = build_runtime_spec(cfg, addr, UNKNOWN_PACKET_WINDOW, UNKNOWN_PACKET_THRESHOLD)?;
    start_ingest_runtime(spec.ingest_config)
        .await
        .map_err(|err| anyhow!(err.to_string()))
}

fn mark_runtime_started(state: &mut DaemonState) {
    state.runtime_mut().advance_generation();
    state.runtime_mut().clear_schema();
    info!(
        source = %state.source().active_addr(),
        packets = state.runtime().schema().packet_count(),
        "ingest runtime started"
    );
}
