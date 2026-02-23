use std::collections::BTreeSet;

use rat_config::{FieldDef, GeneratedPacketDef};

use crate::model::DiscoveredPacket;
use crate::{SyncError, RAT_ID_MAX, RAT_ID_MIN};

pub(crate) fn allocate_packet_ids(
    discovered: &[DiscoveredPacket],
) -> Result<Vec<GeneratedPacketDef>, SyncError> {
    let mut used_ids = BTreeSet::new();
    let mut assigned = Vec::with_capacity(discovered.len());

    for packet in discovered {
        let id = select_fresh_packet_id(packet.signature_hash, &used_ids);

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

fn compute_signature_hash_parts(
    struct_name: &str,
    packet_type: &str,
    packed: bool,
    byte_size: usize,
    fields: &[FieldDef],
) -> u64 {
    let mut signature = format!("{struct_name}|{packet_type}|{packed}|{byte_size}");
    for field in fields {
        signature.push('|');
        signature.push_str(&field.name);
        signature.push(':');
        signature.push_str(&field.c_type);
        signature.push(':');
        signature.push_str(&field.offset.to_string());
        signature.push(':');
        signature.push_str(&field.size.to_string());
    }
    fnv1a64(signature.as_bytes())
}

pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
