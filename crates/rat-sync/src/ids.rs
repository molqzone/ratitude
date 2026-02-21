use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

use rat_config::{FieldDef, GeneratedPacketDef};

use crate::model::DiscoveredPacket;
use crate::{SyncError, RAT_ID_MAX, RAT_ID_MIN};

pub(crate) fn allocate_packet_ids(
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

pub(crate) fn select_fresh_packet_id(signature_hash: u64, used_ids: &BTreeSet<u16>) -> u16 {
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

pub(crate) fn compute_signature_hash(packet: &DiscoveredPacket) -> u64 {
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

pub(crate) fn compute_fingerprint(packets: &[GeneratedPacketDef]) -> u64 {
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

pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub(crate) fn parse_signature_hex(raw: &str) -> Option<u64> {
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

pub(crate) fn parse_fingerprint_hex(raw: &str) -> Option<u64> {
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
