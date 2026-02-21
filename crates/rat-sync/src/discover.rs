use std::collections::HashSet;
use std::path::Path;

use rat_config::{ConfigPaths, RatitudeConfig};

use crate::assembler::assemble_discovered_packets;
use crate::ast::resolve_scan_root;
use crate::model::DiscoveredPacket;
use crate::parser_runner::parse_tagged_file;
use crate::scanner::scan_source_files;
use crate::SyncError;

pub(crate) fn discover_packets(
    cfg: &RatitudeConfig,
    paths: &ConfigPaths,
    scan_root_override: Option<&Path>,
) -> Result<Vec<DiscoveredPacket>, SyncError> {
    let scan_root = if let Some(override_path) = scan_root_override {
        resolve_scan_root(paths.config_path(), override_path)
    } else {
        paths.scan_root_path().to_path_buf()
    };

    let extension_set: HashSet<String> = cfg
        .project
        .extensions
        .iter()
        .map(|ext| ext.to_ascii_lowercase())
        .collect();

    let files = scan_source_files(
        &scan_root,
        cfg.project.recursive,
        &extension_set,
        paths.config_path(),
    )?;

    let mut discovered = Vec::new();
    for file in files {
        let Some(parsed) = parse_tagged_file(&file)? else {
            continue;
        };
        let packets = assemble_discovered_packets(&file, &scan_root, parsed)?;
        discovered.extend(packets);
    }

    Ok(discovered)
}
