use rat_config::{GeneratedConfig, GeneratedMeta};

use crate::ids::{allocate_packet_ids, compute_fingerprint};
use crate::layout::{collect_layout_blockers, collect_layout_warnings};
use crate::model::{SyncPipelineInput, SyncPipelineOutput};
use crate::SyncError;

pub fn run_sync_pipeline(input: SyncPipelineInput) -> Result<SyncPipelineOutput, SyncError> {
    let layout_blockers = collect_layout_blockers(&input.discovered_packets);
    if !layout_blockers.is_empty() {
        return Err(SyncError::Validation(format!(
            "layout validation failed:\n- {}",
            layout_blockers.join("\n- ")
        )));
    }
    let layout_warnings = collect_layout_warnings(&input.discovered_packets);

    let packets = allocate_packet_ids(
        &input.discovered_packets,
        input.previous_generated_packets(),
    )?;
    let fingerprint = compute_fingerprint(&packets);

    let generated = GeneratedConfig {
        meta: GeneratedMeta {
            project: input.project_name,
            fingerprint: format!("0x{:016X}", fingerprint),
        },
        packets,
    };

    let changed = input
        .previous_generated
        .as_ref()
        .map(|previous| previous != &generated)
        .unwrap_or(true);

    Ok(SyncPipelineOutput {
        generated,
        changed,
        layout_warnings,
    })
}
