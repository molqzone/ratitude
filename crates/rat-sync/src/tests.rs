use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use rat_config::{FieldDef, PacketType};

use crate::ast::align_up;
use crate::ids::{compute_signature_hash, fnv1a64, select_fresh_packet_id};
use crate::layout::detect_packed_layout;
use crate::model::{DiscoveredPacket, SyncPipelineInput};
use crate::parser::normalize_packet_type;
use crate::pipeline::run_sync_pipeline;
use crate::{sync_packets_fs, RAT_ID_MAX, RAT_ID_MIN};

#[test]
fn packet_type_normalization_supports_default_only() {
    assert_eq!(
        normalize_packet_type("plot").expect("plot"),
        PacketType::Plot
    );
    assert_eq!(
        normalize_packet_type("quat").expect("quat"),
        PacketType::Quat
    );
    assert!(normalize_packet_type("pose").is_err());
    assert_eq!(
        normalize_packet_type("").expect("default"),
        PacketType::Plot
    );
    assert!(normalize_packet_type("json").is_err());
}

#[test]
fn alignment_works() {
    assert_eq!(align_up(5, 4), 8);
    assert_eq!(align_up(8, 4), 8);
    assert_eq!(align_up(9, 1), 9);
}

#[test]
fn id_allocator_avoids_reserved_ids() {
    let used = BTreeSet::from([1_u16, 2, 3, 0xFE]);
    let id = select_fresh_packet_id(0, &used);
    assert!((RAT_ID_MIN..=RAT_ID_MAX).contains(&id));
    assert!(!used.contains(&id));
}

#[test]
fn fnv_hash_is_stable() {
    assert_eq!(fnv1a64(b"ratitude"), 0x68EDD638D6E4A56B);
}

fn sample_fields() -> Vec<FieldDef> {
    vec![
        FieldDef {
            name: "value".to_string(),
            c_type: "int32_t".to_string(),
            offset: 0,
            size: 4,
        },
        FieldDef {
            name: "tick".to_string(),
            c_type: "uint32_t".to_string(),
            offset: 4,
            size: 4,
        },
    ]
}

#[test]
fn signature_hash_ignores_source_path() {
    let base = DiscoveredPacket {
        signature_hash: 0,
        struct_name: "RatSample".to_string(),
        packet_type: PacketType::Plot,
        packed: false,
        byte_size: 8,
        source: "src/a.c".to_string(),
        fields: sample_fields(),
    };
    let moved = DiscoveredPacket {
        source: "src/sub/main.c".to_string(),
        ..base.clone()
    };

    assert_eq!(
        compute_signature_hash(&base),
        compute_signature_hash(&moved),
        "signature should depend on packet semantics, not source path"
    );
}

#[test]
fn run_sync_pipeline_is_deterministic_for_identical_input() {
    let mut discovered = DiscoveredPacket {
        signature_hash: 0,
        struct_name: "RatSample".to_string(),
        packet_type: PacketType::Plot,
        packed: false,
        byte_size: 8,
        source: "src/main.c".to_string(),
        fields: sample_fields(),
    };
    discovered.signature_hash = compute_signature_hash(&discovered);

    let first = run_sync_pipeline(SyncPipelineInput {
        project_name: "sync_test".to_string(),
        discovered_packets: vec![discovered.clone()],
    })
    .expect("first pipeline run");
    let second = run_sync_pipeline(SyncPipelineInput {
        project_name: "sync_test".to_string(),
        discovered_packets: vec![discovered],
    })
    .expect("second pipeline run");

    assert_eq!(
        first.generated, second.generated,
        "identical input should produce identical generated output"
    );
}

#[test]
fn run_sync_pipeline_blocks_non_packed_padding_layout_without_filesystem() {
    let discovered = DiscoveredPacket {
        signature_hash: 0,
        struct_name: "RatPadded".to_string(),
        packet_type: PacketType::Plot,
        packed: false,
        byte_size: 8,
        source: "src/main.c".to_string(),
        fields: vec![
            FieldDef {
                name: "a".to_string(),
                c_type: "uint8_t".to_string(),
                offset: 0,
                size: 1,
            },
            FieldDef {
                name: "b".to_string(),
                c_type: "uint32_t".to_string(),
                offset: 4,
                size: 4,
            },
        ],
    };

    let err = run_sync_pipeline(SyncPipelineInput {
        project_name: "sync_test".to_string(),
        discovered_packets: vec![discovered],
    })
    .expect_err("pipeline should reject non-packed padding");
    assert!(err.to_string().contains("layout validation failed"));
}

#[test]
fn packed_detection_is_explicit() {
    let plain = "typedef struct { int32_t packed; } Foo;";
    assert!(!detect_packed_layout(plain));

    let packed_attr = "typedef struct __attribute__((packed)) { int32_t value; } Foo;";
    assert!(detect_packed_layout(packed_attr));

    let packed_keyword = "typedef __packed struct { int32_t value; } Foo;";
    assert!(detect_packed_layout(packed_keyword));
}

fn write_test_config(path: &Path, scan_root: &str) {
    let mut cfg = rat_config::RatitudeConfig::default();
    cfg.project.name = "sync_test".to_string();
    cfg.project.scan_root = scan_root.to_string();
    cfg.generation.out_dir = ".".to_string();
    cfg.generation.header_name = "rat_gen.h".to_string();
    rat_config::ConfigStore::new(path)
        .save(&cfg)
        .expect("save config");
}

#[test]
fn sync_packets_fs_accepts_new_tag_syntax_and_generates_outputs() {
    let temp = std::env::temp_dir().join(format!("rat_sync_new_syntax_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(temp.join("src")).expect("mkdir");

    let config_path = temp.join("rat.toml");
    write_test_config(&config_path, "src");

    let source = r#"
// @rat, plot
typedef struct {
  int32_t value;
  uint32_t tick;
} RatSample;
"#;
    fs::write(temp.join("src").join("main.c"), source).expect("write source");

    let result = sync_packets_fs(&config_path, None).expect("sync should pass");
    assert_eq!(result.packet_defs.len(), 1);
    assert_eq!(result.packet_defs[0].packet_type, PacketType::Plot);
    assert!((RAT_ID_MIN..=RAT_ID_MAX).contains(&result.packet_defs[0].id));

    assert!(temp.join("rat_gen.h").exists());

    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn sync_packets_fs_rejects_invalid_tag_syntax() {
    let temp = std::env::temp_dir().join(format!("rat_sync_invalid_tag_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(temp.join("src")).expect("mkdir");

    let config_path = temp.join("rat.toml");
    write_test_config(&config_path, "src");

    let source = r#"
// @rat:id=0x01, type=plot
typedef struct {
  int32_t value;
} RatSample;
"#;
    fs::write(temp.join("src").join("main.c"), source).expect("write source");

    let err = sync_packets_fs(&config_path, None).expect_err("invalid syntax should fail");
    assert!(err.to_string().contains("invalid @rat annotation syntax"));

    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn sync_packets_fs_rejects_non_packed_padding_layout() {
    let temp = std::env::temp_dir().join(format!("rat_sync_layout_block_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(temp.join("src")).expect("mkdir");

    let config_path = temp.join("rat.toml");
    write_test_config(&config_path, "src");

    let source = r#"
// @rat, plot
typedef struct {
  uint8_t a;
  uint32_t b;
} RatPadded;
"#;
    fs::write(temp.join("src").join("main.c"), source).expect("write source");

    let err = sync_packets_fs(&config_path, None).expect_err("sync should fail");
    assert!(
        err.to_string().contains("compiler-dependent padding"),
        "expected padding blocker, got {err:#}"
    );
    assert!(
        err.to_string().contains("layout validation failed"),
        "expected validation summary, got {err:#}"
    );

    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn sync_packets_fs_rejects_non_packed_wide_field_layout() {
    let temp =
        std::env::temp_dir().join(format!("rat_sync_layout_wide_block_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(temp.join("src")).expect("mkdir");

    let config_path = temp.join("rat.toml");
    write_test_config(&config_path, "src");

    let source = r#"
// @rat, plot
typedef struct {
  uint64_t tick;
} RatWide;
"#;
    fs::write(temp.join("src").join("main.c"), source).expect("write source");

    let err = sync_packets_fs(&config_path, None).expect_err("sync should fail");
    assert!(
        err.to_string().contains("contains >=8-byte fields"),
        "expected wide field blocker, got {err:#}"
    );

    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn sync_packets_fs_accepts_packed_layout_with_wide_fields() {
    let temp =
        std::env::temp_dir().join(format!("rat_sync_layout_packed_ok_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(temp.join("src")).expect("mkdir");

    let config_path = temp.join("rat.toml");
    write_test_config(&config_path, "src");

    let source = r#"
// @rat, plot
typedef struct __attribute__((packed)) {
  uint8_t a;
  uint64_t tick;
} RatPacked;
"#;
    fs::write(temp.join("src").join("main.c"), source).expect("write source");

    let result = sync_packets_fs(&config_path, None).expect("packed layout should pass");
    assert!(
        result.layout_warnings.is_empty(),
        "packed layout should not produce blockers/warnings: {:?}",
        result.layout_warnings
    );

    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn sync_packets_fs_rejects_aligned_layout_modifier() {
    let temp = std::env::temp_dir().join(format!("rat_sync_layout_reject_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(temp.join("src")).expect("mkdir");

    let config_path = temp.join("rat.toml");
    write_test_config(&config_path, "src");

    let source = r#"
// @rat, plot
typedef struct __attribute__((aligned(8))) {
  int32_t value;
} RatAligned;
"#;
    fs::write(temp.join("src").join("main.c"), source).expect("write source");

    let err = sync_packets_fs(&config_path, None).expect_err("aligned modifier should fail");
    assert!(err.to_string().contains("unsupported layout modifier"));

    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn sync_packets_fs_respects_ratignore_glob_rules() {
    let temp = std::env::temp_dir().join(format!("rat_sync_ignore_glob_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(temp.join("src")).expect("mkdir");

    let config_path = temp.join("rat.toml");
    write_test_config(&config_path, "src");

    fs::write(
        temp.join(".ratignore"),
        "# ignore sensor packet\nsrc/ignore_me.c\n",
    )
    .expect("write .ratignore");

    let kept = r#"
// @rat, plot
typedef struct {
  int32_t value;
  uint32_t tick;
} KeepPacket;
"#;
    let ignored = r#"
// @rat, plot
typedef struct {
  int32_t value;
  uint32_t tick;
} IgnoredPacket;
"#;
    fs::write(temp.join("src").join("keep.c"), kept).expect("write kept source");
    fs::write(temp.join("src").join("ignore_me.c"), ignored).expect("write ignored source");

    let result = sync_packets_fs(&config_path, None).expect("sync should pass");
    assert_eq!(result.packet_defs.len(), 1);
    assert_eq!(result.packet_defs[0].struct_name, "KeepPacket");

    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn sync_packets_fs_ratignore_supports_comments_and_blank_lines() {
    let temp =
        std::env::temp_dir().join(format!("rat_sync_ignore_comments_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(temp.join("src")).expect("mkdir");

    let config_path = temp.join("rat.toml");
    write_test_config(&config_path, "src");

    fs::write(temp.join(".ratignore"), "\n# comment\n\nsrc/skip.c\n").expect("write .ratignore");

    let keep = r#"
// @rat, plot
typedef struct {
  int32_t value;
  uint32_t tick;
} KeepPacket;
"#;
    let skip = r#"
// @rat, plot
typedef struct {
  int32_t value;
  uint32_t tick;
} SkipPacket;
"#;
    fs::write(temp.join("src").join("keep.c"), keep).expect("write keep");
    fs::write(temp.join("src").join("skip.c"), skip).expect("write skip");

    let result = sync_packets_fs(&config_path, None).expect("sync should pass");
    assert_eq!(result.packet_defs.len(), 1);
    assert_eq!(result.packet_defs[0].struct_name, "KeepPacket");

    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn sync_packets_fs_ratignore_supports_directory_glob() {
    let temp =
        std::env::temp_dir().join(format!("rat_sync_ignore_dir_glob_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(temp.join("src").join("generated")).expect("mkdir generated");
    fs::create_dir_all(temp.join("src").join("live")).expect("mkdir live");

    let config_path = temp.join("rat.toml");
    write_test_config(&config_path, "src");
    fs::write(temp.join(".ratignore"), "src/generated/**\n").expect("write .ratignore");

    let keep = r#"
// @rat, plot
typedef struct {
  int32_t value;
  uint32_t tick;
} LivePacket;
"#;
    let skip = r#"
// @rat, plot
typedef struct {
  int32_t value;
  uint32_t tick;
} GeneratedPacket;
"#;
    fs::write(temp.join("src").join("live").join("keep.c"), keep).expect("write keep");
    fs::write(temp.join("src").join("generated").join("skip.c"), skip).expect("write skip");

    let result = sync_packets_fs(&config_path, None).expect("sync should pass");
    assert_eq!(result.packet_defs.len(), 1);
    assert_eq!(result.packet_defs[0].struct_name, "LivePacket");

    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn sync_packets_fs_rejects_ratignore_negate_pattern() {
    let temp = std::env::temp_dir().join(format!("rat_sync_ignore_negate_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(temp.join("src")).expect("mkdir");

    let config_path = temp.join("rat.toml");
    write_test_config(&config_path, "src");
    fs::write(temp.join(".ratignore"), "!src/*.c\n").expect("write .ratignore");
    fs::write(
        temp.join("src").join("main.c"),
        r#"
// @rat, plot
typedef struct {
  int32_t value;
  uint32_t tick;
} KeepPacket;
"#,
    )
    .expect("write source");

    let err = sync_packets_fs(&config_path, None).expect_err("negate pattern should fail");
    assert!(
        err.to_string().contains("does not support negate patterns"),
        "unexpected error: {err:#}"
    );

    let _ = fs::remove_dir_all(&temp);
}
