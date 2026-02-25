use std::path::Path;

use rat_config::ConfigStore;

use crate::discover::discover_packets;
use crate::header::{read_generated_header_packets, write_generated_header};
use crate::model::{SyncFsResult, SyncPipelineInput};
use crate::pipeline::run_sync_pipeline_with_previous_packets;
use crate::SyncError;

pub fn sync_packets_fs(
    config_path: impl AsRef<Path>,
    scan_root_override: Option<&Path>,
) -> Result<SyncFsResult, SyncError> {
    let config_path = config_path.as_ref();
    let store = ConfigStore::new(config_path);
    let cfg = store.load()?;
    let paths = store.paths_for(&cfg);

    let discovered_packets = discover_packets(&cfg, &paths, scan_root_override)?;
    let generated_header_path = paths.generated_header_path().to_path_buf();
    let previous_packets = read_generated_header_packets(&generated_header_path)?;

    let pipeline_output = run_sync_pipeline_with_previous_packets(
        SyncPipelineInput {
            project_name: cfg.project.name.clone(),
            discovered_packets,
        },
        &previous_packets,
    )?;

    write_generated_header(&generated_header_path, &pipeline_output.generated)?;

    cfg.validate()?;

    Ok(SyncFsResult {
        config: cfg,
        generated: pipeline_output.generated.clone(),
        packet_defs: pipeline_output.generated.packets,
        layout_warnings: pipeline_output.layout_warnings,
    })
}
