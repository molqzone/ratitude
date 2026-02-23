use std::fs;
use std::path::Path;

use tree_sitter::Parser;

use crate::model::ParsedTaggedFile;
use crate::parser::{collect_comment_tags, collect_type_definitions};
use crate::SyncError;

pub(crate) fn parse_tagged_file(path: &Path) -> Result<Option<ParsedTaggedFile>, SyncError> {
    let source = fs::read(path).map_err(|source_err| SyncError::ReadSource {
        path: path.to_path_buf(),
        source: source_err,
    })?;
    parse_tagged_source(path, &source)
}

pub(crate) fn parse_tagged_source(
    path: &Path,
    source: &[u8],
) -> Result<Option<ParsedTaggedFile>, SyncError> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_c::LANGUAGE.into())
        .map_err(|err| {
            SyncError::Validation(format!(
                "tree-sitter init failed for {}: {err}",
                path.display()
            ))
        })?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| SyncError::ParseSource {
            path: path.to_path_buf(),
        })?;

    let tags = collect_comment_tags(&tree, source, path)?;
    if tags.is_empty() {
        return Ok(None);
    }

    let mut structs = collect_type_definitions(&tree, source, path)?;
    if structs.is_empty() {
        return Err(SyncError::Validation(format!(
            "found @rat tags in {} but no typedef struct definitions",
            path.display()
        )));
    }
    structs.sort_by_key(|value| value.start_byte);

    Ok(Some(ParsedTaggedFile { tags, structs }))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn parse_tagged_source_supports_new_tag_syntax_without_filesystem() {
        let src = br#"
// @rat, plot
typedef struct {
  int32_t value;
} RatSample;
"#;
        let parsed = parse_tagged_source(Path::new("mem://demo.c"), src)
            .expect("parse")
            .expect("tagged file");
        assert_eq!(parsed.tags.len(), 1);
        assert_eq!(parsed.structs.len(), 1);
        assert_eq!(parsed.structs[0].name, "RatSample");
    }

    #[test]
    fn parse_tagged_source_rejects_invalid_tag_syntax_without_filesystem() {
        let src = br#"
// @rat:id=0x01, type=plot
typedef struct {
  int32_t value;
} RatSample;
"#;
        let err = parse_tagged_source(Path::new("mem://demo.c"), src)
            .expect_err("invalid syntax should fail");
        assert!(err.to_string().contains("invalid @rat annotation syntax"));
    }

    #[test]
    fn parse_tagged_source_rejects_trailing_text_after_annotation() {
        let src = br#"
// @rat, plot payload packet
typedef struct {
  int32_t value;
} RatSample;
"#;
        let err = parse_tagged_source(Path::new("mem://demo.c"), src)
            .expect_err("trailing text should fail");
        assert!(err.to_string().contains("invalid @rat annotation syntax"));
    }

    #[test]
    fn parse_tagged_source_accepts_whitespace_around_type_separator() {
        let src = br#"
//     @rat   ,    quat    
typedef struct {
  float x;
  float y;
  float z;
  float w;
} RatQuat;
"#;
        let parsed = parse_tagged_source(Path::new("mem://demo.c"), src)
            .expect("parse")
            .expect("tagged file");
        assert_eq!(parsed.tags.len(), 1);
        assert_eq!(parsed.tags[0].packet_type, "quat");
    }
}
