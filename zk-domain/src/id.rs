use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    io::{self, Read},
};
use thiserror::Error;

const CONTENT_HASH_CHUNK_SIZE: usize = 64 * 1024;
const STRICT_ID_EXPECTED: &str = "a 26-character ULID or 36-character hyphenated UUID";
const STRICT_CONTENT_HASH_EXPECTED: &str = "exactly 64 lowercase hexadecimal characters";

macro_rules! impl_string_newtype {
    ($name:ident) => {
        impl $name {
            /// Creates an identifier wrapper from an existing string value.
            pub fn new(s: &str) -> Self {
                Self(s.to_string())
            }

            /// Returns the identifier as a borrowed string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

macro_rules! impl_generated_string_newtype {
    ($name:ident) => {
        impl $name {
            /// Generates a new ULID-backed identifier value.
            pub fn generate() -> Self {
                Self(ulid::Ulid::new().to_string())
            }

            /// Creates an identifier wrapper from an existing string value.
            pub fn new(s: &str) -> Self {
                Self(s.to_string())
            }

            /// Returns the identifier as a borrowed string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

macro_rules! impl_from_string_via_new_and_display {
    ($name:ident) => {
        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self::new(&s)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

// ==========================================================================
// NoteId
// ==========================================================================

/// Strict identifier parsing failure for server-facing boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdFormatError {
    /// The input string is empty.
    #[error("value must not be empty")]
    Empty,
    /// The input does not match the expected overall format.
    #[error("value must match {expected}")]
    InvalidShape {
        /// Human-readable description of the accepted format.
        expected: &'static str,
    },
    /// The input shape is plausible but contains invalid characters.
    #[error("value contains invalid characters")]
    InvalidChars,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
/// Stable identifier for notes across rename and move operations.
pub struct NoteId(String);

impl_generated_string_newtype!(NoteId);

impl NoteId {
    /// Strictly parses a note identifier for untrusted boundary inputs.
    ///
    /// Lenient construction remains available through [`NoteId::new`] and
    /// [`From<String>`] for legacy round-trip scenarios inside the vault.
    ///
    /// # Errors
    ///
    /// Returns [`IdFormatError`] when the value is empty or is neither a
    /// canonical ULID nor a hyphenated UUID string.
    pub fn try_from_strict(value: &str) -> Result<Self, IdFormatError> {
        validate_generated_id(value)?;
        Ok(Self::new(value))
    }
}

// ==========================================================================
// AssetId
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
/// Stable identifier for assets across rename and move operations.
pub struct AssetId(String);

impl_generated_string_newtype!(AssetId);

impl AssetId {
    /// Strictly parses an asset identifier for untrusted boundary inputs.
    ///
    /// Lenient construction remains available through [`AssetId::new`] and
    /// [`From<String>`] for legacy round-trip scenarios inside the vault.
    ///
    /// # Errors
    ///
    /// Returns [`IdFormatError`] when the value is empty or is neither a
    /// canonical ULID nor a hyphenated UUID string.
    pub fn try_from_strict(value: &str) -> Result<Self, IdFormatError> {
        validate_generated_id(value)?;
        Ok(Self::new(value))
    }
}

// ==========================================================================
// DeviceId
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
/// Identifier for a syncing device or client instance.
pub struct DeviceId(String);

impl_string_newtype!(DeviceId);

// ==========================================================================
// ContentHash
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
/// Content hash used to detect file changes and sync drift.
pub struct ContentHash(String);

impl ContentHash {
    /// Computes a SHA-256 hash from a reader in fixed-size chunks and returns
    /// both the final hash and the total byte count consumed.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] when the underlying reader fails.
    pub fn from_reader<R: Read>(mut reader: R) -> io::Result<(Self, u64)> {
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; CONTENT_HASH_CHUNK_SIZE];
        let mut total_bytes = 0_u64;

        loop {
            let bytes_read = reader.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }

            hasher.update(&buffer[..bytes_read]);
            total_bytes += u64::try_from(bytes_read).map_err(|_| {
                io::Error::other("reader chunk size exceeds u64 byte counter capacity")
            })?;
        }

        Ok((Self(hex::encode(hasher.finalize())), total_bytes))
    }

    /// Computes a SHA-256 hash from raw bytes and returns it as lowercase hex.
    ///
    /// # Panics
    ///
    /// Panics only if hashing a byte slice returns an I/O error, which cannot
    /// happen for in-memory slices.
    pub fn from_bytes(data: &[u8]) -> Self {
        let (hash, _) = Self::from_reader(data).expect("byte slice reads cannot fail");
        hash
    }
}

impl_string_newtype!(ContentHash);

impl ContentHash {
    /// Strictly parses a content hash for untrusted boundary inputs.
    ///
    /// Lenient construction remains available through [`ContentHash::new`] and
    /// [`From<String>`] for legacy round-trip scenarios inside the vault.
    ///
    /// # Errors
    ///
    /// Returns [`IdFormatError`] when the value is empty, has the wrong
    /// length, or contains characters outside lowercase hexadecimal.
    pub fn try_from_strict(value: &str) -> Result<Self, IdFormatError> {
        if value.is_empty() {
            return Err(IdFormatError::Empty);
        }

        if value.len() != 64 {
            return Err(IdFormatError::InvalidShape {
                expected: STRICT_CONTENT_HASH_EXPECTED,
            });
        }

        if !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(IdFormatError::InvalidChars);
        }

        Ok(Self::new(value))
    }
}

// ==========================================================================
// ServerRevision
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize, PartialOrd, Ord)]
/// Monotonic server-side revision marker for sync ordering.
pub struct ServerRevision(i64);

impl ServerRevision {
    /// Constructs a server revision value from a raw integer.
    pub fn new(value: i64) -> Self {
        Self(value)
    }

    /// Returns the raw integer representation of the revision marker.
    pub fn value(&self) -> i64 {
        self.0
    }
}

impl From<i64> for ServerRevision {
    fn from(value: i64) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for ServerRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ==========================================================================
// RelativePath
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
/// Normalized vault-relative path used across domain models.
pub struct RelativePath(String);

impl RelativePath {
    /// Creates a normalized vault-relative path from input text.
    pub fn new(s: &str) -> Self {
        let normalized = Self::normalize_path(s);
        Self(normalized)
    }

    /// Returns the normalized path as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn normalize_path(s: &str) -> String {
        let unified = s.replace('\\', "/");
        let without_dot_prefix = unified.strip_prefix("./").unwrap_or(&unified);
        let mut segments: Vec<&str> = Vec::new();

        for segment in without_dot_prefix.split('/') {
            if segment.is_empty() || segment == "." {
                continue;
            }
            if segment == ".." {
                if !segments.is_empty() {
                    let _ = segments.pop();
                }
                continue;
            }
            segments.push(segment);
        }

        segments.join("/")
    }
}

impl_from_string_via_new_and_display!(RelativePath);

fn validate_generated_id(value: &str) -> Result<(), IdFormatError> {
    if value.is_empty() {
        return Err(IdFormatError::Empty);
    }

    match value.len() {
        26 => validate_ulid(value),
        36 => validate_uuid(value),
        _ => Err(IdFormatError::InvalidShape {
            expected: STRICT_ID_EXPECTED,
        }),
    }
}

fn validate_ulid(value: &str) -> Result<(), IdFormatError> {
    if !value.bytes().all(is_valid_ulid_byte) {
        return Err(IdFormatError::InvalidChars);
    }

    ulid::Ulid::from_string(value)
        .map(|_| ())
        .map_err(|_| IdFormatError::InvalidShape {
            expected: STRICT_ID_EXPECTED,
        })
}

fn validate_uuid(value: &str) -> Result<(), IdFormatError> {
    for (index, byte) in value.bytes().enumerate() {
        let is_dash_position = matches!(index, 8 | 13 | 18 | 23);

        if is_dash_position {
            if byte != b'-' {
                return Err(IdFormatError::InvalidShape {
                    expected: STRICT_ID_EXPECTED,
                });
            }

            continue;
        }

        if !byte.is_ascii_hexdigit() {
            return Err(IdFormatError::InvalidChars);
        }
    }

    Ok(())
}

fn is_valid_ulid_byte(byte: u8) -> bool {
    byte.is_ascii_digit()
        || matches!(
            byte,
            b'A'..=b'H' | b'J'..=b'K' | b'M'..=b'N' | b'P'..=b'T' | b'V'..=b'Z'
        )
}
