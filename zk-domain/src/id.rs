use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

// ==========================================================================
// NoteId
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct NoteId(String);

impl NoteId {
    pub fn generate() -> Self {
        Self(ulid::Ulid::new().to_string())
    }

    pub fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for NoteId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl fmt::Display for NoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ==========================================================================
// AssetId
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct AssetId(String);

impl AssetId {
    pub fn generate() -> Self {
        Self(ulid::Ulid::new().to_string())
    }

    pub fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for AssetId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ==========================================================================
// DeviceId
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct DeviceId(String);

impl DeviceId {
    pub fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for DeviceId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

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

    pub fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for ContentHash {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

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
        let unified = s.replace('\\', "/"); // 1. unify separators
        unified
            .strip_prefix("./")  // 2. strip leading ./
            .unwrap_or(&unified)
            .split('/')  // 3. split on /
            .filter(|c| !c.is_empty())  // 4. drop empty segments
            .collect::<Vec<_>>()
            .join("/")
    }
}

impl From<String> for RelativePath {
    fn from(s: String) -> Self {
        Self::new(&s)
    }
}

impl fmt::Display for RelativePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
