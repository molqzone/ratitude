use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use rat_config::{ConfigError, FieldDef, PacketDef, RatitudeConfig};
use regex::Regex;
use thiserror::Error;
use tree_sitter::{Node, Parser, Tree};
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, Error)]
pub enum SyncError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("failed to read source file {path}: {source}")]
    ReadSource {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse C source {path}")]
    ParseSource { path: PathBuf },
    #[error("sync validation failed: {0}")]
    Validation(String),
}

#[derive(Debug, Clone)]
pub struct SyncResult {
    pub config: RatitudeConfig,
    pub changed: bool,
}

#[derive(Debug, Clone)]
struct DiscoveredPacket {
    id: u16,
    struct_name: String,
    packet_type: String,
    packed: bool,
    byte_size: usize,
    source: String,
    fields: Vec<FieldDef>,
}

#[derive(Debug, Clone)]
struct TagMatch {
    end_byte: usize,
    id: u16,
    packet_type: String,
    line: usize,
}

#[derive(Debug, Clone)]
struct StructDef {
    start_byte: usize,
    name: String,
    packed: bool,
    byte_size: usize,
    fields: Vec<FieldDef>,
}

#[derive(Debug, Clone)]
struct FieldSpec {
    name: String,
    c_type: String,
    size: usize,
}

pub fn sync_packets(
    config_path: impl AsRef<Path>,
    scan_root_override: Option<&Path>,
) -> Result<SyncResult, SyncError> {
    let config_path = config_path.as_ref();
    let (mut cfg, exists) = rat_config::load_or_default(config_path)?;

    let discovered = discover_packets(&cfg, scan_root_override)?;
    let merged = merge_packets(&cfg.packets, discovered);

    let mut old_packets = cfg.packets.clone();
    old_packets.sort_by_key(|packet| packet.id);
    let changed = old_packets != merged;

    cfg.packets = merged;
    if !exists || changed {
        cfg.save(config_path)?;
    }

    Ok(SyncResult {
        config: cfg,
        changed,
    })
}

fn discover_packets(
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
    let ignore_dirs: HashSet<String> = cfg.project.ignore_dirs.iter().cloned().collect();

    let mut walker = WalkDir::new(&scan_root).follow_links(false);
    if !cfg.project.recursive {
        walker = walker.max_depth(1);
    }

    let mut discovered = Vec::new();
    let mut seen_ids: HashMap<u16, String> = HashMap::new();

    let mut iter = walker.into_iter();
    while let Some(entry) = iter.next() {
        let entry = entry.map_err(|err| SyncError::Validation(err.to_string()))?;
        if should_skip_dir(&entry, &scan_root, &ignore_dirs) {
            iter.skip_current_dir();
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }

        let ext = entry
            .path()
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| format!(".{}", value.to_ascii_lowercase()));
        if let Some(ext) = ext {
            if !extension_set.contains(&ext) {
                continue;
            }
        } else {
            continue;
        }

        let packets = parse_tagged_file(entry.path(), &scan_root)?;
        for packet in packets {
            if let Some(prev) = seen_ids.insert(packet.id, packet.source.clone()) {
                return Err(SyncError::Validation(format!(
                    "duplicate packet id 0x{:02X} in {} and {}",
                    packet.id, prev, packet.source
                )));
            }
            discovered.push(packet);
        }
    }

    discovered.sort_by_key(|packet| packet.id);
    Ok(discovered)
}

fn should_skip_dir(entry: &DirEntry, scan_root: &Path, ignore_dirs: &HashSet<String>) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    if entry.path() == scan_root {
        return false;
    }
    match entry.file_name().to_str() {
        Some(name) => ignore_dirs.contains(name),
        None => false,
    }
}

fn parse_tagged_file(path: &Path, scan_root: &Path) -> Result<Vec<DiscoveredPacket>, SyncError> {
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
                "@rat tag id=0x{:02X} in {}:{} has no following typedef struct",
                tag.id,
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

        out.push(DiscoveredPacket {
            id: tag.id,
            struct_name: st.name,
            packet_type: tag.packet_type,
            packed: st.packed,
            byte_size: st.byte_size,
            source: relative,
            fields: st.fields,
        });
    }

    Ok(out)
}

fn collect_comment_tags(
    tree: &Tree,
    source: &[u8],
    path: &Path,
) -> Result<Vec<TagMatch>, SyncError> {
    let tag_re = Regex::new(r"@rat:id=(0x[0-9A-Fa-f]+)\s*,\s*type=([A-Za-z_][A-Za-z0-9_]*)")
        .map_err(|err| SyncError::Validation(format!("invalid tag regex: {err}")))?;

    let mut tags = Vec::new();
    walk_nodes(tree.root_node(), &mut |node| {
        if node.kind() != "comment" {
            return Ok(());
        }

        let text = node.utf8_text(source).map_err(|err| {
            SyncError::Validation(format!("invalid utf8 comment in {}: {err}", path.display()))
        })?;

        let mut matches = tag_re.captures_iter(text);
        let first = matches.next();
        if first.is_none() {
            return Ok(());
        }
        if matches.next().is_some() {
            return Err(SyncError::Validation(format!(
                "multiple @rat tags in one comment block at {}:{}",
                path.display(),
                node.start_position().row + 1
            )));
        }

        let Some(cap) = first else {
            return Ok(());
        };

        let raw_id = cap.get(1).map(|value| value.as_str()).ok_or_else(|| {
            SyncError::Validation(format!(
                "missing packet id capture at {}:{}",
                path.display(),
                node.start_position().row + 1
            ))
        })?;
        let packet_type = cap
            .get(2)
            .map(|value| value.as_str().to_string())
            .ok_or_else(|| {
                SyncError::Validation(format!(
                    "missing packet type capture at {}:{}",
                    path.display(),
                    node.start_position().row + 1
                ))
            })?;
        let packet_id =
            u16::from_str_radix(raw_id.trim_start_matches("0x"), 16).map_err(|err| {
                SyncError::Validation(format!(
                    "invalid packet id {} at {}:{} ({})",
                    raw_id,
                    path.display(),
                    node.start_position().row + 1,
                    err
                ))
            })?;
        if packet_id > 0xFF {
            return Err(SyncError::Validation(format!(
                "packet id out of range {} at {}:{}",
                raw_id,
                path.display(),
                node.start_position().row + 1
            )));
        }

        let matched = cap.get(0).ok_or_else(|| {
            SyncError::Validation(format!(
                "missing full tag capture at {}:{}",
                path.display(),
                node.start_position().row + 1
            ))
        })?;
        tags.push(TagMatch {
            end_byte: node.start_byte() + matched.end(),
            id: packet_id,
            packet_type,
            line: node.start_position().row + 1,
        });
        Ok(())
    })?;

    tags.sort_by_key(|tag| (tag.end_byte, tag.id));
    Ok(tags)
}

fn collect_type_definitions(
    tree: &Tree,
    source: &[u8],
    path: &Path,
) -> Result<Vec<StructDef>, SyncError> {
    let mut out = Vec::new();
    walk_nodes(tree.root_node(), &mut |node| {
        if node.kind() != "type_definition" {
            return Ok(());
        }
        if let Some(struct_def) = parse_type_definition_node(node, source, path)? {
            out.push(struct_def);
        }
        Ok(())
    })?;
    Ok(out)
}

fn parse_type_definition_node(
    node: Node,
    source: &[u8],
    path: &Path,
) -> Result<Option<StructDef>, SyncError> {
    let type_node = match node.child_by_field_name("type") {
        Some(value) => value,
        None => return Ok(None),
    };

    let struct_node = match find_first_kind(type_node, "struct_specifier") {
        Some(value) => value,
        None => return Ok(None),
    };

    let body_node = match struct_node.child_by_field_name("body") {
        Some(value) => value,
        None => return Ok(None),
    };

    let declarators = children_for_field(node, "declarator");
    if declarators.len() != 1 {
        return Err(SyncError::Validation(format!(
            "typedef struct in {}:{} must have exactly one declarator",
            path.display(),
            node.start_position().row + 1
        )));
    }

    let name = extract_declarator_name(declarators[0], source).map_err(|reason| {
        SyncError::Validation(format!(
            "invalid typedef declarator in {}:{} ({})",
            path.display(),
            node.start_position().row + 1,
            reason
        ))
    })?;

    let whole = node
        .utf8_text(source)
        .map_err(|err| {
            SyncError::Validation(format!("invalid utf8 typedef in {}: {err}", path.display()))
        })?
        .to_ascii_lowercase();
    let packed = whole.contains("packed");

    let (fields, byte_size) = parse_struct_fields(body_node, source, packed, path, &name)?;

    Ok(Some(StructDef {
        start_byte: node.start_byte(),
        name,
        packed,
        byte_size,
        fields,
    }))
}

fn parse_struct_fields(
    body_node: Node,
    source: &[u8],
    packed: bool,
    path: &Path,
    struct_name: &str,
) -> Result<(Vec<FieldDef>, usize), SyncError> {
    let mut parsed = Vec::new();
    let mut cursor = body_node.walk();
    for child in body_node.named_children(&mut cursor) {
        if child.kind() != "field_declaration" {
            continue;
        }
        parsed.push(parse_field_declaration(child, source, path, struct_name)?);
    }

    if parsed.is_empty() {
        return Err(SyncError::Validation(format!(
            "struct {} in {} has no supported fields",
            struct_name,
            path.display()
        )));
    }

    let mut fields = Vec::with_capacity(parsed.len());
    let mut offset = 0usize;
    let mut max_align = 1usize;
    for field in parsed {
        let align = if packed { 1 } else { field.size };
        if !packed {
            max_align = max_align.max(align);
            offset = align_up(offset, align);
        }
        fields.push(FieldDef {
            name: field.name,
            c_type: field.c_type,
            offset,
            size: field.size,
        });
        offset += field.size;
    }

    let byte_size = if packed {
        offset
    } else {
        align_up(offset, max_align)
    };

    Ok((fields, byte_size))
}

fn parse_field_declaration(
    node: Node,
    source: &[u8],
    path: &Path,
    struct_name: &str,
) -> Result<FieldSpec, SyncError> {
    if has_kind(node, "bitfield_clause") {
        return Err(SyncError::Validation(format!(
            "unsupported bitfield in {} ({}) at line {}",
            path.display(),
            struct_name,
            node.start_position().row + 1
        )));
    }

    let type_node = node.child_by_field_name("type").ok_or_else(|| {
        SyncError::Validation(format!(
            "field declaration missing type in {} ({}) at line {}",
            path.display(),
            struct_name,
            node.start_position().row + 1
        ))
    })?;

    if matches!(type_node.kind(), "struct_specifier" | "union_specifier") {
        return Err(SyncError::Validation(format!(
            "unsupported nested declaration in {} ({}) at line {}",
            path.display(),
            struct_name,
            node.start_position().row + 1
        )));
    }

    let c_type = normalize_c_type(type_node.utf8_text(source).map_err(|err| {
        SyncError::Validation(format!(
            "invalid utf8 type in {} ({}) at line {} ({})",
            path.display(),
            struct_name,
            node.start_position().row + 1,
            err
        ))
    })?);
    let size = c_type_size(&c_type).ok_or_else(|| {
        SyncError::Validation(format!(
            "unsupported c type in {} ({}) at line {}: {}",
            path.display(),
            struct_name,
            node.start_position().row + 1,
            c_type
        ))
    })?;

    let declarators = children_for_field(node, "declarator");
    if declarators.len() != 1 {
        return Err(SyncError::Validation(format!(
            "unsupported multi declarator in {} ({}) at line {}",
            path.display(),
            struct_name,
            node.start_position().row + 1
        )));
    }

    let declarator = declarators[0];
    if has_kind(declarator, "pointer_declarator")
        || has_kind(declarator, "array_declarator")
        || has_kind(declarator, "function_declarator")
    {
        return Err(SyncError::Validation(format!(
            "unsupported field syntax in {} ({}) at line {}",
            path.display(),
            struct_name,
            node.start_position().row + 1
        )));
    }

    let name_node = find_first_kind(declarator, "field_identifier")
        .or_else(|| find_first_kind(declarator, "identifier"))
        .ok_or_else(|| {
            SyncError::Validation(format!(
                "invalid field declarator in {} ({}) at line {}",
                path.display(),
                struct_name,
                node.start_position().row + 1
            ))
        })?;

    let name = name_node
        .utf8_text(source)
        .map_err(|err| {
            SyncError::Validation(format!(
                "invalid utf8 field name in {}: {err}",
                path.display()
            ))
        })?
        .trim()
        .to_string();

    if !is_identifier(&name) {
        return Err(SyncError::Validation(format!(
            "invalid field name in {} ({}) at line {}: {}",
            path.display(),
            struct_name,
            node.start_position().row + 1,
            name
        )));
    }

    Ok(FieldSpec { name, c_type, size })
}

fn extract_declarator_name(node: Node, source: &[u8]) -> Result<String, &'static str> {
    if has_kind(node, "pointer_declarator")
        || has_kind(node, "array_declarator")
        || has_kind(node, "function_declarator")
    {
        return Err("unsupported typedef declarator");
    }

    let name_node = find_first_kind(node, "type_identifier")
        .or_else(|| find_first_kind(node, "identifier"))
        .ok_or("missing type identifier")?;
    let name = name_node
        .utf8_text(source)
        .map_err(|_| "invalid identifier utf8")?
        .trim()
        .to_string();

    if !is_identifier(&name) {
        return Err("invalid type identifier");
    }

    Ok(name)
}

fn merge_packets(existing: &[PacketDef], discovered: Vec<DiscoveredPacket>) -> Vec<PacketDef> {
    let existing_by_id: HashMap<u16, &PacketDef> =
        existing.iter().map(|packet| (packet.id, packet)).collect();

    let mut merged = Vec::with_capacity(discovered.len());
    for packet in discovered {
        let foxglove = existing_by_id
            .get(&packet.id)
            .and_then(|old| old.foxglove.clone())
            .or_else(|| Some(default_foxglove_topic(&packet.struct_name)));

        merged.push(PacketDef {
            id: packet.id,
            struct_name: packet.struct_name,
            packet_type: packet.packet_type,
            packed: packet.packed,
            byte_size: packet.byte_size,
            source: packet.source,
            fields: packet.fields,
            foxglove,
        });
    }

    merged.sort_by_key(|packet| packet.id);
    merged
}

fn default_foxglove_topic(struct_name: &str) -> BTreeMap<String, toml::Value> {
    let mut map = BTreeMap::new();
    map.insert(
        "topic".to_string(),
        toml::Value::String(format!("/rat/{}", struct_name.to_ascii_lowercase())),
    );
    map
}

fn resolve_scan_root(config_path: &Path, scan_root_override: &Path) -> PathBuf {
    if scan_root_override.is_absolute() {
        return scan_root_override.to_path_buf();
    }

    let base_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    base_dir.join(scan_root_override)
}

fn walk_nodes(
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

fn find_first_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
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

fn has_kind(node: Node<'_>, kind: &str) -> bool {
    find_first_kind(node, kind).is_some()
}

fn children_for_field<'a>(node: Node<'a>, field: &str) -> Vec<Node<'a>> {
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

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(ch) if ch == '_' || ch.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn normalize_c_type(raw: &str) -> String {
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

fn c_type_size(c_type: &str) -> Option<usize> {
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

fn align_up(value: usize, align: usize) -> usize {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_works() {
        assert_eq!(align_up(5, 4), 8);
        assert_eq!(align_up(8, 4), 8);
        assert_eq!(align_up(9, 1), 9);
    }
}
