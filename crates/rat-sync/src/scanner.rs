use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use glob::Pattern;
use walkdir::WalkDir;

use crate::SyncError;

#[derive(Debug, Clone)]
struct RatIgnoreMatcher {
    root: PathBuf,
    patterns: Vec<Pattern>,
}

impl RatIgnoreMatcher {
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

pub(crate) fn scan_source_files(
    scan_root: &Path,
    recursive: bool,
    extension_set: &HashSet<String>,
    config_path: &Path,
) -> Result<Vec<PathBuf>, SyncError> {
    let rat_ignore = load_ratignore(config_path)?;

    let mut walker = WalkDir::new(scan_root)
        .follow_links(false)
        .sort_by_file_name();
    if !recursive {
        walker = walker.max_depth(1);
    }

    let mut files = Vec::new();

    let mut iter = walker.into_iter();
    while let Some(entry) = iter.next() {
        let entry = entry.map_err(|err| SyncError::Validation(err.to_string()))?;
        if entry.file_type().is_dir() {
            let skip_by_pattern = rat_ignore
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
        if rat_ignore
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

        files.push(entry.path().to_path_buf());
    }

    Ok(files)
}

fn load_ratignore(config_path: &Path) -> Result<Option<RatIgnoreMatcher>, SyncError> {
    let root = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    let ignore_path = root.join(".ratignore");
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
        let normalized = if idx == 0 {
            line.strip_prefix('\u{feff}').unwrap_or(line)
        } else {
            line
        };
        let trimmed = normalized.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('!') {
            return Err(SyncError::Validation(format!(
                ".ratignore does not support negate patterns in {}:{}",
                ignore_path.display(),
                line_no
            )));
        }

        let normalized_pattern = trimmed.replace('\\', "/");
        let normalized_pattern = normalized_pattern.trim_start_matches('/');
        if normalized_pattern.is_empty() {
            continue;
        }
        if normalized_pattern.ends_with('/') {
            let base_pattern = normalized_pattern.trim_end_matches('/');
            if base_pattern.is_empty() {
                continue;
            }
            let direct = Pattern::new(base_pattern).map_err(|err| {
                SyncError::Validation(format!(
                    "invalid .ratignore pattern in {}:{} ({})",
                    ignore_path.display(),
                    line_no,
                    err
                ))
            })?;
            let recursive = Pattern::new(&format!("{base_pattern}/**")).map_err(|err| {
                SyncError::Validation(format!(
                    "invalid .ratignore pattern in {}:{} ({})",
                    ignore_path.display(),
                    line_no,
                    err
                ))
            })?;
            patterns.push(direct);
            patterns.push(recursive);
            continue;
        }

        let pattern = Pattern::new(normalized_pattern).map_err(|err| {
            SyncError::Validation(format!(
                "invalid .ratignore pattern in {}:{} ({})",
                ignore_path.display(),
                line_no,
                err
            ))
        })?;
        patterns.push(pattern);
    }

    Ok(Some(RatIgnoreMatcher { root, patterns }))
}
