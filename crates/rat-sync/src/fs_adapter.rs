use std::path::Path;

use rat_config::{load_generated_or_default, save_generated, ConfigStore};

use crate::discover::discover_packets;
use crate::header::write_generated_header;
use crate::model::{SyncFsResult, SyncPipelineInput};
use crate::pipeline::run_sync_pipeline;
use crate::SyncError;

pub fn sync_packets_fs(
    config_path: impl AsRef<Path>,
    scan_root_override: Option<&Path>,
) -> Result<SyncFsResult, SyncError> {
    let config_path = config_path.as_ref();
    let store = ConfigStore::new(config_path);
    let (cfg, _) = store.load_or_default()?;
    let paths = store.paths_for(&cfg);

    let discovered_packets = discover_packets(&cfg, &paths, scan_root_override)?;
    let generated_path = paths.generated_toml_path().to_path_buf();
    let generated_header_path = paths.generated_header_path().to_path_buf();

    let (previous_generated, previous_exists) = load_generated_or_default(&generated_path)?;
    let pipeline_output = run_sync_pipeline(SyncPipelineInput {
        project_name: cfg.project.name.clone(),
        discovered_packets,
        previous_generated: if previous_exists {
            Some(previous_generated)
        } else {
            None
        },
    })?;

    if pipeline_output.changed {
        save_generated(&generated_path, &pipeline_output.generated)?;
    }
    if pipeline_output.changed || !generated_header_path.exists() {
        write_generated_header(&generated_header_path, &pipeline_output.generated)?;
    }

    cfg.validate()?;

    Ok(SyncFsResult {
        config: cfg,
        generated: pipeline_output.generated.clone(),
        packet_defs: pipeline_output.generated.packets,
        changed: pipeline_output.changed,
        layout_warnings: pipeline_output.layout_warnings,
    })
}
