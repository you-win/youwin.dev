//! Proving that what arrived is a backup, before agreeing that it is one.
//!
//! This is the module that justifies the program. Anything can store bytes; the
//! question a backup has to answer is "would this restore?", and the only
//! honest time to ask it is while the sender is still on the line and can be
//! told no.
//!
//! Both checks run against the staging file, never the file in place. A bad
//! upload therefore costs nothing at all — it is deleted, the request is
//! refused, and yesterday's good copy is still sitting exactly where it was.

use std::{
    io::BufRead as _,
    path::Path,
};

use anyhow::{Context as _, Result, bail};
use sqlx::{Connection as _, SqliteConnection, sqlite::SqliteConnectOptions};

/// Opens an arriving snapshot and asks SQLite whether it is intact.
///
/// Returns the number of posts in it, which is not a pass/fail condition — an
/// archive with nothing in it is a legitimate first day — but is the single most
/// useful number to have in the journal. A count that drops by half overnight is
/// visible in `journalctl` without anything having to decide what that means.
///
/// `PRAGMA integrity_check` walks every page and every index. On a personal
/// archive that is milliseconds, and it catches the failures a checksum would
/// not: a snapshot taken from a database that was already damaged, or one
/// truncated by a disk that filled up mid-`VACUUM`.
pub async fn database(path: &Path) -> Result<i64> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        // Never create. Without this a path that somehow does not exist becomes
        // a brand new empty database that passes every check below.
        .create_if_missing(false)
        // `immutable` tells SQLite the file cannot change underneath it, which
        // skips locking and the -wal/-shm sidecars entirely. That matters here:
        // a plain read-only open of a WAL database cannot create -shm and fails,
        // and a read-write open would litter the backup directory with sidecars
        // that `prune` is deliberately unable to clean up.
        .read_only(true)
        .immutable(true);

    let mut conn = SqliteConnection::connect_with(&options)
        .await
        .with_context(|| format!("opening {} as a SQLite database", path.display()))?;

    let verdict = inspect(&mut conn).await;

    // Closed explicitly, and on the failure path too. The caller deletes this
    // file the instant the verdict is an error, and dropping a connection only
    // *schedules* the close — which on Windows leaves a handle open long enough
    // for the delete to fail, and the staging file to survive a refusal it was
    // supposed to be erased by. Deterministic here beats correct-on-Linux-only.
    let _ = conn.close().await;

    verdict
}

async fn inspect(conn: &mut SqliteConnection) -> Result<i64> {
    // One row saying "ok", or up to 100 rows describing what is wrong.
    let report: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_all(&mut *conn)
        .await
        .context("running PRAGMA integrity_check")?;

    if report != ["ok"] {
        // Truncated: the full report can be a hundred lines and this ends up in
        // an HTTP response body on another machine's journal.
        let detail = report.join("; ");
        let detail = detail.chars().take(500).collect::<String>();
        bail!("SQLite reports the database is damaged: {detail}");
    }

    // Well-formed SQLite is not the same claim as "this is the archive". A
    // pristine empty database passes integrity_check; so does somebody else's.
    // `posts` is the one table this archive has always had.
    let posts: i64 = sqlx::query_scalar("SELECT count(*) FROM posts")
        .fetch_one(conn)
        .await
        .context(
            "it is a valid SQLite database but has no `posts` table, so it is not this archive",
        )?;

    Ok(posts)
}

/// Parses an arriving export.
///
/// Deliberately does not deserialize into a post type. The claim worth making
/// here is the one that makes the file useful in ten years — that it is
/// well-formed JSON and it is the array `posts.json` has always been — not that
/// it matches today's struct. A receiver that rejected a backup because the
/// sender grew a field would be worse than no receiver.
///
/// Validated through a reader with [`serde_json::de::IgnoredAny`] rather than
/// into a `Value`, so a large export is checked without being built in memory.
///
/// Synchronous: call it from `spawn_blocking`.
pub fn export(path: &Path) -> Result<()> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let mut reader = std::io::BufReader::new(file);

    // Peeked rather than consumed, so the parse below still sees the whole file.
    let first = reader.fill_buf().context("reading the export")?.first().copied();

    match first {
        Some(b'[') => {}
        Some(byte) => bail!(
            "expected a JSON array of posts, but it starts with {:?}",
            char::from(byte),
        ),
        None => bail!("it is empty"),
    }

    serde_json::from_reader::<_, serde::de::IgnoredAny>(reader)
        .context("it is not well-formed JSON")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    fn write(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.path().join(name);
        let mut file = std::fs::File::create(&path).expect("create");
        file.write_all(bytes).expect("write");
        path
    }

    #[test]
    fn an_export_must_be_a_well_formed_json_array() {
        let dir = tempfile::TempDir::new().expect("tempdir");

        export(&write(&dir, "ok.json", br#"[{"id":1},{"id":2}]"#)).expect("a real export");
        // Empty is legitimate: an archive on its first day.
        export(&write(&dir, "empty.json", b"[]")).expect("an empty archive");

        // Truncated mid-upload is the failure this exists for. It looks like an
        // array right up until it does not.
        export(&write(&dir, "cut.json", br#"[{"id":1},{"id"#))
            .expect_err("a truncated export must be refused");

        // An object is well-formed JSON and still not what posts.json is.
        export(&write(&dir, "object.json", br#"{"posts":[]}"#))
            .expect_err("an object is not the export");

        export(&write(&dir, "empty-file.json", b"")).expect_err("an empty file");
        export(&write(&dir, "html.json", b"<html>502 Bad Gateway</html>"))
            .expect_err("a proxy error page is not an export");
    }

    #[tokio::test]
    async fn a_file_that_is_not_sqlite_is_refused() {
        let dir = tempfile::TempDir::new().expect("tempdir");

        let error = database(&write(&dir, "not.db", b"40 megabytes of the wrong thing"))
            .await
            .expect_err("must be refused");
        // SQLite's own words, and they end up in the 422 body on the sending
        // box's journal. Asserted because that sentence is the entire diagnostic
        // somebody gets at the other end.
        assert!(
            format!("{error:#}").contains("file is not a database"),
            "the error should say what was wrong: {error:#}",
        );

        // Zero bytes is what a disk that filled up mid-upload leaves behind, and
        // SQLite treats an empty file as a valid empty database — so this is the
        // case the `posts` table check exists to catch, not integrity_check.
        database(&write(&dir, "empty.db", b""))
            .await
            .expect_err("an empty file is not the archive");
    }

    #[tokio::test]
    async fn a_valid_database_without_the_archive_in_it_is_refused() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("someone-elses.db");

        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let mut conn = SqliteConnection::connect_with(&options).await.expect("create");
        sqlx::query("CREATE TABLE notes (id INTEGER PRIMARY KEY)")
            .execute(&mut conn)
            .await
            .expect("create table");
        conn.close().await.expect("close");

        // Passes integrity_check, and is still not the thing being backed up.
        let error = database(&path).await.expect_err("must be refused");
        assert!(
            format!("{error:#}").contains("posts"),
            "the error should name what was missing: {error:#}",
        );
    }

    #[tokio::test]
    async fn a_real_archive_passes_and_reports_its_size() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("archive.db");

        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let mut conn = SqliteConnection::connect_with(&options).await.expect("create");
        sqlx::query("CREATE TABLE posts (id INTEGER PRIMARY KEY, body TEXT)")
            .execute(&mut conn)
            .await
            .expect("create table");
        sqlx::query("INSERT INTO posts (body) VALUES ('one'), ('two'), ('three')")
            .execute(&mut conn)
            .await
            .expect("insert");
        conn.close().await.expect("close");

        assert_eq!(database(&path).await.expect("a valid archive"), 3);
    }
}
