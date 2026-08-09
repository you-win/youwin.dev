//! What actually leaves the machine, and what happens when it cannot.
//!
//! Same reasoning as `tests/purge.rs`: this is code with no caller waiting on
//! it, running at 3am, whose failure mode is silence. So a real HTTP server
//! stands in for the remote and the bytes that arrive are compared against the
//! bytes on disk — because "it uploaded something" and "it uploaded the backup"
//! are different claims, and only one of them is worth having.

use std::sync::{Arc, Mutex};

use axum::{
    Router,
    body::Bytes,
    extract::Path as AxumPath,
    http::{HeaderMap, StatusCode},
    routing::put,
};
use sqlx::SqlitePool;
use tempfile::TempDir;
use tokio::net::TcpListener;
use youwin_server::{
    backup,
    clock::now_millis,
    db::{
        Db,
        posts::{self, Visibility},
    },
    export,
    offsite::Uploader,
    public::view::time_fmt,
};

/// One captured upload.
#[derive(Debug, Clone)]
struct Received {
    name: String,
    content_type: Option<String>,
    authorization: Option<String>,
    body: Vec<u8>,
}

/// A stand-in for whatever answers `PUT` on a loopback port.
///
/// `status` is what it replies with, so the refusal path is exercised against a
/// server that really refuses rather than a mock that pretends to.
async fn stub(status: StatusCode) -> (String, Arc<Mutex<Vec<Received>>>) {
    let received: Arc<Mutex<Vec<Received>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = received.clone();

    let app = Router::new().route(
        "/store/{name}",
        put(move |AxumPath(name): AxumPath<String>, headers: HeaderMap, body: Bytes| {
            let sink = sink.clone();
            async move {
                sink.lock().unwrap().push(Received {
                    name,
                    content_type: header(&headers, "content-type"),
                    authorization: header(&headers, "authorization"),
                    body: body.to_vec(),
                });
                (status, "")
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://{}/store", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (base, received)
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

fn db(pool: SqlitePool) -> Db {
    Db {
        read: pool.clone(),
        write: pool,
    }
}

/// The date both subcommands stamp their off-site names with.
fn today() -> String {
    time_fmt::date(now_millis())
}

async fn a_post(pool: &SqlitePool) {
    posts::insert(
        pool,
        "something worth keeping a copy of",
        None,
        Visibility::Public,
        None,
        1_786_259_199_000,
    )
    .await
    .expect("insert");
}

#[sqlx::test]
async fn the_backup_that_lands_off_site_is_the_file_on_disk(pool: SqlitePool) {
    a_post(&pool).await;

    let (base, received) = stub(StatusCode::CREATED).await;
    let dir = TempDir::new().expect("tempdir");
    let uploader = Uploader::new(Some(&base), Some("Bearer offsite-token"));

    backup::run(&db(pool), dir.path(), &uploader)
        .await
        .expect("backup");

    let seen = received.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "expected exactly one upload: {seen:?}");

    let name = format!("youwin-{}.db", today());
    assert_eq!(seen[0].name, name);
    assert_eq!(seen[0].content_type.as_deref(), Some("application/octet-stream"));
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer offsite-token"));

    // The bytes are the backup, not a truncated stream or the `.part`. A
    // SQLite file starts with a fixed magic string, which is the cheapest way to
    // assert this is a database rather than something shaped like one.
    let on_disk = std::fs::read(dir.path().join(&name)).expect("the local backup");
    assert_eq!(seen[0].body, on_disk, "uploaded bytes differ from the file kept locally");
    assert!(
        seen[0].body.starts_with(b"SQLite format 3\0"),
        "what arrived is not a SQLite database",
    );
}

#[sqlx::test]
async fn the_export_sends_dated_json_and_keeps_the_tree_local(pool: SqlitePool) {
    a_post(&pool).await;

    let (base, received) = stub(StatusCode::OK).await;
    let dir = TempDir::new().expect("tempdir");
    let uploader = Uploader::new(Some(&base), None);

    export::run(&db(pool), dir.path(), &uploader)
        .await
        .expect("export");

    let seen = received.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "only posts.json goes off-site: {seen:?}");

    // Dated off-site, undated locally — the local directory is refreshed in
    // place, which is right for a working export and wrong for the copy that has
    // to survive a bad run.
    assert_eq!(seen[0].name, format!("youwin-{}.json", today()));
    assert_eq!(seen[0].content_type.as_deref(), Some("application/json"));
    // No auth configured, so none is sent — a target that authenticates through
    // the URL must not receive an empty `Authorization`.
    assert_eq!(seen[0].authorization, None);

    let local = std::fs::read(dir.path().join("posts.json")).expect("posts.json");
    assert_eq!(seen[0].body, local);

    // The markdown tree is derivable from that JSON and stays where it is.
    assert!(dir.path().join("markdown").is_dir());
    let uploaded: serde_json::Value = serde_json::from_slice(&seen[0].body).expect("valid JSON");
    assert_eq!(uploaded.as_array().expect("an array").len(), 1);
}

#[sqlx::test]
async fn a_remote_that_refuses_fails_the_backup_but_not_the_local_copy(pool: SqlitePool) {
    a_post(&pool).await;

    let (base, received) = stub(StatusCode::FORBIDDEN).await;
    let dir = TempDir::new().expect("tempdir");
    let uploader = Uploader::new(Some(&base), Some("Bearer expired"));

    let result = backup::run(&db(pool), dir.path(), &uploader).await;

    // Loud, unlike the cache purge. A nightly timer that exits zero having
    // uploaded nothing is the exact failure this feature exists to prevent.
    let error = result.expect_err("a refused upload must fail the subcommand");
    let text = format!("{error:#}");
    assert!(text.contains("403"), "the status should be in the error: {text}");

    assert_eq!(received.lock().unwrap().len(), 1, "it should have tried");

    // And the snapshot is still on disk: the upload happens after the rename, so
    // a remote problem never costs you the local backup as well.
    let local = dir.path().join(format!("youwin-{}.db", today()));
    assert!(local.is_file(), "{} should still exist", local.display());
}

#[sqlx::test]
async fn an_unreachable_remote_fails_rather_than_passing_quietly(pool: SqlitePool) {
    a_post(&pool).await;
    let dir = TempDir::new().expect("tempdir");

    // Nothing is listening on port 9, ever.
    let uploader = Uploader::new(Some("http://127.0.0.1:9/store"), None);
    let result = backup::run(&db(pool), dir.path(), &uploader).await;

    assert!(result.is_err(), "an unreachable remote must not look like success");
    assert!(dir.path().join(format!("youwin-{}.db", today())).is_file());
}

#[sqlx::test]
async fn with_no_url_configured_nothing_is_contacted_and_both_still_work(pool: SqlitePool) {
    a_post(&pool).await;

    let (base, received) = stub(StatusCode::OK).await;
    let dir = TempDir::new().expect("tempdir");
    let uploader = Uploader::new(None, None);
    let db = db(pool);

    // Off is a supported way to run, and it is the default: the local snapshot
    // and the export must be exactly what they were before this existed.
    backup::run(&db, &dir.path().join("backups"), &uploader)
        .await
        .expect("backup");
    export::run(&db, &dir.path().join("export"), &uploader)
        .await
        .expect("export");

    assert!(
        received.lock().unwrap().is_empty(),
        "the stub at {base} should never have been contacted",
    );
    assert!(dir.path().join("backups").join(format!("youwin-{}.db", today())).is_file());
    assert!(dir.path().join("export").join("posts.json").is_file());
}
