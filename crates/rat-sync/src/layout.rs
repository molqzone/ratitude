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

pub(crate) fn detect_packed_layout(raw_typedef: &str) -> bool {
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
