use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    io::{self, Read},
};

const CONTENT_HASH_CHUNK_SIZE: usize = 64 * 1024;

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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
/// Stable identifier for notes across rename and move operations.
pub struct NoteId(String);

impl_generated_string_newtype!(NoteId);

// ==========================================================================
// AssetId
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
/// Stable identifier for assets across rename and move operations.
pub struct AssetId(String);

impl_generated_string_newtype!(AssetId);

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
