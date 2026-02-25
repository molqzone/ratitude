mod c_types;
mod context;
mod types;
mod wire;

pub use c_types::{c_type_size, normalize_c_type};
pub use context::{parse_text, ProtocolContext};
pub use types::{
    DynamicFieldDef, DynamicPacketDef, PacketData, PacketType, ProtocolError, RatPacket,
};
pub use wire::{cobs_decode, hash_schema_bytes};

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn parse_text_stops_at_null() {
        assert_eq!(parse_text(b"abc\0def"), "abc");
    }

    #[test]
    fn packet_type_parse_is_case_insensitive() {
        assert_eq!(PacketType::parse("plot"), Some(PacketType::Plot));
        assert_eq!(PacketType::parse(" QuAt "), Some(PacketType::Quat));
        assert_eq!(PacketType::parse("unknown"), None);
    }

    #[test]
    fn cobs_simple() {
        assert_eq!(
            cobs_decode(&[0x03, 0x11, 0x22]).expect("decode"),
            vec![0x11, 0x22]
        );
    }

    #[test]
    fn text_id_isolation_between_contexts() {
        let mut ctx_a = ProtocolContext::new();
        ctx_a.set_text_packet_id(0x01);
        let mut ctx_b = ProtocolContext::new();
        ctx_b.set_text_packet_id(0x02);

        let data_a = ctx_a.parse_packet(0x01, b"abc").expect("ctx_a parse");
        let data_b = ctx_b.parse_packet(0x01, b"abc");

        assert!(matches!(data_a, PacketData::Text(_)));
        assert!(matches!(data_b, Err(ProtocolError::UnknownPacketId(0x01))));
    }

    #[test]
    fn dynamic_registry_isolation_between_contexts() {
        let mut ctx_a = ProtocolContext::new();
        let ctx_b = ProtocolContext::new();
        ctx_a
            .register_dynamic(DynamicPacketDef {
                id: 0x20,
                struct_name: "Demo".to_string(),
                packed: true,
                byte_size: 4,
                fields: vec![DynamicFieldDef {
                    name: "value".to_string(),
                    c_type: "int32_t".to_string(),
                    offset: 0,
                    size: 4,
                }],
            })
            .expect("register dynamic");

        let payload = 42_i32.to_le_bytes();

        let data_a = ctx_a.parse_packet(0x20, &payload).expect("ctx_a parse");
        let data_b = ctx_b.parse_packet(0x20, &payload);

        match data_a {
            PacketData::Dynamic(map) => {
                assert_eq!(map.get("value").and_then(Value::as_i64), Some(42));
            }
            other => panic!("unexpected packet kind: {other:?}"),
        }
        assert!(matches!(data_b, Err(ProtocolError::UnknownPacketId(0x20))));
    }

    #[test]
    fn unknown_packet_id_returns_error() {
        let ctx = ProtocolContext::new();
        let err = ctx
            .parse_packet(0x42, &[0x01, 0x02])
            .expect_err("should fail");
        assert!(matches!(err, ProtocolError::UnknownPacketId(0x42)));
    }

    #[test]
    fn schema_hash_is_stable() {
        assert_eq!(
            hash_schema_bytes(b"abc"),
            0xE71FA2190541574B,
            "schema hash must stay stable across crates"
        );
    }

    #[test]
    fn normalize_c_type_ignores_qualifier_order_and_whitespace() {
        assert_eq!(normalize_c_type("volatile const uint32_t"), "uint32_t");
        assert_eq!(
            normalize_c_type("  const   volatile   int16_t  "),
            "int16_t"
        );
        assert_eq!(normalize_c_type("\tconst\tfloat\n"), "float");
    }
}
