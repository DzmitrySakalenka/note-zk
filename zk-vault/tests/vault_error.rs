// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{error::Error, io, path::PathBuf};

use zk_vault::error::VaultError;

#[test]
fn io_error_variant_includes_path_and_source() {
    let err = VaultError::Io {
        path: PathBuf::from("notes/a.md"),
        source: io::Error::new(io::ErrorKind::NotFound, "missing file"),
    };
    let rendered = err.to_string();

    assert!(rendered.contains("notes/a.md"));
    assert!(rendered.contains("I/O"));
    assert!(Error::source(&err).is_some());
}

#[test]
fn frontmatter_parse_error_variant_includes_path_and_source() {
    let parse_error =
        serde_yaml::from_str::<serde_yaml::Value>(": invalid").expect_err("yaml must fail");
    let err = VaultError::FrontmatterParse {
        path: PathBuf::from("notes/bad.md"),
        source: parse_error,
    };
    let rendered = err.to_string();

    assert!(rendered.contains("notes/bad.md"));
    assert!(rendered.contains("frontmatter"));
    assert!(Error::source(&err).is_some());
}

#[test]
fn utf8_decode_error_variant_includes_path_and_source() {
    let decode_error = String::from_utf8(vec![0xFF]).expect_err("bytes should be invalid utf8");
    let err = VaultError::Utf8Decode {
        path: PathBuf::from("notes/non_utf8.md"),
        source: decode_error,
    };
    let rendered = err.to_string();

    assert!(rendered.contains("notes/non_utf8.md"));
    assert!(rendered.contains("UTF-8"));
    assert!(Error::source(&err).is_some());
}
