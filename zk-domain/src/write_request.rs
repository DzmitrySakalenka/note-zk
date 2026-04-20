use serde::{Deserialize, Serialize};

use crate::id::RelativePath;

// ==========================================================================
// TargetSelector
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
/// Identifies one note or asset by stable ID or current vault-relative path.
pub enum TargetSelector {
    /// Selects an item by its raw ID string.
    ById(String),
    /// Selects an item by its vault-relative path.
    ByPath(RelativePath),
}

// ==========================================================================
// ItemKind
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
/// Write-path hint describing whether a target is a note or an asset.
pub enum ItemKind {
    /// Markdown note target.
    Note,
    /// Non-note asset target.
    Asset,
}

// ==========================================================================
// TrashRequest
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
/// Request payload for moving one item into the configured trash folder.
pub struct TrashRequest {
    /// Caller-supplied target kind hint.
    pub kind: ItemKind,
    /// Item selector resolved by the application facade.
    pub target: TargetSelector,
    /// Optional user-facing reason recorded alongside the trash action.
    pub reason: Option<String>,
}

// ==========================================================================
// RestoreRequest
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
/// Request payload for restoring one trashed item back into the vault.
pub struct RestoreRequest {
    /// Caller-supplied target kind hint.
    pub kind: ItemKind,
    /// Item selector resolved by the application facade.
    pub target: TargetSelector,
    /// Optional alternate destination when the original path is occupied.
    pub alternate_path: Option<RelativePath>,
}

// ==========================================================================
// PurgeScope
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
/// Scope selector for hard-deletion of trashed items.
pub enum PurgeScope {
    /// Purges every item whose retention window has already expired.
    Expired,
    /// Purges one specific item identified by kind and target selector.
    Item {
        /// Caller-supplied target kind hint.
        kind: ItemKind,
        /// Item selector resolved by the application facade.
        target: TargetSelector,
    },
}

// ==========================================================================
// PurgeRequest
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
/// Request payload for purging expired or explicitly selected trashed items.
pub struct PurgeRequest {
    /// Requested purge scope.
    pub scope: PurgeScope,
    /// When `false`, the purge is a dry-run that only reports planned work.
    pub confirm: bool,
}
