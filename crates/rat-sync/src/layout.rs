use std::path::Path;

use crate::model::DiscoveredPacket;
use crate::SyncError;

fn compact_ascii_lowercase(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn identifier_tokens_ascii_lowercase(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut token = String::new();
    for ch in value.chars() {
        if ch == '_' || ch.is_ascii_alphanumeric() {
            token.push(ch.to_ascii_lowercase());
            continue;
        }
        if !token.is_empty() {
            out.push(std::mem::take(&mut token));
        }
    }
    if !token.is_empty() {
        out.push(token);
    }
    out
}

fn parse_parenthesized(raw: &str, open_idx: usize) -> Option<(usize, String)> {
    let bytes = raw.as_bytes();
    if bytes.get(open_idx) != Some(&b'(') {
        return None;
    }

    let mut depth = 0usize;
    let mut idx = open_idx;
    while idx < bytes.len() {
        match bytes[idx] {
            b'(' => depth = depth.saturating_add(1),
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let inner = raw[open_idx + 1..idx].to_string();
                    return Some((idx + 1, inner));
                }
            }
            _ => {}
        }
        idx += 1;
    }
    None
}

fn attribute_tokens_ascii_lowercase(compact: &str) -> Vec<String> {
    const ATTRIBUTE_TAG: &str = "__attribute__";

    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(hit) = compact[cursor..].find(ATTRIBUTE_TAG) {
        let start = cursor + hit + ATTRIBUTE_TAG.len();
        let Some((next, args)) = parse_parenthesized(compact, start) else {
            break;
        };
        out.extend(identifier_tokens_ascii_lowercase(&args));
        cursor = next;
    }
    out
}

pub(crate) fn detect_packed_layout(raw_typedef: &str) -> bool {
    let compact = compact_ascii_lowercase(raw_typedef);
    let tokens = identifier_tokens_ascii_lowercase(raw_typedef);
    if tokens
        .iter()
        .any(|token| token == "__packed" || token == "__packed__")
    {
        return true;
    }

    attribute_tokens_ascii_lowercase(&compact)
        .into_iter()
        .any(|token| token == "packed" || token == "__packed" || token == "__packed__")
}

pub(crate) fn validate_layout_modifiers(
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

pub(crate) fn collect_layout_warnings(_discovered: &[DiscoveredPacket]) -> Vec<String> {
    // High-risk non-packed layout drift is blocked in `collect_layout_blockers`.
    // Keep warning channel for future low-risk diagnostics.
    Vec::new()
}

pub(crate) fn collect_layout_blockers(discovered: &[DiscoveredPacket]) -> Vec<String> {
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
