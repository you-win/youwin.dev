//! The two halves, driven against each other.
//!
//! This is the reason the receiver shares a workspace with the site rather than
//! living in a repo of its own. Everything between them is a wire contract —
//! a name, a header, and a body — and a contract described in prose in two
//! places is one that drifts. Here the *real* [`Uploader`] runs against the
//! *real* router: a genuine `VACUUM INTO` snapshot of a genuine WAL database,
//! sent by the code that sends it in production, verified by the code that
//! verifies it in production.
//!
//! Concretely, the thing this catches that no unit test can: `verify::database`
//! opens arrivals `immutable`, and the file it is opening is whatever
//! `VACUUM INTO` happens to produce. If a SQLite upgrade ever changed that
//! output such that it could not be opened that way, every nightly backup would
//! start being refused — and it would be discovered here, on a push, rather than
//! at 3am by a timer.

use std::{fs, net::SocketAddr, path::Path, sync::Arc};

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use tempfile::TempDir;
use tokio::net::TcpListener;
use youwin_offsite::{config::Config, http};
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

const AUTH: &str = "Bearer the-same-string-on-both-boxes";

/// A receiver, and the directory it will write into.
async fn receiver() -> (String, TempDir) {
    let dir = TempDir::new().expect("tempdir");

    let cfg = Config {
        bind: "127.0.0.1:0".parse::<SocketAddr>().expect("bind"),
        dir: dir.path().to_owned(),
        auth: AUTH.to_owned(),
        keep: 90,
        max_bytes: 1 << 30,
    };

    let listener = TcpListener::bind(cfg.bind).await.expect("listen");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let router = http::router(Arc::new(http::Receiver::new(&cfg)));

    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    (base, dir)
}

/// A live WAL database with the site's real schema in it.
///
/// WAL specifically, because that is the whole reason `backup` uses
/// `VACUUM INTO` rather than copying the file — and therefore the state the
/// receiver's verifier has to be able to read the output of.
async fn live_archive(path: &Path) -> Db {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("opening the archive");

    // The server's own migrations, resolved at a path rather than through
    // `sqlx::migrate!()`. That macro looks in the crate it is compiled in, which
    // here would be this one; `CARGO_MANIFEST_DIR` makes the reach across the
    // workspace absolute rather than dependent on the working directory.
    let migrations = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../server/migrations"));
    sqlx::migrate::Migrator::new(migrations)
        .await
        .expect("reading the server's migrations")
        .run(&pool)
        .await
        .expect("applying the server's migrations");

    Db {
        read: pool.clone(),
        write: pool,
    }
}

/// The date both subcommands stamp their off-site names with.
fn today() -> String {
    time_fmt::date(now_millis())
}

#[tokio::test]
async fn a_real_nightly_run_arrives_and_is_accepted() {
    let sending = TempDir::new().expect("tempdir");
    let db = live_archive(&sending.path().join("youwin.db")).await;

    posts::insert(
        &db.write,
        "something worth keeping a copy of",
        None,
        Visibility::Public,
        None,
        1_786_259_199_000,
    )
    .await
    .expect("insert");

    let (base, received) = receiver().await;
    let uploader = Uploader::new(Some(&base), Some(AUTH));

    let backups = sending.path().join("backups");
    let exports = sending.path().join("export");

    // Exactly what youwin-backup.service runs, in the same order.
    backup::run(&db, &backups, &uploader).await.expect("backup");
    export::run(&db, &exports, &uploader).await.expect("export");

    let date = today();
    let snapshot = received.path().join(format!("youwin-{date}.db"));
    let exported = received.path().join(format!("youwin-{date}.json"));

    assert!(
        snapshot.is_file(),
        "a genuine VACUUM INTO snapshot was refused by the receiver's verifier",
    );
    assert!(exported.is_file(), "the dated export did not land");

    // Byte-identical to what the sending box kept locally. "It uploaded
    // something" and "it uploaded the backup" are different claims.
    assert_eq!(
        fs::read(&snapshot).expect("the received snapshot"),
        fs::read(backups.join(format!("youwin-{date}.db"))).expect("the local snapshot"),
    );
    assert_eq!(
        fs::read(&exported).expect("the received export"),
        fs::read(exports.join("posts.json")).expect("the local posts.json"),
    );

    // The markdown tree is derivable from that JSON and deliberately stays on
    // the sending box — so the receiver must have got two files, not three.
    let mut landed: Vec<String> = fs::read_dir(received.path())
        .expect("read_dir")
        .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    landed.sort();
    assert_eq!(landed, [format!("youwin-{date}.db"), format!("youwin-{date}.json")]);

    db.close().await;
}

#[tokio::test]
async fn a_wrong_credential_fails_the_sending_unit_and_writes_nothing() {
    let sending = TempDir::new().expect("tempdir");
    let db = live_archive(&sending.path().join("youwin.db")).await;

    let (base, received) = receiver().await;
    let uploader = Uploader::new(Some(&base), Some("Bearer rotated-six-weeks-ago"));

    let backups = sending.path().join("backups");
    let result = backup::run(&db, &backups, &uploader).await;

    // Loud on both ends: the receiver refuses, and the sender's subcommand exits
    // non-zero so its unit goes to `failed` rather than quietly uploading
    // nothing. A nightly timer that exits zero having sent nothing looks exactly
    // like one that worked.
    let error = result.expect_err("a refused upload must fail the subcommand");
    let text = format!("{error:#}");
    assert!(text.contains("401"), "the status should reach the sender: {text}");

    assert_eq!(
        fs::read_dir(received.path()).expect("read_dir").count(),
        0,
        "an unauthorized upload must leave nothing on the receiving box",
    );

    // And the local snapshot is still there — the upload happens after the
    // rename, so a receiver problem never costs the sending box its own copy.
    assert!(backups.join(format!("youwin-{}.db", today())).is_file());

    db.close().await;
}
