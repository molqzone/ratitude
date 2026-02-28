use std::path::{Component, Path, PathBuf};

use tree_sitter::Node;

use crate::SyncError;

pub(crate) fn resolve_scan_root(config_path: &Path, scan_root_override: &Path) -> PathBuf {
    let resolved = if scan_root_override.is_absolute() {
        scan_root_override.to_path_buf()
    } else {
        let base_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
        base_dir.join(scan_root_override)
    };
    normalize_path_lexical(&resolved)
}

fn normalize_path_lexical(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    let mut has_absolute_root = false;

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => {
                normalized.push(component.as_os_str());
                has_absolute_root = true;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                let can_pop_normal = normalized
                    .components()
                    .next_back()
                    .map(|last| matches!(last, Component::Normal(_)))
                    .unwrap_or(false);
                if can_pop_normal {
                    normalized.pop();
                } else if !has_absolute_root {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Normal(segment) => normalized.push(segment),
        }
    }

    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}

pub(crate) fn walk_nodes(
    node: Node,
    visitor: &mut dyn FnMut(Node) -> Result<(), SyncError>,
) -> Result<(), SyncError> {
    visitor(node)?;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_nodes(child, visitor)?;
    }
    Ok(())
}

pub(crate) fn find_first_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    if node.kind() == kind {
        return Some(node);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_first_kind(child, kind) {
            return Some(found);
        }
    }
    None
}

pub(crate) fn has_kind(node: Node<'_>, kind: &str) -> bool {
    find_first_kind(node, kind).is_some()
}

pub(crate) fn children_for_field<'a>(node: Node<'a>, field: &str) -> Vec<Node<'a>> {
    let mut out = Vec::new();
    let child_count = node.child_count();
    for idx in 0..child_count {
        let child_index = idx as u32;
        if node.field_name_for_child(child_index) == Some(field) {
            if let Some(child) = node.child(child_index) {
                out.push(child);
            }
        }
    }
    out
}

pub(crate) fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(ch) if ch == '_' || ch.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}
pub(crate) fn align_up(value: usize, align: usize) -> usize {
    if align <= 1 {
        return value;
    }
    let remainder = value % align;
    if remainder == 0 {
        value
    } else {
        value.saturating_add(align - remainder)
    }
}
