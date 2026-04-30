// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{error::Error as _, io, path::PathBuf};

use zk_index::IndexError;
use zk_sync::{SyncError, SyncResult};
use zk_vault::error::VaultError;

#[test]
fn sync_error_variants_have_expected_display_and_sources() {
    let item_missing = SyncError::ItemMissing("note-1".to_string());
    assert_eq!(
        item_missing.to_string(),
        "item note-1 missing from local state"
    );
    assert!(item_missing.source().is_none());

    let revision_mismatch = SyncError::RevisionMismatch {
        base: Some(7),
        server: 9,
    };
    assert_eq!(
        revision_mismatch.to_string(),
        "base revision Some(7) does not match server revision 9"
    );
    assert!(revision_mismatch.source().is_none());

    let out_of_order = SyncError::OutOfOrderEvent { last: 10, next: 10 };
    assert_eq!(
        out_of_order.to_string(),
        "unexpected event sequence: expected > 10, got 10"
    );
    assert!(out_of_order.source().is_none());

    let ambiguous_bootstrap = SyncError::AmbiguousBootstrap;
    assert_eq!(
        ambiguous_bootstrap.to_string(),
        "initial sync is ambiguous: both local and remote have content"
    );
    assert!(ambiguous_bootstrap.source().is_none());

    let unsupported = SyncError::Unsupported("resolve_conflict");
    assert_eq!(
        unsupported.to_string(),
        "unsupported sync operation: resolve_conflict"
    );
    assert!(unsupported.source().is_none());

    let protocol = SyncError::Protocol("accepted push result missing revision".to_string());
    assert_eq!(
        protocol.to_string(),
        "protocol failure: accepted push result missing revision"
    );
    assert!(protocol.source().is_none());

    let vault_write = SyncError::VaultWrite(VaultError::Io {
        path: PathBuf::from("vault/alpha.md"),
        source: io::Error::other("disk failed"),
    });
    assert_eq!(vault_write.to_string(), "vault write failed");
    assert!(vault_write.source().is_some());

    let index_error = SyncError::Index(IndexError::Io {
        path: PathBuf::from("index.sqlite"),
        source: io::Error::other("database open failed"),
        context: "open index".to_string(),
    });
    assert_eq!(index_error.to_string(), "index operation failed");
    assert!(index_error.source().is_some());

    let transport = SyncError::Transport(Box::new(io::Error::other("connection reset")));
    assert_eq!(transport.to_string(), "transport failure");
    assert!(transport.source().is_some());

    let alias_round_trip: SyncResult<()> = Err(SyncError::AmbiguousBootstrap);
    assert!(matches!(
        alias_round_trip,
        Err(SyncError::AmbiguousBootstrap)
    ));
}
