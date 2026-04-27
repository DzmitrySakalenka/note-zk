// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use zk_domain::id::{AssetId, ContentHash, IdFormatError, NoteId};

const VALID_ULID: &str = "01HZ3KQBFG9T2R5XJ4N8VW6YMP";
const VALID_UUID: &str = "550e8400-e29b-41d4-a716-446655440000";
const VALID_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[test]
fn note_id_try_from_strict_accepts_valid_ulid() {
    let id = NoteId::try_from_strict(VALID_ULID).expect("valid ULID should parse");
    assert_eq!(id.as_str(), VALID_ULID);
}

#[test]
fn asset_id_try_from_strict_accepts_valid_uuid() {
    let id = AssetId::try_from_strict(VALID_UUID).expect("valid UUID should parse");
    assert_eq!(id.as_str(), VALID_UUID);
}

#[test]
fn note_id_try_from_strict_rejects_empty_value() {
    let error = NoteId::try_from_strict("").expect_err("empty input must fail");
    assert_eq!(error, IdFormatError::Empty);
}

#[test]
fn note_id_try_from_strict_rejects_short_ulid_prefix() {
    let error = NoteId::try_from_strict(&VALID_ULID[..25]).expect_err("short ULID must fail");
    assert_eq!(
        error,
        IdFormatError::InvalidShape {
            expected: "a 26-character ULID or 36-character hyphenated UUID",
        }
    );
}

#[test]
fn note_id_try_from_strict_rejects_invalid_ulid_characters() {
    let error =
        NoteId::try_from_strict("01HZ3KQBFG9T2R5XJ4N8VW6YMU").expect_err("invalid ULID must fail");
    assert_eq!(error, IdFormatError::InvalidChars);
}

#[test]
fn content_hash_try_from_strict_accepts_valid_raw_hex() {
    let hash = ContentHash::try_from_strict(VALID_HASH).expect("valid content hash should parse");
    assert_eq!(hash.as_str(), VALID_HASH);
}

#[test]
fn content_hash_try_from_strict_rejects_uppercase_hex() {
    let error = ContentHash::try_from_strict(
        "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855",
    )
    .expect_err("uppercase hash must fail");
    assert_eq!(error, IdFormatError::InvalidChars);
}

#[test]
fn content_hash_try_from_strict_rejects_prefixed_hash() {
    let error = ContentHash::try_from_strict(
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    )
    .expect_err("prefixed hash must fail");
    assert_eq!(
        error,
        IdFormatError::InvalidShape {
            expected: "exactly 64 lowercase hexadecimal characters",
        }
    );
}
