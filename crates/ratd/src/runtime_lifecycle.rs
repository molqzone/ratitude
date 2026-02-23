use std::time::Duration;

use anyhow::{anyhow, Result};
use rat_config::{FieldDef, PacketDef, RatitudeConfig};
use rat_core::{start_ingest_runtime, IngestRuntime, RuntimePacketDef};
use tracing::info;

use crate::config_io::load_config;
use crate::daemon::DaemonState;
use crate::output_manager::OutputManager;
use crate::runtime_spec::build_runtime_spec;

const UNKNOWN_PACKET_WINDOW: Duration = Duration::from_secs(5);
const UNKNOWN_PACKET_THRESHOLD: u32 = 20;

pub(crate) async fn activate_runtime(
    old_runtime: Option<IngestRuntime>,
    state: &mut DaemonState,
    output_manager: &mut OutputManager,
) -> Result<IngestRuntime> {
    if let Some(old_runtime) = old_runtime {
        output_manager.shutdown().await;
        old_runtime.shutdown(false).await;
    }

    state.config = load_config(&state.config_path).await?;
    let runtime = start_runtime(&state.config, &state.active_source).await?;
    state.runtime_schema.clear();
    info!(
        source = %state.active_source,
        packets = state.runtime_schema.packet_count(),
        "ingest runtime started"
    );
    Ok(runtime)
}

pub(crate) async fn apply_schema_ready(
    state: &mut DaemonState,
    output_manager: &mut OutputManager,
    runtime: &IngestRuntime,
    schema_hash: u64,
    packets: Vec<RuntimePacketDef>,
) -> Result<()> {
    let packet_defs = runtime_packets_to_packet_defs(packets);
    state.runtime_schema.replace(schema_hash, packet_defs);
    output_manager
        .apply(runtime.hub(), state.runtime_schema.packets().to_vec())
        .await
}

fn runtime_packets_to_packet_defs(runtime_packets: Vec<RuntimePacketDef>) -> Vec<PacketDef> {
    runtime_packets
        .into_iter()
        .map(|packet| PacketDef {
            id: packet.id,
            struct_name: packet.struct_name,
            packet_type: packet.packet_type,
            packed: packet.packed,
            byte_size: packet.byte_size,
            source: "runtime-schema".to_string(),
            fields: packet
                .fields
                .into_iter()
                .map(|field| FieldDef {
                    name: field.name,
                    c_type: field.c_type,
                    offset: field.offset,
                    size: field.size,
                })
                .collect(),
        })
        .collect()
}

async fn start_runtime(cfg: &RatitudeConfig, addr: &str) -> Result<IngestRuntime> {
    let spec = build_runtime_spec(cfg, addr, UNKNOWN_PACKET_WINDOW, UNKNOWN_PACKET_THRESHOLD)?;
    start_ingest_runtime(spec.ingest_config)
        .await
        .map_err(|err| anyhow!(err.to_string()))
}
