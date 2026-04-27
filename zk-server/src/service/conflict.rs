//! Pure conflict predicates for server-side push acceptance.
//!
//! `base` means "the server revision the client based its edit on". It is
//! `None` only for pure creates. Existing rows paired with `base = None` are
//! duplicate-create conflicts, while missing rows paired with `base = Some(_)`
//! are stale conflicts against a server item that no longer exists.

/// Outcome of one server-side accept-or-conflict decision.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptOutcome {
    /// The change is accepted and should use `next_revision`.
    Accept {
        /// Next server revision to stamp onto the accepted row.
        next_revision: i64,
    },
    /// The client based its change on a stale revision.
    ConflictStale {
        /// Server revision currently known for the item.
        current_revision: i64,
    },
    /// The client attempted to create an item that already exists.
    ConflictDuplicateCreate,
}

/// Returns whether `base` diverges from the server's `current` revision.
#[must_use]
pub fn is_stale_revision(base: &Option<i64>, current: i64) -> bool {
    match base {
        None => false,
        Some(base_revision) => *base_revision != current,
    }
}

/// Returns whether this is the one fresh-create case the server should accept.
#[must_use]
pub fn is_fresh_create(current_row_exists: bool, base: &Option<i64>) -> bool {
    !current_row_exists && base.is_none()
}

/// Returns the next server revision after one accepted mutation.
#[must_use]
pub fn next_server_revision(current: i64) -> i64 {
    debug_assert!(current != i64::MAX, "server revision should never overflow");
    current + 1
}

/// Decides whether one incoming mutation is accepted or conflicts.
#[must_use]
pub fn accept_or_conflict(
    base: &Option<i64>,
    current: i64,
    current_row_exists: bool,
) -> AcceptOutcome {
    if is_fresh_create(current_row_exists, base) {
        return AcceptOutcome::Accept {
            next_revision: next_server_revision(0),
        };
    }

    if !current_row_exists {
        return AcceptOutcome::ConflictStale {
            current_revision: 0,
        };
    }

    if base.is_none() {
        return AcceptOutcome::ConflictDuplicateCreate;
    }

    if is_stale_revision(base, current) {
        return AcceptOutcome::ConflictStale {
            current_revision: current,
        };
    }

    AcceptOutcome::Accept {
        next_revision: next_server_revision(current),
    }
}
