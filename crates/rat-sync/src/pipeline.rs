use crate::generated::{GeneratedConfig, GeneratedMeta, GeneratedPacketDef};
use crate::ids::allocate_packet_ids;
use crate::layout::{collect_layout_blockers, collect_layout_warnings};
use crate::model::{SyncPipelineInput, SyncPipelineOutput};
use crate::schema::compute_runtime_schema_hash_from_packets;
use crate::SyncError;

pub fn run_sync_pipeline(input: SyncPipelineInput) -> Result<SyncPipelineOutput, SyncError> {
    run_sync_pipeline_with_previous_packets(input, &[])
}

pub(crate) fn run_sync_pipeline_with_previous_packets(
    input: SyncPipelineInput,
    previous_packets: &[GeneratedPacketDef],
) -> Result<SyncPipelineOutput, SyncError> {
    let layout_blockers = collect_layout_blockers(&input.discovered_packets);
    if !layout_blockers.is_empty() {
        return Err(SyncError::Validation(format!(
            "layout validation failed:\n- {}",
            layout_blockers.join("\n- ")
        )));
    }
    let layout_warnings = collect_layout_warnings(&input.discovered_packets);

    let packets = allocate_packet_ids(&input.discovered_packets, previous_packets)?;
    let schema_hash = compute_runtime_schema_hash_from_packets(&packets);

    let generated = GeneratedConfig {
        meta: GeneratedMeta {
            project: input.project_name,
            schema_hash: format!("0x{:016X}", schema_hash),
        },
        packets,
    };

    Ok(SyncPipelineOutput {
        generated,
        layout_warnings,
    })
}
