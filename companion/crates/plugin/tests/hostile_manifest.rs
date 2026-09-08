//! The untrusted-input corpus for the plugin manifest parser (Task 1's Step
//! 5): every case here is something a hostile or merely broken manifest
//! author could send, and every one of them must come back as a named
//! `Err`, never a panic, an unbounded allocation, or a hang.
//!
//! This exercises the crate's public API only (`plugin::parse_manifest` and
//! `plugin::parse_manifest_bytes`), the same surface Tasks 2, 3, 4 and 7
//! will call.

use plugin::{ManifestError, parse_manifest, parse_manifest_bytes};

const MINIMAL_HEADER: &str = r#"
name = "aqi"
version = "1.0.0"

[source]
kind = "json"
url = "https://example.invalid/aqi.json"
refresh_minutes = 15

"#;

#[test]
fn a_ten_megabyte_manifest_is_rejected() {
    let huge = "x".repeat(10 * 1024 * 1024);
    let err = parse_manifest(&huge).expect_err("a 10 MB manifest must be rejected");
    assert!(matches!(err, ManifestError::TooLarge { .. }));
}

#[test]
fn deeply_nested_tables_are_rejected_without_a_stack_overflow() {
    let depth = 5_000;
    let mut source = String::from("x = ");
    for _ in 0..depth {
        source.push_str("{a=");
    }
    source.push('1');
    for _ in 0..depth {
        source.push('}');
    }
    // The point of this test is that the process is still alive to make
    // this assertion at all.
    let err = parse_manifest(&source).expect_err("deep nesting must be rejected, not crash");
    assert!(matches!(err, ManifestError::TooDeeplyNested { .. }));
}

#[test]
fn duplicate_asset_names_are_rejected() {
    let source = format!(
        "{MINIMAL_HEADER}\
         [[assets]]\nkind = \"font\"\nfile = \"dup.ttf\"\n\n\
         [[assets]]\nkind = \"image\"\nfile = \"dup.ttf\"\n"
    );
    let err = parse_manifest(&source).expect_err("a duplicate asset file name must be rejected");
    assert!(matches!(err, ManifestError::DuplicateAsset { file } if file == "dup.ttf"));
}

#[test]
fn a_glyph_codepoint_in_the_surrogate_range_is_rejected() {
    let source = format!(
        "{MINIMAL_HEADER}\
         [[assets]]\nkind = \"icon-font\"\nfile = \"icons.ttf\"\n\
         glyphs = [ {{ name = \"lone-surrogate\", codepoint = 0xDFFF }} ]\n"
    );
    let err = parse_manifest(&source).expect_err("a surrogate codepoint must be rejected");
    assert!(matches!(
        err,
        ManifestError::InvalidGlyphCodepoint {
            codepoint: 0xDFFF,
            ..
        }
    ));
}

#[test]
fn a_glyph_codepoint_above_the_unicode_ceiling_is_rejected() {
    let source = format!(
        "{MINIMAL_HEADER}\
         [[assets]]\nkind = \"icon-font\"\nfile = \"icons.ttf\"\n\
         glyphs = [ {{ name = \"too-high\", codepoint = 0x110000 }} ]\n"
    );
    let err = parse_manifest(&source).expect_err("a codepoint above 0x10FFFF must be rejected");
    assert!(matches!(
        err,
        ManifestError::InvalidGlyphCodepoint {
            codepoint: 0x0011_0000,
            ..
        }
    ));
}

#[test]
fn zero_refresh_minutes_is_rejected() {
    let source = r#"
name = "aqi"
version = "1.0.0"

[source]
kind = "json"
url = "https://example.invalid/aqi.json"
refresh_minutes = 0
"#;
    let err = parse_manifest(source).expect_err("refresh_minutes = 0 must be rejected");
    assert!(matches!(
        err,
        ManifestError::InvalidRefreshMinutes { value: 0 }
    ));
}

#[test]
fn a_file_scheme_url_is_rejected() {
    let source = r#"
name = "aqi"
version = "1.0.0"

[source]
kind = "json"
url = "file:///etc/passwd"
refresh_minutes = 15
"#;
    let err = parse_manifest(source).expect_err("a file:// source URL must be rejected");
    assert!(matches!(
        err,
        ManifestError::DisallowedUrlScheme { scheme } if scheme == "file"
    ));
}

#[test]
fn non_utf8_bytes_are_rejected() {
    // A lone continuation byte: never valid UTF-8 at that position.
    let bytes: &[u8] = &[b'n', b'a', b'm', b'e', 0x80, 0x00];
    let err = parse_manifest_bytes(bytes).expect_err("non-UTF-8 bytes must be rejected");
    assert!(matches!(err, ManifestError::InvalidUtf8));
}
