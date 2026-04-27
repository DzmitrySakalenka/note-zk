// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::io::Cursor;

use zk_domain::id::ContentHash;

const MEBIBYTE: usize = 1024 * 1024;

#[test]
fn content_hash_from_reader_empty_matches_known_digest() {
    let (hash, size_bytes) =
        ContentHash::from_reader(Cursor::new(Vec::<u8>::new())).expect("hash empty reader");

    assert_eq!(size_bytes, 0);
    assert_eq!(
        hash.as_str(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn content_hash_from_reader_matches_from_bytes_for_large_buffer() {
    let bytes = deterministic_bytes(10 * MEBIBYTE);
    let expected_hash = ContentHash::from_bytes(&bytes);

    let (reader_hash, size_bytes) =
        ContentHash::from_reader(Cursor::new(bytes.clone())).expect("hash large reader");

    assert_eq!(reader_hash, expected_hash);
    assert_eq!(size_bytes, bytes.len() as u64);
}

fn deterministic_bytes(len: usize) -> Vec<u8> {
    let mut state = 0x1234_5678_9abc_def0_u64;
    let mut bytes = Vec::with_capacity(len);

    for _ in 0..len {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        bytes.push((state >> 56) as u8);
    }

    bytes
}
