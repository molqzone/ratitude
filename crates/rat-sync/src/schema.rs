use rat_config::{GeneratedConfig, GeneratedPacketDef};
use rat_protocol::hash_schema_bytes;

pub(crate) fn build_runtime_schema_toml(generated: &GeneratedConfig) -> String {
    build_runtime_schema_toml_from_packets(&generated.packets)
}

pub(crate) fn build_runtime_schema_toml_from_packets(packets: &[GeneratedPacketDef]) -> String {
    let mut out = String::new();
    for packet in packets {
        out.push_str("[[packets]]\n");
        out.push_str(&format!("id = {}\n", packet.id));
        out.push_str(&format!(
            "struct_name = \"{}\"\n",
            escape_toml_string(&packet.struct_name)
        ));
        out.push_str(&format!(
            "type = \"{}\"\n",
            escape_toml_string(&packet.packet_type)
        ));
        out.push_str(&format!("packed = {}\n", packet.packed));
        out.push_str(&format!("byte_size = {}\n", packet.byte_size));
        out.push('\n');

        for field in &packet.fields {
            out.push_str("[[packets.fields]]\n");
            out.push_str(&format!("name = \"{}\"\n", escape_toml_string(&field.name)));
            out.push_str(&format!(
                "c_type = \"{}\"\n",
                escape_toml_string(&field.c_type)
            ));
            out.push_str(&format!("offset = {}\n", field.offset));
            out.push_str(&format!("size = {}\n", field.size));
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
