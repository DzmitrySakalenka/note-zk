//! `SQLite` connection lifecycle helpers for the server database.

use std::{
    io,
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{Connection, Transaction, TransactionBehavior};

use crate::{ServerError, ServerResult, migration};

/// Shared server database handle.
///
/// Phase 1 uses a single mutex-protected connection because Milestone C
/// workloads do not justify a pool yet.
/// TODO: switch from `Mutex<Connection>` to a connection pool when concurrent
/// reads become a bottleneck; tracked under TG18 operational hardening. See
/// `.tasks/.local/progress.md` lines 1376-1383 for the V1 trade-off.
pub struct ServerDb {
    conn: Mutex<Connection>,
}

impl ServerDb {
    /// Opens or creates a file-backed server database, configures connection
    /// pragmas, and ensures the schema bootstrap table exists.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when `SQLite` cannot open the file, configure
    /// the connection, or initialize schema bookkeeping.
    pub fn open(path: &Path) -> ServerResult<Self> {
        let connection = Connection::open(path).map_err(ServerError::Database)?;
        initialize_connection(&connection)?;

        Ok(Self {
            conn: Mutex::new(connection),
        })
    }

    /// Opens an in-memory server database with the same bootstrap path as
    /// file-backed databases.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when the in-memory connection cannot be opened,
    /// configured, or migrated.
    pub fn open_in_memory() -> ServerResult<Self> {
        let connection = Connection::open_in_memory().map_err(ServerError::Database)?;
        initialize_connection(&connection)?;

        Ok(Self {
            conn: Mutex::new(connection),
        })
    }

    /// Borrows the shared connection for a single closure invocation.
    ///
    /// # Errors
    ///
    /// Returns the closure error, or [`ServerError::Internal`] if the mutex is
    /// poisoned.
    pub fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> ServerResult<R>) -> ServerResult<R> {
        let connection = self.lock_connection()?;
        f(&connection)
    }

    /// Mutably borrows the shared connection for a single closure invocation.
    ///
    /// # Errors
    ///
    /// Returns the closure error, or [`ServerError::Internal`] if the mutex is
    /// poisoned.
    pub fn with_conn_mut<R>(
        &self,
        f: impl FnOnce(&mut Connection) -> ServerResult<R>,
    ) -> ServerResult<R> {
        let mut connection = self.lock_connection()?;
        f(&mut connection)
    }

    /// Runs a closure inside a deferred `SQLite` transaction.
    ///
    /// Successful closures commit their changes. Error returns rely on
    /// `rusqlite`'s default rollback-on-drop behavior.
    ///
    /// # Errors
    ///
    /// Returns the closure error, a `SQLite` transaction error, or
    /// [`ServerError::Internal`] if the mutex is poisoned.
    pub fn with_tx<R>(
        &self,
        f: impl FnOnce(&Transaction<'_>) -> ServerResult<R>,
    ) -> ServerResult<R> {
        let mut connection = self.lock_connection()?;
        let transaction = Transaction::new(&mut connection, TransactionBehavior::Deferred)
            .map_err(ServerError::Database)?;
        let result = f(&transaction)?;
        transaction.commit().map_err(ServerError::Database)?;

        Ok(result)
    }

    /// Returns the current server schema version.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when the version row cannot be read.
    pub fn schema_version(&self) -> ServerResult<i64> {
        self.with_conn(|connection| {
            connection
                .query_row(
                    "SELECT version FROM server_schema_version WHERE id = 1",
                    [],
                    |row| row.get(0),
                )
                .map_err(ServerError::Database)
        })
    }

    fn lock_connection(&self) -> ServerResult<MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| poisoned_connection_error())
    }
}

fn initialize_connection(connection: &Connection) -> ServerResult<()> {
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(ServerError::Database)?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(ServerError::Database)?;
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .map_err(ServerError::Database)?;
    migration::ensure_schema(connection)?;

    Ok(())
}

fn poisoned_connection_error() -> ServerError {
    ServerError::Internal(Box::new(io::Error::other("server database mutex poisoned")))
}
