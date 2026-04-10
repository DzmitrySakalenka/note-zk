use std::collections::HashSet;

use serde_json;
use zk_domain::id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision};

// ==========================================================================
// NoteId
// ==========================================================================

#[test]
fn note_id_display_roundtrip() {
    let id = NoteId::from("01HZ3KQBFG9T2R5XJ4N8VW6YMP");
    let displayed = id.to_string();
    assert_eq!(displayed, "01HZ3KQBFG9T2R5XJ4N8VW6YMP");
    let parsed = NoteId::from(displayed.as_str());
    assert_eq!(id, parsed);
}

#[test]
fn note_id_generate_produces_unique_values() {
    let a = NoteId::generate();
    let b = NoteId::generate();
    assert_ne!(a, b);
}

#[test]
fn note_id_generate_is_valid_ulid() {
    let id = NoteId::generate();
    let s = id.to_string();
    assert_eq!(s.len(), 26, "ULID should be 26 characters");
    assert!(
        s.chars().all(|c| c.is_ascii_alphanumeric()),
        "ULID should be alphanumeric: {s}"
    );
}

#[test]
fn note_id_hash_and_eq() {
    let a = NoteId::from("01HZ3KQBFG9T2R5XJ4N8VW6YMP");
    let b = NoteId::from("01HZ3KQBFG9T2R5XJ4N8VW6YMP");
    let c = NoteId::from("01JABC123456789DEF0GHIJKLM");

    assert_eq!(a, b);
    assert_ne!(a, c);

    let mut set = HashSet::new();
    set.insert(a.clone());
    assert!(set.contains(&b));
    assert!(!set.contains(&c));
}

#[test]
fn note_id_serde_roundtrip() {
    let id = NoteId::from("01HZ3KQBFG9T2R5XJ4N8VW6YMP");
    let json = serde_json::to_string(&id).expect("serialize");
    assert_eq!(json, r#""01HZ3KQBFG9T2R5XJ4N8VW6YMP""#);
    let back: NoteId = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(id, back);
}

// ==========================================================================
// AssetId
// ==========================================================================

#[test]
fn asset_id_display_and_eq() {
    let a = AssetId::from("01HZ3KQBFG9T2R5XJ4N8VW6YMP");
    let b = AssetId::from("01HZ3KQBFG9T2R5XJ4N8VW6YMP");
    assert_eq!(a, b);
    assert_eq!(a.to_string(), "01HZ3KQBFG9T2R5XJ4N8VW6YMP");
}

#[test]
fn asset_id_serde_roundtrip() {
    let id = AssetId::from("ASSET001");
    let json = serde_json::to_string(&id).expect("serialize");
    let back: AssetId = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(id, back);
}

// ==========================================================================
// DeviceId
// ==========================================================================

#[test]
fn device_id_display_and_eq() {
    let a = DeviceId::from("device-macbook-2026");
    let b = DeviceId::from("device-macbook-2026");
    assert_eq!(a, b);
    assert_eq!(a.to_string(), "device-macbook-2026");
}

#[test]
fn device_id_serde_roundtrip() {
    let id = DeviceId::from("device-macbook-2026");
    let json = serde_json::to_string(&id).expect("serialize");
    let back: DeviceId = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(id, back);
}

// ==========================================================================
// ContentHash
// ==========================================================================

#[test]
fn content_hash_from_bytes_known_vector() {
    // SHA-256 of empty input is a well-known constant.
    let hash = ContentHash::from_bytes(b"");
    assert_eq!(
        hash.to_string(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn content_hash_from_bytes_hello_world() {
    let hash = ContentHash::from_bytes(b"hello world");
    assert_eq!(
        hash.to_string(),
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
    );
}

#[test]
fn content_hash_is_lowercase_hex() {
    let hash = ContentHash::from_bytes(b"test data");
    let s = hash.to_string();
    assert_eq!(s.len(), 64, "SHA-256 hex should be 64 chars");
    assert!(
        s.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "hash should be lowercase hex: {s}"
    );
}

#[test]
fn content_hash_equality() {
    let a = ContentHash::from_bytes(b"same");
    let b = ContentHash::from_bytes(b"same");
    let c = ContentHash::from_bytes(b"different");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn content_hash_serde_roundtrip() {
    let hash = ContentHash::from_bytes(b"serde test");
    let json = serde_json::to_string(&hash).expect("serialize");
    let back: ContentHash = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(hash, back);
}

#[test]
fn content_hash_from_str() {
    let hex = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let hash = ContentHash::from(hex);
    assert_eq!(hash.to_string(), hex);
}

// ==========================================================================
// ServerRevision
// ==========================================================================

#[test]
fn server_revision_display_and_eq() {
    let a = ServerRevision::new(42);
    let b = ServerRevision::new(42);
    let c = ServerRevision::new(99);
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_eq!(a.to_string(), "42");
}

#[test]
fn server_revision_value_accessor() {
    let rev = ServerRevision::new(100);
    assert_eq!(rev.value(), 100);
}

#[test]
fn server_revision_serde_roundtrip() {
    let rev = ServerRevision::new(7);
    let json = serde_json::to_string(&rev).expect("serialize");
    assert_eq!(json, "7");
    let back: ServerRevision = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(rev, back);
}

#[test]
fn server_revision_ordering() {
    let a = ServerRevision::new(1);
    let b = ServerRevision::new(2);
    assert!(a < b);
}

// ==========================================================================
// RelativePath
// ==========================================================================

#[test]
fn relative_path_preserves_forward_slashes() {
    let p = RelativePath::new("notes/daily/2026-04-09.md");
    assert_eq!(p.to_string(), "notes/daily/2026-04-09.md");
}

#[test]
fn relative_path_normalizes_backslashes() {
    let p = RelativePath::new(r"notes\daily\2026-04-09.md");
    assert_eq!(p.to_string(), "notes/daily/2026-04-09.md");
}

#[test]
fn relative_path_strips_leading_dot_slash() {
    let p = RelativePath::new("./notes/daily/2026-04-09.md");
    assert_eq!(p.to_string(), "notes/daily/2026-04-09.md");
}

#[test]
fn relative_path_strips_leading_dot_backslash() {
    let p = RelativePath::new(r".\notes\daily\2026-04-09.md");
    assert_eq!(p.to_string(), "notes/daily/2026-04-09.md");
}

#[test]
fn relative_path_collapses_repeated_slashes() {
    let p = RelativePath::new("notes//daily///file.md");
    assert_eq!(p.to_string(), "notes/daily/file.md");
}

#[test]
fn relative_path_strips_trailing_slash() {
    let p = RelativePath::new("notes/daily/");
    assert_eq!(p.to_string(), "notes/daily");
}

#[test]
fn relative_path_simple_filename() {
    let p = RelativePath::new("readme.md");
    assert_eq!(p.to_string(), "readme.md");
}

#[test]
fn relative_path_equality() {
    let a = RelativePath::new("notes/foo.md");
    let b = RelativePath::new(r"notes\foo.md");
    assert_eq!(
        a, b,
        "same path with different separators should be equal after normalization"
    );
}

#[test]
fn relative_path_hash_consistency() {
    let a = RelativePath::new("notes/foo.md");
    let b = RelativePath::new(r".\notes\foo.md");
    let mut set = HashSet::new();
    set.insert(a);
    assert!(
        set.contains(&b),
        "normalized-equal paths should hash the same"
    );
}

#[test]
fn relative_path_serde_roundtrip() {
    let path = RelativePath::new("notes/daily/2026-04-09.md");
    let json = serde_json::to_string(&path).expect("serialize");
    assert_eq!(json, r#""notes/daily/2026-04-09.md""#);
    let back: RelativePath = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(path, back);
}

#[test]
fn relative_path_as_str() {
    let path = RelativePath::new("notes/foo.md");
    assert_eq!(path.as_str(), "notes/foo.md");
}
