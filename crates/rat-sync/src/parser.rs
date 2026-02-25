use std::path::Path;
use std::sync::OnceLock;

use rat_config::{FieldDef, PacketType};
use rat_protocol::{c_type_size, normalize_c_type};
use regex::Regex;
use tree_sitter::{Node, Tree};

use crate::ast::{
    align_up, children_for_field, find_first_kind, has_kind, is_identifier, walk_nodes,
};
use crate::layout::{detect_packed_layout, validate_layout_modifiers};
use crate::model::{FieldSpec, StructDef, TagMatch};
use crate::SyncError;

pub(crate) fn collect_comment_tags(
    tree: &Tree,
    source: &[u8],
    path: &Path,
) -> Result<Vec<TagMatch>, SyncError> {
    static STRICT_TAG_RE: OnceLock<Regex> = OnceLock::new();
    let strict_tag_re = STRICT_TAG_RE.get_or_init(|| {
        Regex::new(r"(?s)^\s*(?://+|/\*)\s*@rat(?:\s*,\s*([A-Za-z_][A-Za-z0-9_]*))?\s*(?:\*/)?\s*$")
            .expect("compile @rat tag regex")
    });

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

        let Some(cap) = strict_tag_re.captures(text) else {
            return Err(SyncError::Validation(format!(
                "invalid @rat annotation syntax in {}:{}; use // @rat, <type>",
                path.display(),
                node.start_position().row + 1
            )));
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

        tags.push(TagMatch {
            end_byte: node.end_byte(),
            packet_type,
            line: node.start_position().row + 1,
        });
        Ok(())
    })?;

    tags.sort_by_key(|tag| tag.end_byte);
    Ok(tags)
}

pub(crate) fn normalize_packet_type(raw: &str) -> Result<PacketType, &'static str> {
    if raw.trim().is_empty() {
        return Ok(PacketType::Plot);
    }
    PacketType::parse(raw).ok_or("supported types: plot|quat|image|log")
}

pub(crate) fn collect_type_definitions(
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

pub(crate) fn parse_type_definition_node(
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

pub(crate) fn parse_struct_fields(
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

pub(crate) fn parse_field_declaration(
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

pub(crate) fn extract_declarator_name(node: Node, source: &[u8]) -> Result<String, &'static str> {
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
