use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use glob::Pattern;
use rat_config::RatitudeConfig;
use tree_sitter::Parser;
use walkdir::WalkDir;

use crate::ast::resolve_scan_root;
use crate::ids::compute_signature_hash;
use crate::model::DiscoveredPacket;
use crate::parser::{collect_comment_tags, collect_type_definitions};
use crate::SyncError;

#[derive(Debug, Clone)]
struct RttdIgnoreMatcher {
    root: PathBuf,
    patterns: Vec<Pattern>,
}

pub(crate) fn discover_packets(
    cfg: &RatitudeConfig,
    scan_root_override: Option<&Path>,
) -> Result<Vec<DiscoveredPacket>, SyncError> {
    let scan_root = if let Some(override_path) = scan_root_override {
        resolve_scan_root(cfg.config_path(), override_path)
    } else {
        cfg.scan_root_path().to_path_buf()
    };

    let extension_set: HashSet<String> = cfg
        .project
        .extensions
        .iter()
        .map(|ext| ext.to_ascii_lowercase())
        .collect();
    let rttd_ignore = load_rttdignore(cfg.config_path())?;

    let mut walker = WalkDir::new(&scan_root)
        .follow_links(false)
        .sort_by_file_name();
    if !cfg.project.recursive {
        walker = walker.max_depth(1);
    }

    let mut discovered = Vec::new();

    let mut iter = walker.into_iter();
    while let Some(entry) = iter.next() {
        let entry = entry.map_err(|err| SyncError::Validation(err.to_string()))?;
        if entry.file_type().is_dir() {
            let skip_by_pattern = rttd_ignore
                .as_ref()
                .map(|matcher| matcher.is_ignored(entry.path()))
                .unwrap_or(false);
            if skip_by_pattern {
                iter.skip_current_dir();
            }
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        if rttd_ignore
            .as_ref()
            .map(|matcher| matcher.is_ignored(entry.path()))
            .unwrap_or(false)
        {
            continue;
        }

        let ext = entry
            .path()
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| format!(".{}", value.to_ascii_lowercase()));
        let Some(ext) = ext else {
            continue;
        };
        if !extension_set.contains(&ext) {
            continue;
        }

        let packets = parse_tagged_file(entry.path(), &scan_root)?;
        discovered.extend(packets);
    }

    Ok(discovered)
}

impl RttdIgnoreMatcher {
    fn is_ignored(&self, path: &Path) -> bool {
        let relative = path
            .strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        self.patterns
            .iter()
            .any(|pattern| pattern.matches(&relative))
    }
}

fn load_rttdignore(config_path: &Path) -> Result<Option<RttdIgnoreMatcher>, SyncError> {
    let root = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let ignore_path = root.join(".rttdignore");
    if !ignore_path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&ignore_path).map_err(|source_err| SyncError::ReadSource {
        path: ignore_path.clone(),
        source: source_err,
    })?;
    let mut patterns = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('!') {
            return Err(SyncError::Validation(format!(
                ".rttdignore does not support negate patterns in {}:{}",
                ignore_path.display(),
                line_no
            )));
        }

        let pattern = Pattern::new(trimmed).map_err(|err| {
            SyncError::Validation(format!(
                "invalid .rttdignore pattern in {}:{} ({})",
                ignore_path.display(),
                line_no,
                err
            ))
        })?;
        patterns.push(pattern);
    }

    Ok(Some(RttdIgnoreMatcher { root, patterns }))
}

pub(crate) fn parse_tagged_file(
    path: &Path,
    scan_root: &Path,
) -> Result<Vec<DiscoveredPacket>, SyncError> {
    let source = fs::read(path).map_err(|source_err| SyncError::ReadSource {
        path: path.to_path_buf(),
        source: source_err,
    })?;

    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_c::LANGUAGE.into())
        .map_err(|err| {
            SyncError::Validation(format!(
                "tree-sitter init failed for {}: {err}",
                path.display()
            ))
        })?;
    let tree = parser
        .parse(&source, None)
        .ok_or_else(|| SyncError::ParseSource {
            path: path.to_path_buf(),
        })?;

    let tags = collect_comment_tags(&tree, &source, path)?;
    if tags.is_empty() {
        return Ok(Vec::new());
    }

    let mut structs = collect_type_definitions(&tree, &source, path)?;
    if structs.is_empty() {
        return Err(SyncError::Validation(format!(
            "found @rat tags in {} but no typedef struct definitions",
            path.display()
        )));
    }
    structs.sort_by_key(|value| value.start_byte);

    let mut out = Vec::with_capacity(tags.len());
    let mut used_structs = HashSet::new();

    for tag in tags {
        let mut matched = None;
        for (idx, st) in structs.iter().enumerate() {
            if st.start_byte < tag.end_byte {
                continue;
            }
            if used_structs.contains(&idx) {
                continue;
            }
            matched = Some((idx, st.clone()));
            break;
        }

        let (idx, st) = matched.ok_or_else(|| {
            SyncError::Validation(format!(
                "@rat tag in {}:{} has no following typedef struct",
                path.display(),
                tag.line
            ))
        })?;
        used_structs.insert(idx);

        let relative = path
            .strip_prefix(scan_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        let mut packet = DiscoveredPacket {
            signature_hash: 0,
            struct_name: st.name,
            packet_type: tag.packet_type,
            packed: st.packed,
            byte_size: st.byte_size,
            source: relative,
            fields: st.fields,
        };
        packet.signature_hash = compute_signature_hash(&packet);
        out.push(packet);
    }

    Ok(out)
}
