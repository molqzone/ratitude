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
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    let root = resolve_ignore_root(config_dir);

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

fn resolve_ignore_root(config_dir: &Path) -> PathBuf {
    let joined = if config_dir.is_absolute() {
        config_dir.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(config_dir),
            Err(_) => config_dir.to_path_buf(),
        }
    };

    joined.canonicalize().unwrap_or(joined)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn unique_rel_dir(prefix: &str) -> (String, PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let rel = format!("target/{prefix}_{unique}");
        let abs = std::env::current_dir().expect("cwd").join(&rel);
        (rel, abs)
    }

    #[test]
    fn ratignore_works_when_config_path_is_relative() {
        let (base_rel, base_abs) = unique_rel_dir("rat_sync_ignore_relative_config");
        let _ = fs::remove_dir_all(&base_abs);
        fs::create_dir_all(base_abs.join("src")).expect("mkdir src");

        fs::write(base_abs.join(".ratignore"), "src/skip.c\n").expect("write .ratignore");
        fs::write(base_abs.join("src").join("keep.c"), "int keep = 1;\n").expect("write keep");
        fs::write(base_abs.join("src").join("skip.c"), "int skip = 1;\n").expect("write skip");

        let mut extension_set = HashSet::new();
        extension_set.insert(".c".to_string());

        let config_rel = PathBuf::from(&base_rel).join("rat.toml");
        let files = scan_source_files(
            &base_abs.join("src"),
            true,
            &extension_set,
            config_rel.as_path(),
        )
        .expect("scan files");

        assert_eq!(files.len(), 1);
        assert_eq!(files[0], base_abs.join("src").join("keep.c"));

        let _ = fs::remove_dir_all(base_abs);
    }
}
