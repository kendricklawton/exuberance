//! Persistent stores for exuberance (ROADMAP P8.2).
//!
//! [`SqliteStore`] is the production [`IvStore`] — daily ATM IV persisted
//! to a local SQLite file so IV *rank* is computable across runs, the capability the engine
//! exists to provide. SQLite (vendored via `rusqlite`'s `bundled` feature — no system
//! dependency) rather than a flat file because this same store grows to hold the journal
//! (Phase 23) and a bar cache (Phase 9); doing it once avoids a throwaway.
//!
//! The IV history *seam* and the in-memory impl live in `exub-core`; this crate is only the
//! SQLite adapter, so a lean build (no `sqlite` feature) never compiles SQLite at all.

use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use exub_core::{IvStore, ProviderError, ProviderResult};
use rusqlite::Connection;

/// A SQLite-backed [`IvStore`]. The `Connection` sits behind an `Arc<Mutex<…>>` so the store
/// is `Send + Sync`; each method locks, runs one tiny local query, and drops the guard —
/// never holding it across an `.await` (there are none), so the futures stay `Send`.
#[derive(Clone)]
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    /// Open (creating if absent) a store at `path`, ensuring the schema exists.
    ///
    /// # Errors
    /// [`ProviderError::Transport`] if the database can't be opened or the schema can't be
    /// created.
    pub fn open(path: impl AsRef<Path>) -> ProviderResult<Self> {
        let conn = Connection::open(path)
            .map_err(|e| ProviderError::Transport(format!("opening IV store: {e}")))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS iv_obs (
                 symbol    TEXT    NOT NULL,
                 date_days INTEGER NOT NULL,
                 iv        REAL    NOT NULL,
                 PRIMARY KEY (symbol, date_days)
             );",
        )
        .map_err(|e| ProviderError::Transport(format!("initializing IV store schema: {e}")))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn lock(&self) -> ProviderResult<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| ProviderError::Transport("IV store lock poisoned".into()))
    }
}

#[async_trait]
impl IvStore for SqliteStore {
    async fn record_iv(&self, symbol: &str, date_days: i64, iv: f64) -> ProviderResult<()> {
        let conn = self.lock()?;
        // INSERT OR REPLACE on the (symbol, date_days) PK = idempotent per day.
        conn.execute(
            "INSERT OR REPLACE INTO iv_obs (symbol, date_days, iv) VALUES (?1, ?2, ?3)",
            rusqlite::params![symbol, date_days, iv],
        )
        .map_err(|e| ProviderError::Transport(format!("recording IV: {e}")))?;
        Ok(())
    }

    async fn record_iv_batch(&self, symbol: &str, obs: &[(i64, f64)]) -> ProviderResult<()> {
        let mut conn = self.lock()?;
        let tx = conn
            .transaction()
            .map_err(|e| ProviderError::Transport(format!("begin IV batch: {e}")))?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT OR REPLACE INTO iv_obs (symbol, date_days, iv) VALUES (?1, ?2, ?3)",
                )
                .map_err(|e| ProviderError::Transport(format!("prepare IV batch: {e}")))?;
            for &(date_days, iv) in obs {
                stmt.execute(rusqlite::params![symbol, date_days, iv])
                    .map_err(|e| ProviderError::Transport(format!("IV batch insert: {e}")))?;
            }
        }
        tx.commit()
            .map_err(|e| ProviderError::Transport(format!("commit IV batch: {e}")))?;
        Ok(())
    }

    async fn iv_series(&self, symbol: &str) -> ProviderResult<Vec<(i64, f64)>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT date_days, iv FROM iv_obs WHERE symbol = ?1 ORDER BY date_days ASC")
            .map_err(|e| ProviderError::Transport(format!("prepare IV query: {e}")))?;
        let rows = stmt
            .query_map([symbol], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)))
            .map_err(|e| ProviderError::Transport(format!("querying IV series: {e}")))?;
        let mut series = Vec::new();
        for row in rows {
            series.push(row.map_err(|e| ProviderError::Transport(format!("reading IV row: {e}")))?);
        }
        Ok(series)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn persists_across_reopen_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("iv.db");

        {
            let store = SqliteStore::open(&path).unwrap();
            store.record_iv("AAA", 100, 0.20).await.unwrap();
            store
                .record_iv_batch("AAA", &[(101, 0.25), (102, 0.30)])
                .await
                .unwrap();
            // Same-day re-record overwrites, not duplicates.
            store.record_iv("AAA", 100, 0.22).await.unwrap();
        } // connection dropped — force a real reopen from disk.

        let store = SqliteStore::open(&path).unwrap();
        let series = store.iv_series("AAA").await.unwrap();
        assert_eq!(series, vec![(100, 0.22), (101, 0.25), (102, 0.30)]);
        assert!(store.iv_series("MISSING").await.unwrap().is_empty());
    }
}
