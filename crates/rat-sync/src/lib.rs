use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use rat_config::{
    self, load_generated_or_default, save_generated, ConfigError, FieldDef, GeneratedConfig,
    GeneratedMeta, GeneratedPacketDef, RatitudeConfig,
};
use regex::Regex;
use thiserror::Error;
use tree_sitter::{Node, Parser, Tree};
use walkdir::{DirEntry, WalkDir};

const RAT_ID_MIN: u16 = 1;
const RAT_ID_MAX: u16 = 0xFE;

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
    #[error("failed to write generated header {path}: {source}")]
    WriteHeader {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct SyncResult {
    pub config: RatitudeConfig,
    pub changed: bool,
    pub layout_warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct DiscoveredPacket {
    signature_hash: u64,
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
    let (mut cfg, _) = rat_config::load_or_default(config_path)?;

    let discovered = discover_packets(&cfg, scan_root_override)?;
    let layout_blockers = collect_layout_blockers(&discovered);
    if !layout_blockers.is_empty() {
        return Err(SyncError::Validation(format!(
            "layout validation failed:\n- {}",
            layout_blockers.join("\n- ")
        )));
    }
    let layout_warnings = collect_layout_warnings(&discovered);

    let generated_path = cfg.generated_toml_path().to_path_buf();
    let generated_header_path = cfg.generated_header_path().to_path_buf();

    let (old_generated, old_exists) = load_generated_or_default(&generated_path)?;
    let packets = allocate_packet_ids(&discovered, &old_generated.packets)?;
    let fingerprint = compute_fingerprint(&packets);

    let generated = GeneratedConfig {
        meta: GeneratedMeta {
            project: cfg.project.name.clone(),
            fingerprint: format!("0x{:016X}", fingerprint),
        },
        packets,
    };

    let changed = !old_exists || old_generated != generated;
    if changed {
        save_generated(&generated_path, &generated)?;
    }
    if changed || !generated_header_path.exists() {
        write_generated_header(&generated_header_path, &generated)?;
    }

    cfg.packets = generated.to_packet_defs();
    cfg.validate()?;

    Ok(SyncResult {
        config: cfg,
        changed,
        layout_warnings,
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

fn collect_comment_tags(
    tree: &Tree,
    source: &[u8],
    path: &Path,
) -> Result<Vec<TagMatch>, SyncError> {
    let tag_re = Regex::new(r"@rat(?:\s*,\s*([A-Za-z_][A-Za-z0-9_]*))?")
        .map_err(|err| SyncError::Validation(format!("invalid tag regex: {err}")))?;
    let old_id_re = Regex::new(r"@rat\s*:\s*id\s*=")
        .map_err(|err| SyncError::Validation(format!("invalid old-id regex: {err}")))?;
    let old_type_re = Regex::new(r"@rat\s*,\s*type\s*=")
        .map_err(|err| SyncError::Validation(format!("invalid old-type regex: {err}")))?;

    let mut tags = Vec::new();
    walk_nodes(tree.root_node(), &mut |node| {
        if node.kind() != "comment" {
            return Ok(());
        }

        let text = node.utf8_text(source).map_err(|err| {
            SyncError::Validation(format!("invalid utf8 comment in {}: {err}", path.display()))
        })?;

        if !text.contains("@rat") {
            return Ok(());
        }

        if old_id_re.is_match(text) {
            return Err(SyncError::Validation(format!(
                "legacy @rat:id syntax is not supported in {}:{}; use // @rat, <type>",
                path.display(),
                node.start_position().row + 1
            )));
        }
        if old_type_re.is_match(text) {
            return Err(SyncError::Validation(format!(
                "legacy @rat, type= syntax is not supported in {}:{}; use // @rat, <type>",
                path.display(),
                node.start_position().row + 1
            )));
        }

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

        let raw_packet_type = cap.get(1).map(|value| value.as_str()).unwrap_or("plot");
        let packet_type = normalize_packet_type(raw_packet_type).map_err(|reason| {
            SyncError::Validation(format!(
                "invalid @rat type in {}:{} ({})",
                path.display(),
                node.start_position().row + 1,
                reason
            ))
        })?;

        let matched = cap.get(0).ok_or_else(|| {
            SyncError::Validation(format!(
                "missing full tag capture at {}:{}",
                path.display(),
                node.start_position().row + 1
            ))
        })?;

        tags.push(TagMatch {
            end_byte: node.start_byte() + matched.end(),
            packet_type,
            line: node.start_position().row + 1,
        });
        Ok(())
    })?;

    tags.sort_by_key(|tag| tag.end_byte);
    Ok(tags)
}

fn normalize_packet_type(raw: &str) -> Result<String, &'static str> {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Ok("plot".to_string());
    }

    match normalized.as_str() {
        "plot" => Ok("plot".to_string()),
        "quat" => Ok("quat".to_string()),
        "image" => Ok("image".to_string()),
        "log" => Ok("log".to_string()),
        _ => Err("supported types: plot|quat|image|log"),
    }
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

    let line = node.start_position().row + 1;
    let whole = node.utf8_text(source).map_err(|err| {
        SyncError::Validation(format!("invalid utf8 typedef in {}: {err}", path.display()))
    })?;
    let packed = detect_packed_layout(whole);
    validate_layout_modifiers(whole, path, line, &name)?;

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

fn compact_ascii_lowercase(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn is_identifier_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

fn contains_identifier_token(haystack: &str, token: &str) -> bool {
    let haystack_bytes = haystack.as_bytes();
    let token_bytes = token.as_bytes();
    if token_bytes.is_empty() || token_bytes.len() > haystack_bytes.len() {
        return false;
    }

    for idx in 0..=(haystack_bytes.len() - token_bytes.len()) {
        if &haystack_bytes[idx..idx + token_bytes.len()] != token_bytes {
            continue;
        }

        let left_ok = idx == 0 || !is_identifier_byte(haystack_bytes[idx - 1]);
        let right_idx = idx + token_bytes.len();
        let right_ok =
            right_idx == haystack_bytes.len() || !is_identifier_byte(haystack_bytes[right_idx]);
        if left_ok && right_ok {
            return true;
        }
    }

    false
}

fn detect_packed_layout(raw_typedef: &str) -> bool {
    let lowered = raw_typedef.to_ascii_lowercase();
    let compact = compact_ascii_lowercase(raw_typedef);
    if compact.contains("__attribute__((packed") || compact.contains("__attribute__((__packed__") {
        return true;
    }
    contains_identifier_token(&lowered, "__packed")
        || contains_identifier_token(&lowered, "__packed__")
        || compact.contains("__packedstruct")
        || compact.contains("__packed__struct")
        || compact.contains("struct__packed")
        || compact.contains("struct__packed__")
        || contains_identifier_token(&compact, "__packed")
        || contains_identifier_token(&compact, "__packed__")
}

fn validate_layout_modifiers(
    raw_typedef: &str,
    path: &Path,
    line: usize,
    struct_name: &str,
) -> Result<(), SyncError> {
    let compact = compact_ascii_lowercase(raw_typedef);
    let has_custom_alignment = compact.contains("aligned(")
        || compact.contains("__align(")
        || compact.contains("alignas(")
        || compact.contains("#pragmapack")
        || compact.contains("pragmapack(");
    if has_custom_alignment {
        return Err(SyncError::Validation(format!(
            "unsupported layout modifier in {} ({}) at line {}: aligned/pragma-pack is not supported for @rat structs; use natural layout or packed only",
            path.display(),
            struct_name,
            line
        )));
    }
    Ok(())
}

fn collect_layout_warnings(_discovered: &[DiscoveredPacket]) -> Vec<String> {
    // High-risk non-packed layout drift is blocked in `collect_layout_blockers`.
    // Keep warning channel for future low-risk diagnostics.
    Vec::new()
}

fn collect_layout_blockers(discovered: &[DiscoveredPacket]) -> Vec<String> {
    let mut blockers = Vec::new();
    for packet in discovered {
        if packet.packed {
            continue;
        }

        let reasons = collect_layout_risk_reasons(packet);
        if reasons.is_empty() {
            continue;
        }

        blockers.push(format!(
            "packet {} ({}) uses non-packed layout and {}; declare it packed or remove ABI-sensitive fields",
            packet.struct_name,
            packet.source,
            reasons.join(" + ")
        ));
    }
    blockers
}

fn collect_layout_risk_reasons(packet: &DiscoveredPacket) -> Vec<&'static str> {
    let mut reasons: Vec<&'static str> = Vec::new();
    let has_wide_fields = packet.fields.iter().any(|field| field.size >= 8);
    if has_wide_fields {
        reasons.push("contains >=8-byte fields");
    }

    let mut expected_end = 0usize;
    let mut has_internal_padding = false;
    let mut payload_sum = 0usize;
    for field in &packet.fields {
        if field.offset > expected_end {
            has_internal_padding = true;
        }
        let field_end = field.offset.saturating_add(field.size);
        expected_end = expected_end.max(field_end);
        payload_sum = payload_sum.saturating_add(field.size);
    }
    let has_tail_padding = packet.byte_size > payload_sum;
    if has_internal_padding || has_tail_padding {
        reasons.push("includes compiler-dependent padding");
    }
    reasons
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

fn allocate_packet_ids(
    discovered: &[DiscoveredPacket],
    previous: &[GeneratedPacketDef],
) -> Result<Vec<GeneratedPacketDef>, SyncError> {
    let mut previous_by_signature: HashMap<u64, u16> = HashMap::new();
    for packet in previous {
        if let Some(old_signature) = parse_signature_hex(&packet.signature_hash) {
            previous_by_signature
                .entry(old_signature)
                .or_insert(packet.id);
        }
        let semantic_signature = compute_generated_packet_signature_hash(packet);
        previous_by_signature
            .entry(semantic_signature)
            .or_insert(packet.id);
    }

    let mut used_ids = BTreeSet::new();
    let mut assigned = Vec::with_capacity(discovered.len());

    for packet in discovered {
        let mut chosen = previous_by_signature
            .get(&packet.signature_hash)
            .copied()
            .filter(|id| (RAT_ID_MIN..=RAT_ID_MAX).contains(id) && !used_ids.contains(id));

        if chosen.is_none() {
            chosen = Some(select_fresh_packet_id(packet.signature_hash, &used_ids));
        }

        let id = chosen.ok_or_else(|| {
            SyncError::Validation(format!(
                "failed to assign packet id for {} ({})",
                packet.struct_name, packet.source
            ))
        })?;

        if !used_ids.insert(id) {
            return Err(SyncError::Validation(format!(
                "duplicate assigned packet id 0x{:02X}",
                id
            )));
        }

        assigned.push(GeneratedPacketDef {
            id,
            signature_hash: format!("0x{:016X}", packet.signature_hash),
            struct_name: packet.struct_name.clone(),
            packet_type: packet.packet_type.clone(),
            packed: packet.packed,
            byte_size: packet.byte_size,
            source: packet.source.clone(),
            fields: packet.fields.clone(),
        });
    }

    Ok(assigned)
}

fn select_fresh_packet_id(signature_hash: u64, used_ids: &BTreeSet<u16>) -> u16 {
    let mut candidate = ((signature_hash % 254) as u16) + 1;
    while used_ids.contains(&candidate) {
        candidate = if candidate >= RAT_ID_MAX {
            RAT_ID_MIN
        } else {
            candidate + 1
        };
    }
    candidate
}

fn compute_signature_hash(packet: &DiscoveredPacket) -> u64 {
    compute_signature_hash_parts(
        &packet.struct_name,
        &packet.packet_type,
        packet.packed,
        packet.byte_size,
        &packet.fields,
    )
}

fn compute_generated_packet_signature_hash(packet: &GeneratedPacketDef) -> u64 {
    compute_signature_hash_parts(
        &packet.struct_name,
        &packet.packet_type,
        packet.packed,
        packet.byte_size,
        &packet.fields,
    )
}

fn compute_signature_hash_parts(
    struct_name: &str,
    packet_type: &str,
    packed: bool,
    byte_size: usize,
    fields: &[FieldDef],
) -> u64 {
    let mut signature = String::new();
    let _ = write!(
        signature,
        "{}|{}|{}|{}",
        struct_name, packet_type, packed, byte_size
    );
    for field in fields {
        let _ = write!(
            signature,
            "|{}:{}:{}:{}",
            field.name, field.c_type, field.offset, field.size
        );
    }
    fnv1a64(signature.as_bytes())
}

fn compute_fingerprint(packets: &[GeneratedPacketDef]) -> u64 {
    let mut ordered = packets.to_vec();
    ordered.sort_by_key(|packet| packet.id);

    let mut input = String::new();
    for packet in &ordered {
        let _ = write!(
            input,
            "{:02X}|{}|{}|{}|{};",
            packet.id,
            packet.signature_hash,
            packet.struct_name,
            packet.packet_type,
            packet.byte_size
        );
    }
    fnv1a64(input.as_bytes())
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn parse_signature_hex(raw: &str) -> Option<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    u64::from_str_radix(hex, 16).ok()
}

fn parse_fingerprint_hex(raw: &str) -> Option<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    u64::from_str_radix(hex, 16).ok()
}

fn write_generated_header(path: &Path, generated: &GeneratedConfig) -> Result<(), SyncError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| SyncError::WriteHeader {
            path: path.to_path_buf(),
            source,
        })?;
    }

    let mut out = String::new();
    out.push_str("#ifndef RAT_GEN_H\n");
    out.push_str("#define RAT_GEN_H\n\n");
    out.push_str("/* This file is generated by rat-sync. */\n");
    let header_fingerprint = parse_fingerprint_hex(&generated.meta.fingerprint).unwrap_or(0);
    let _ = writeln!(
        out,
        "#define RAT_GEN_FINGERPRINT 0x{:016X}ULL",
        header_fingerprint
    );
    let _ = writeln!(
        out,
        "#define RAT_GEN_PACKET_COUNT {}u",
        generated.packets.len()
    );
    out.push('\n');

    let mut name_counts: HashMap<String, usize> = HashMap::new();
    for packet in &generated.packets {
        let base = macroize_struct_name(&packet.struct_name);
        let count = name_counts
            .entry(base.clone())
            .and_modify(|value| *value += 1)
            .or_insert(1);
        let macro_name = if *count == 1 {
            format!("RAT_ID_{base}")
        } else {
            format!("RAT_ID_{}_{}", base, count)
        };
        let _ = writeln!(out, "#define {macro_name} 0x{:02X}u", packet.id);
    }

    out.push_str("\n#endif  /* RAT_GEN_H */\n");

    fs::write(path, out).map_err(|source| SyncError::WriteHeader {
        path: path.to_path_buf(),
        source,
    })
}

fn macroize_struct_name(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_uppercase());
        } else {
            out.push('_');
        }
    }

    while out.contains("__") {
        out = out.replace("__", "_");
    }

    out.trim_matches('_').to_string()
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
    use std::fmt::Write as _;
    use std::fs;
    use std::path::Path;

    use super::*;

    #[test]
    fn packet_type_normalization_supports_default_only() {
        assert_eq!(normalize_packet_type("plot").expect("plot"), "plot");
        assert_eq!(normalize_packet_type("quat").expect("quat"), "quat");
        assert!(normalize_packet_type("pose").is_err());
        assert_eq!(normalize_packet_type("").expect("default"), "plot");
        assert!(normalize_packet_type("json").is_err());
    }

    #[test]
    fn alignment_works() {
        assert_eq!(align_up(5, 4), 8);
        assert_eq!(align_up(8, 4), 8);
        assert_eq!(align_up(9, 1), 9);
    }

    #[test]
    fn id_allocator_avoids_reserved_ids() {
        let used = BTreeSet::from([1_u16, 2, 3, 0xFE]);
        let id = select_fresh_packet_id(0, &used);
        assert!((RAT_ID_MIN..=RAT_ID_MAX).contains(&id));
        assert!(!used.contains(&id));
    }

    #[test]
    fn fnv_hash_is_stable() {
        assert_eq!(fnv1a64(b"ratitude"), 0x68EDD638D6E4A56B);
    }

    fn sample_fields() -> Vec<FieldDef> {
        vec![
            FieldDef {
                name: "value".to_string(),
                c_type: "int32_t".to_string(),
                offset: 0,
                size: 4,
            },
            FieldDef {
                name: "tick".to_string(),
                c_type: "uint32_t".to_string(),
                offset: 4,
                size: 4,
            },
        ]
    }

    fn legacy_signature_hash(packet: &DiscoveredPacket) -> u64 {
        let mut signature = String::new();
        let _ = write!(
            signature,
            "{}|{}|{}|{}|{}",
            packet.struct_name, packet.packet_type, packet.packed, packet.byte_size, packet.source
        );
        for field in &packet.fields {
            let _ = write!(
                signature,
                "|{}:{}:{}:{}",
                field.name, field.c_type, field.offset, field.size
            );
        }
        fnv1a64(signature.as_bytes())
    }

    #[test]
    fn signature_hash_ignores_source_path() {
        let base = DiscoveredPacket {
            signature_hash: 0,
            struct_name: "RatSample".to_string(),
            packet_type: "plot".to_string(),
            packed: false,
            byte_size: 8,
            source: "src/a.c".to_string(),
            fields: sample_fields(),
        };
        let moved = DiscoveredPacket {
            source: "src/sub/main.c".to_string(),
            ..base.clone()
        };

        assert_eq!(
            compute_signature_hash(&base),
            compute_signature_hash(&moved),
            "signature should depend on packet semantics, not source path"
        );
    }

    #[test]
    fn allocator_reuses_previous_id_from_legacy_source_based_signature() {
        let mut discovered = DiscoveredPacket {
            signature_hash: 0,
            struct_name: "RatSample".to_string(),
            packet_type: "plot".to_string(),
            packed: false,
            byte_size: 8,
            source: "new_path/main.c".to_string(),
            fields: sample_fields(),
        };
        discovered.signature_hash = compute_signature_hash(&discovered);

        let legacy_packet = DiscoveredPacket {
            source: "old_path/main.c".to_string(),
            ..discovered.clone()
        };
        let legacy_signature = legacy_signature_hash(&legacy_packet);
        let previous = vec![GeneratedPacketDef {
            id: 0x2A,
            signature_hash: format!("0x{:016X}", legacy_signature),
            struct_name: discovered.struct_name.clone(),
            packet_type: discovered.packet_type.clone(),
            packed: discovered.packed,
            byte_size: discovered.byte_size,
            source: legacy_packet.source.clone(),
            fields: discovered.fields.clone(),
        }];

        let assigned = allocate_packet_ids(&[discovered], &previous).expect("allocate ids");
        assert_eq!(assigned.len(), 1);
        assert_eq!(
            assigned[0].id, 0x2A,
            "legacy signature should map to semantic signature and preserve packet id"
        );
    }

    #[test]
    fn packed_detection_is_explicit() {
        let plain = "typedef struct { int32_t packed; } Foo;";
        assert!(!detect_packed_layout(plain));

        let packed_attr = "typedef struct __attribute__((packed)) { int32_t value; } Foo;";
        assert!(detect_packed_layout(packed_attr));

        let packed_keyword = "typedef __packed struct { int32_t value; } Foo;";
        assert!(detect_packed_layout(packed_keyword));
    }

    fn write_test_config(path: &Path, scan_root: &str) {
        let mut cfg = rat_config::RatitudeConfig::default();
        cfg.project.name = "sync_test".to_string();
        cfg.project.scan_root = scan_root.to_string();
        cfg.generation.out_dir = ".".to_string();
        cfg.generation.toml_name = "rat_gen.toml".to_string();
        cfg.generation.header_name = "rat_gen.h".to_string();
        cfg.save(path).expect("save config");
    }

    #[test]
    fn sync_packets_accepts_new_tag_syntax_and_generates_outputs() {
        let temp = std::env::temp_dir().join(format!("rat_sync_new_syntax_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(temp.join("src")).expect("mkdir");

        let config_path = temp.join("rat.toml");
        write_test_config(&config_path, "src");

        let source = r#"
// @rat, plot
typedef struct {
  int32_t value;
  uint32_t tick;
} RatSample;
"#;
        fs::write(temp.join("src").join("main.c"), source).expect("write source");

        let result = sync_packets(&config_path, None).expect("sync should pass");
        assert_eq!(result.config.packets.len(), 1);
        assert_eq!(result.config.packets[0].packet_type, "plot");
        assert!((RAT_ID_MIN..=RAT_ID_MAX).contains(&result.config.packets[0].id));

        assert!(temp.join("rat_gen.toml").exists());
        assert!(temp.join("rat_gen.h").exists());

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn sync_packets_rejects_legacy_tag_syntax() {
        let temp = std::env::temp_dir().join(format!("rat_sync_legacy_tag_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(temp.join("src")).expect("mkdir");

        let config_path = temp.join("rat.toml");
        write_test_config(&config_path, "src");

        let source = r#"
// @rat:id=0x01, type=plot
typedef struct {
  int32_t value;
} RatSample;
"#;
        fs::write(temp.join("src").join("main.c"), source).expect("write source");

        let err = sync_packets(&config_path, None).expect_err("legacy syntax should fail");
        assert!(err.to_string().contains("legacy @rat:id syntax"));

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn sync_packets_rejects_non_packed_padding_layout() {
        let temp =
            std::env::temp_dir().join(format!("rat_sync_layout_block_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(temp.join("src")).expect("mkdir");

        let config_path = temp.join("rat.toml");
        write_test_config(&config_path, "src");

        let source = r#"
// @rat, plot
typedef struct {
  uint8_t a;
  uint32_t b;
} RatPadded;
"#;
        fs::write(temp.join("src").join("main.c"), source).expect("write source");

        let err = sync_packets(&config_path, None).expect_err("sync should fail");
        assert!(
            err.to_string().contains("compiler-dependent padding"),
            "expected padding blocker, got {err:#}"
        );
        assert!(
            err.to_string().contains("layout validation failed"),
            "expected validation summary, got {err:#}"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn sync_packets_rejects_non_packed_wide_field_layout() {
        let temp =
            std::env::temp_dir().join(format!("rat_sync_layout_wide_block_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(temp.join("src")).expect("mkdir");

        let config_path = temp.join("rat.toml");
        write_test_config(&config_path, "src");

        let source = r#"
// @rat, plot
typedef struct {
  uint64_t tick;
} RatWide;
"#;
        fs::write(temp.join("src").join("main.c"), source).expect("write source");

        let err = sync_packets(&config_path, None).expect_err("sync should fail");
        assert!(
            err.to_string().contains("contains >=8-byte fields"),
            "expected wide field blocker, got {err:#}"
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn sync_packets_accepts_packed_layout_with_wide_fields() {
        let temp =
            std::env::temp_dir().join(format!("rat_sync_layout_packed_ok_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(temp.join("src")).expect("mkdir");

        let config_path = temp.join("rat.toml");
        write_test_config(&config_path, "src");

        let source = r#"
// @rat, plot
typedef struct __attribute__((packed)) {
  uint8_t a;
  uint64_t tick;
} RatPacked;
"#;
        fs::write(temp.join("src").join("main.c"), source).expect("write source");

        let result = sync_packets(&config_path, None).expect("packed layout should pass");
        assert!(
            result.layout_warnings.is_empty(),
            "packed layout should not produce blockers/warnings: {:?}",
            result.layout_warnings
        );

        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn sync_packets_rejects_aligned_layout_modifier() {
        let temp =
            std::env::temp_dir().join(format!("rat_sync_layout_reject_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(temp.join("src")).expect("mkdir");

        let config_path = temp.join("rat.toml");
        write_test_config(&config_path, "src");

        let source = r#"
// @rat, plot
typedef struct __attribute__((aligned(8))) {
  int32_t value;
} RatAligned;
"#;
        fs::write(temp.join("src").join("main.c"), source).expect("write source");

        let err = sync_packets(&config_path, None).expect_err("aligned modifier should fail");
        assert!(err.to_string().contains("unsupported layout modifier"));

        let _ = fs::remove_dir_all(&temp);
    }
}
