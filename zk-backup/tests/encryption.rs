// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::fs;

use data_encoding::HEXLOWER;
use secrecy::ExposeSecret;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use zk_backup::{
    BackupError,
    encryption::{
        AgeIdentityDecryptor, AgeRecipientEncryptor, BackupDecryptor, BackupEncryptor,
        PassthroughEncryptor,
    },
};

#[test]
fn age_encrypt_then_decrypt_round_trips_large_fixture() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let plaintext_path = tempdir.path().join("archive.tar.zst");
    let ciphertext_path = tempdir.path().join("archive.tar.zst.age");
    let decrypted_path = tempdir.path().join("archive.decrypted.tar.zst");
    let plaintext = fixture_bytes();
    fs::write(&plaintext_path, &plaintext).expect("plaintext fixture should be written");
    let identity = age::x25519::Identity::generate();
    let encryptor = AgeRecipientEncryptor::from_recipient_string(&identity.to_public().to_string())
        .expect("recipient should parse");
    let decryptor =
        AgeIdentityDecryptor::from_identity_string(identity.to_string().expose_secret())
            .expect("identity should parse");

    encryptor
        .encrypt_in_place(&plaintext_path, &ciphertext_path)
        .expect("archive should encrypt");
    decryptor
        .decrypt_in_place(&ciphertext_path, &decrypted_path)
        .expect("archive should decrypt");

    assert_ne!(
        fs::read(&ciphertext_path).expect("ciphertext should be read"),
        plaintext
    );
    assert_eq!(
        fs::read(&decrypted_path).expect("decrypted plaintext should be read"),
        plaintext
    );
}

#[test]
fn age_decrypt_with_wrong_identity_returns_encryption_error() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let plaintext_path = tempdir.path().join("archive.tar.zst");
    let ciphertext_path = tempdir.path().join("archive.tar.zst.age");
    let decrypted_path = tempdir.path().join("wrong-identity.tar.zst");
    fs::write(&plaintext_path, fixture_bytes()).expect("plaintext fixture should be written");
    let matching_identity = age::x25519::Identity::generate();
    let wrong_identity = age::x25519::Identity::generate();
    let encryptor =
        AgeRecipientEncryptor::from_recipient_string(&matching_identity.to_public().to_string())
            .expect("recipient should parse");
    let decryptor =
        AgeIdentityDecryptor::from_identity_string(wrong_identity.to_string().expose_secret())
            .expect("identity should parse");

    encryptor
        .encrypt_in_place(&plaintext_path, &ciphertext_path)
        .expect("archive should encrypt");
    let result = decryptor.decrypt_in_place(&ciphertext_path, &decrypted_path);

    assert!(matches!(result, Err(BackupError::Encryption(_))));
}

#[test]
fn passthrough_encryptor_copies_plaintext_bit_for_bit() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let plaintext_path = tempdir.path().join("archive.tar.zst");
    let copied_path = tempdir.path().join("archive.copy.tar.zst");
    let plaintext = fixture_bytes();
    fs::write(&plaintext_path, &plaintext).expect("plaintext fixture should be written");
    let encryptor = PassthroughEncryptor;

    encryptor
        .encrypt_in_place(&plaintext_path, &copied_path)
        .expect("archive should be copied");

    assert_eq!(encryptor.extension(), ".zst");
    assert_eq!(
        fs::read(&copied_path).expect("copied plaintext should be read"),
        plaintext
    );
}

#[test]
fn invalid_recipient_string_returns_encryption_error() {
    let result = AgeRecipientEncryptor::from_recipient_string("not-a-recipient");

    assert!(matches!(result, Err(BackupError::Encryption(_))));
}

#[test]
fn recipient_fingerprint_is_sha256_prefix_hex() {
    let identity = age::x25519::Identity::generate();
    let recipient = identity.to_public().to_string();
    let encryptor =
        AgeRecipientEncryptor::from_recipient_string(&recipient).expect("recipient should parse");
    let expected = expected_fingerprint(&recipient);

    assert_eq!(encryptor.fingerprint(), expected);
    assert_eq!(encryptor.fingerprint().len(), 32);
    assert!(
        encryptor
            .fingerprint()
            .bytes()
            .all(|byte: u8| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    assert_eq!(encryptor.extension(), ".zst.age");
}

fn expected_fingerprint(recipient: &str) -> String {
    let digest = Sha256::digest(recipient.as_bytes());

    HEXLOWER.encode(&digest[..16])
}

fn fixture_bytes() -> Vec<u8> {
    (0..(1024 * 1024))
        .map(|index: usize| {
            let mixed = index * 31 + index / 7;
            mixed.to_le_bytes()[0]
        })
        .collect()
}
