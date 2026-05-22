// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::io::Cursor;

use zk_domain::id::{ContentHash, DeviceId, NoteId, RelativePath, ServerRevision};

#[test]
fn miri_smoke_covers_pure_domain_operations() {
    let note_id = NoteId::try_from_strict("01HZ3KQBFG9T2R5XJ4N8VW6YMP")
        .expect("fixture note id is a valid ULID");
    let device_id = DeviceId::new("device-miri");
    let revision = ServerRevision::new(42);
    let path = RelativePath::new("./folder/../notes\\miri.md");
    let bytes = b"miri deterministic content";
    let expected_hash = ContentHash::from_bytes(bytes);

    let (reader_hash, size_bytes) =
        ContentHash::from_reader(Cursor::new(bytes)).expect("cursor reads cannot fail");

    assert_eq!(note_id.as_str(), "01HZ3KQBFG9T2R5XJ4N8VW6YMP");
    assert_eq!(device_id.as_str(), "device-miri");
    assert_eq!(revision.value(), 42);
    assert_eq!(path.as_str(), "notes/miri.md");
    assert_eq!(reader_hash, expected_hash);
    assert_eq!(size_bytes, bytes.len() as u64);

    let json = serde_json::to_string(&path).expect("serialize relative path");
    let round_tripped: RelativePath =
        serde_json::from_str(&json).expect("deserialize relative path");
    assert_eq!(round_tripped, path);
}
