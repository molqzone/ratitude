use std::collections::HashSet;
use std::path::Path;

use crate::ids::compute_signature_hash;
use crate::model::{DiscoveredPacket, ParsedTaggedFile};
use crate::SyncError;

pub(crate) fn assemble_discovered_packets(
    path: &Path,
    scan_root: &Path,
    parsed: ParsedTaggedFile,
) -> Result<Vec<DiscoveredPacket>, SyncError> {
    let mut out = Vec::with_capacity(parsed.tags.len());
    let mut used_structs = HashSet::new();

    for tag in parsed.tags {
        let mut matched = None;
        for (idx, st) in parsed.structs.iter().enumerate() {
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rat_config::{FieldDef, PacketType};

    use crate::model::{ParsedTaggedFile, StructDef, TagMatch};

    use super::*;

    #[test]
    fn assemble_discovered_packets_requires_following_struct() {
        let parsed = ParsedTaggedFile {
            tags: vec![TagMatch {
                end_byte: 100,
                packet_type: PacketType::Plot,
                line: 10,
            }],
            structs: vec![StructDef {
                start_byte: 50,
                name: "EarlierStruct".to_string(),
                packed: true,
                byte_size: 4,
                fields: vec![FieldDef {
                    name: "value".to_string(),
                    c_type: "int32_t".to_string(),
                    offset: 0,
                    size: 4,
                }],
            }],
        };

        let err = assemble_discovered_packets(Path::new("src/main.c"), Path::new("src"), parsed)
            .expect_err("must fail");
        assert!(err.to_string().contains("has no following typedef struct"));
    }
}
