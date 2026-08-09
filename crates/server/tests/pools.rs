//! The read/write pool split is the structural claim M0 exists to establish:
//! the public surface holds a pool that *cannot* write, so "the public site
//! mutated the database" is unreachable rather than merely unlikely.
//!
//! These tests assert that property directly, plus the two connection settings
//! that would silently degrade if a future refactor dropped them.

use std::{net::SocketAddr, path::PathBuf};

use youwin_server::{config::Config, db::Db};

fn test_config(database_path: PathBuf) -> Config {
    // Port 0 throughout — nothing here binds a listener, the pools are the
    // subject.
    let unused: SocketAddr = "127.0.0.1:0".parse().unwrap();
    Config {
        database_path,
        // Never read here — nothing in this file builds a router.
        public_dist: PathBuf::from("web/dist/public"),
        public_bind: unused,
        write_bind: unused,
        public_origin: "http://localhost".to_owned(),
        write_origin: "http://localhost".to_owned(),
        password_hash: None,
        cookie_secure: false,
        cf_zone_id: None,
        cf_purge_token: None,
        cf_api_base: String::new(),
        offsite_url: None,
        offsite_auth: None,
    }
}

const INSERT: &str = "INSERT INTO posts
      (public_id, root_id, body, body_html, body_text, created_at, updated_at)
      VALUES (?1, 1, 'hello', '<p>hello</p>', 'hello', 1700000000000, 1700000000000)";

#[tokio::test]
async fn read_pool_refuses_writes_while_write_pool_accepts_them() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::connect(&test_config(dir.path().join("test.db")))
        .await
        .expect("connect");

    // The writer works — which also proves migrations ran, since `posts` only
    // exists because 0001_init.sql was applied.
    sqlx::query(INSERT)
        .bind("aaaaaaaaaaaaaaaa")
        .execute(&db.write)
        .await
        .expect("write pool must accept writes");

    // The reader sees the committed row. Under WAL this is a separate connection
    // reading concurrently with the writer, not the same handle.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM posts")
        .fetch_one(&db.read)
        .await
        .expect("read pool must read");
    assert_eq!(count, 1);

    // …but must refuse to write. This is the guard, enforced by
    // `PRAGMA query_only` in the read pool's after_connect hook.
    let err = sqlx::query(INSERT)
        .bind("bbbbbbbbbbbbbbbb")
        .execute(&db.read)
        .await
        .expect_err("read pool must reject writes");

    let message = err.to_string().to_lowercase();
    assert!(
        message.contains("readonly") || message.contains("read-only"),
        "expected a readonly error from query_only, got: {err}"
    );

    db.close().await;
}

#[tokio::test]
async fn journal_mode_is_wal_and_foreign_keys_are_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::connect(&test_config(dir.path().join("test.db")))
        .await
        .expect("connect");

    // WAL is what lets the read pool run concurrently with the writer. If this
    // ever regressed to the default rollback journal, readers would block on
    // every commit and the split above would stop buying anything.
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&db.read)
        .await
        .unwrap();
    assert_eq!(mode.to_lowercase(), "wal");

    // SQLite disables foreign keys by default; ON DELETE CASCADE on posts.parent_id
    // is silently inert without this.
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&db.write)
        .await
        .unwrap();
    assert_eq!(foreign_keys, 1);

    db.close().await;
}
