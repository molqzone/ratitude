use std::fmt::Write as _;

use rat_config::{GeneratedConfig, GeneratedPacketDef};
use rat_protocol::hash_schema_bytes;

pub(crate) fn build_runtime_schema_toml(generated: &GeneratedConfig) -> String {
    build_runtime_schema_toml_from_packets(&generated.packets)
}

pub(crate) fn build_runtime_schema_toml_from_packets(packets: &[GeneratedPacketDef]) -> String {
    let mut out = String::new();
    for packet in packets {
        out.push_str("[[packets]]\n");
        let _ = writeln!(out, "id = {}", packet.id);
        let _ = writeln!(
            out,
            "struct_name = \"{}\"",
            escape_toml_string(&packet.struct_name)
        );
        let _ = writeln!(
            out,
            "type = \"{}\"",
            escape_toml_string(&packet.packet_type)
        );
        let _ = writeln!(out, "packed = {}", packet.packed);
        let _ = writeln!(out, "byte_size = {}", packet.byte_size);
        out.push('\n');

        for field in &packet.fields {
            out.push_str("[[packets.fields]]\n");
            let _ = writeln!(out, "name = \"{}\"", escape_toml_string(&field.name));
            let _ = writeln!(out, "c_type = \"{}\"", escape_toml_string(&field.c_type));
            let _ = writeln!(out, "offset = {}", field.offset);
            let _ = writeln!(out, "size = {}", field.size);
            out.push('\n');
        }
    }
    out
}

pub(crate) fn compute_runtime_schema_hash_from_packets(packets: &[GeneratedPacketDef]) -> u64 {
    let schema_toml = build_runtime_schema_toml_from_packets(packets);
    hash_schema_bytes(schema_toml.as_bytes())
}

fn escape_toml_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
