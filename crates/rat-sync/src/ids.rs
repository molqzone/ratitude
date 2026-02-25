use std::collections::{BTreeSet, HashMap};

use rat_config::{FieldDef, PacketType};

use crate::generated::GeneratedPacketDef;
use crate::model::DiscoveredPacket;
use crate::{SyncError, RAT_ID_MAX, RAT_ID_MIN};

pub(crate) fn allocate_packet_ids(
    discovered: &[DiscoveredPacket],
    previous_packets: &[GeneratedPacketDef],
) -> Result<Vec<GeneratedPacketDef>, SyncError> {
    let previous_ids = signature_to_id_map(previous_packets);
    let mut used_ids = BTreeSet::new();
    let mut assigned = Vec::with_capacity(discovered.len());

    for packet in discovered {
        let preferred = previous_ids
            .get(&signature_key_for_discovered(packet))
            .copied()
            .filter(|id| (RAT_ID_MIN..=RAT_ID_MAX).contains(id))
            .filter(|id| !used_ids.contains(id));
        let id =
            preferred.unwrap_or_else(|| select_fresh_packet_id(packet.signature_hash, &used_ids));

        if !used_ids.insert(id) {
            return Err(SyncError::Validation(format!(
                "duplicate assigned packet id 0x{:02X}",
                id
            )));
        }

        assigned.push(GeneratedPacketDef {
            id,
            struct_name: packet.struct_name.clone(),
            packet_type: packet.packet_type,
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
    let signature = signature_key_parts(
        &packet.struct_name,
        &packet.packet_type,
        packet.packed,
        packet.byte_size,
        &packet.fields,
    );
    fnv1a64(signature.as_bytes())
}

fn signature_key_for_discovered(packet: &DiscoveredPacket) -> String {
    signature_key_parts(
        &packet.struct_name,
        &packet.packet_type,
        packet.packed,
        packet.byte_size,
        &packet.fields,
    )
}

fn signature_key_for_generated(packet: &GeneratedPacketDef) -> String {
    signature_key_parts(
        &packet.struct_name,
        &packet.packet_type,
        packet.packed,
        packet.byte_size,
        &packet.fields,
    )
}

fn signature_to_id_map(previous_packets: &[GeneratedPacketDef]) -> HashMap<String, u16> {
    let mut out = HashMap::new();
    for packet in previous_packets {
        out.insert(signature_key_for_generated(packet), packet.id);
    }
    out
}

fn signature_key_parts(
    struct_name: &str,
    packet_type: &PacketType,
    packed: bool,
    byte_size: usize,
    fields: &[FieldDef],
) -> String {
    let mut signature = format!(
        "{struct_name}|{}|{packed}|{byte_size}",
        packet_type.as_str()
    );
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
    signature
}

pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
