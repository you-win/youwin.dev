pub mod archive;
pub mod familiar;
pub mod posts;
pub mod search;
pub mod sessions;
pub mod tags;

use std::time::Duration;

use anyhow::{Context as _, Result};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};

use crate::config::Config;

/// Two pools over one SQLite file.
///
/// SQLite permits exactly one writer. WAL lets readers run concurrently with the
/// writer, but two connections attempting to write at once produce SQLITE_BUSY.
/// Rather than retry-loop on that, the writer is a pool of exactly one
/// connection: writers queue in sqlx instead of colliding in SQLite.
#[derive(Clone)]
pub struct Db {
    pub read: SqlitePool,
    pub write: SqlitePool,
}

impl Db {
    pub async fn connect(cfg: &Config) -> Result<Self> {
        let opts = SqliteConnectOptions::new()
            .filename(&cfg.database_path)
            .create_if_missing(true)
            // Persisted in the file itself, so this is idempotent after the first run.
            .journal_mode(SqliteJournalMode::Wal)
            // Safe under WAL: fsync happens per checkpoint rather than per commit.
            .synchronous(SqliteSynchronous::Normal)
            // sqlx defaults this on, unlike raw SQLite. Stated so the intent
            // survives a version bump.
            .foreign_keys(true)
            // Safety net for checkpoint stalls, not a substitute for the
            // single-writer pool above.
            .busy_timeout(Duration::from_secs(5))
            .pragma("temp_store", "MEMORY");

        // Opened first: it runs the migrations, so the file and its -wal/-shm
        // sidecars exist before anything else touches them.
        let write = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts.clone())
            .await
            .with_context(|| format!("opening {}", cfg.database_path.display()))?;

        sqlx::migrate!()
            .run(&write)
            .await
            .context("applying migrations")?;

        // Concurrency for the public site.
        //
        // `query_only` rather than SqliteConnectOptions::read_only: a file-level
        // read-only connection to a WAL database cannot create the -shm file, so
        // it silently depends on a read-write connection already being open.
        // That works until startup order changes. This pragma enforces the same
        // intent at the statement level with no ordering dependency.
        let read = SqlitePoolOptions::new()
            .max_connections(4)
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    sqlx::query("PRAGMA query_only = ON").execute(conn).await?;
                    Ok(())
                })
            })
            .connect_with(opts)
            .await
            .context("opening read pool")?;

        Ok(Self { read, write })
    }

    /// Closes both pools so SQLite checkpoints the WAL on the way out. Without
    /// this a systemd restart can leave a hot journal behind.
    pub async fn close(&self) {
        self.read.close().await;
        self.write.close().await;
    }
}
