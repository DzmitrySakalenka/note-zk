use quickcheck::{Arbitrary, Gen};

pub const ID_PLAINTEXT_LEN: usize = 52;
pub const SHA256_DIGEST_LEN: usize = 32;
pub const SHA256_HEX_LEN: usize = 64;

const BASE32_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
const INVALID_ALPHABET: &[u8] = b"0189abcdefghijklmnopqrstuvwxyz=_-";

#[derive(Clone, Debug)]
pub struct Base32WrongLength(pub String);

#[derive(Clone, Debug)]
pub struct InvalidBase32Text(pub String);

impl Arbitrary for Base32WrongLength {
    fn arbitrary(g: &mut Gen) -> Self {
        let mut length = usize::arbitrary(g) % ((ID_PLAINTEXT_LEN * 2) + 1);
        if length == ID_PLAINTEXT_LEN {
            length += 1;
        }

        Self(base32_text(g, length))
    }
}

impl Arbitrary for InvalidBase32Text {
    fn arbitrary(g: &mut Gen) -> Self {
        let mut text = base32_text(g, ID_PLAINTEXT_LEN);
        let index = usize::arbitrary(g) % ID_PLAINTEXT_LEN;
        let invalid = invalid_char(g).to_string();

        text.replace_range(index..=index, &invalid);

        Self(text)
    }
}

pub fn decoded_sha256_len(hex_digest: &str) -> Option<usize> {
    data_encoding::HEXLOWER
        .decode(hex_digest.as_bytes())
        .ok()
        .map(|digest| digest.len())
}

fn base32_text(g: &mut Gen, length: usize) -> String {
    (0..length)
        .map(|_| {
            let index = usize::arbitrary(g) % BASE32_ALPHABET.len();
            BASE32_ALPHABET[index] as char
        })
        .collect()
}

fn invalid_char(g: &mut Gen) -> char {
    let index = usize::arbitrary(g) % INVALID_ALPHABET.len();

    INVALID_ALPHABET[index] as char
}
