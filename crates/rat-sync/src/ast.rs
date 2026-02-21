use std::path::{Path, PathBuf};

use tree_sitter::Node;

use crate::SyncError;

pub(crate) fn resolve_scan_root(config_path: &Path, scan_root_override: &Path) -> PathBuf {
    if scan_root_override.is_absolute() {
        return scan_root_override.to_path_buf();
    }

    let base_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    base_dir.join(scan_root_override)
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
        if node.field_name_for_child(child_index).as_deref() == Some(field) {
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

pub(crate) fn normalize_c_type(raw: &str) -> String {
    let mut value = raw.trim().to_ascii_lowercase();
    while value.contains("  ") {
        value = value.replace("  ", " ");
    }
    value = value
        .trim_start_matches("const ")
        .trim_start_matches("volatile ")
        .to_string();
    value.trim().to_string()
}

pub(crate) fn c_type_size(c_type: &str) -> Option<usize> {
    match c_type {
        "float" => Some(4),
        "double" => Some(8),
        "int8_t" | "uint8_t" | "bool" | "_bool" => Some(1),
        "int16_t" | "uint16_t" => Some(2),
        "int32_t" | "uint32_t" => Some(4),
        "int64_t" | "uint64_t" => Some(8),
        _ => None,
    }
}

pub(crate) fn align_up(value: usize, align: usize) -> usize {
    if align <= 1 {
        return value;
    }
    let remainder = value % align;
    if remainder == 0 {
        value
    } else {
        value + (align - remainder)
    }
}
