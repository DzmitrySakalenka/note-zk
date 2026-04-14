use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

macro_rules! impl_string_newtype {
    ($name:ident) => {
        impl $name {
            pub fn from(s: &str) -> Self {
                Self(s.to_string())
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
            pub fn generate() -> Self {
                Self(ulid::Ulid::new().to_string())
            }

            pub fn from(s: &str) -> Self {
                Self(s.to_string())
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
pub struct NoteId(String);

impl_generated_string_newtype!(NoteId);

// ==========================================================================
// AssetId
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct AssetId(String);

impl_generated_string_newtype!(AssetId);

// ==========================================================================
// DeviceId
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct DeviceId(String);

impl_string_newtype!(DeviceId);

// ==========================================================================
// ContentHash
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct ContentHash(String);

impl ContentHash {
    pub fn from_bytes(data: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        Self(hex::encode(result))
    }
}

impl_string_newtype!(ContentHash);

// ==========================================================================
// ServerRevision
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize, PartialOrd, Ord)]
pub struct ServerRevision(i64);

impl ServerRevision {
    pub fn new(value: i64) -> Self {
        Self(value)
    }

    pub fn value(&self) -> i64 {
        self.0
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
pub struct RelativePath(String);

impl RelativePath {
    pub fn new(s: &str) -> Self {
        let normalized = Self::normalize_path(s);
        Self(normalized)
    }

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
