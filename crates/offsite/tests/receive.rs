//! What this service does with a request, over a real socket.
//!
//! Same reasoning as the server's `tests/offsite.rs`: this is code with nobody
//! waiting on it, running at 3am, whose failure mode is silence. So the router
//! is served on a loopback port and driven with a real HTTP client, and every
//! assertion is about the *directory* afterwards rather than the status code
//! alone — "it returned 201" and "the backup is on the disk" are different
//! claims, and only one of them is worth having.
//!
//! The refusals get the same treatment in reverse. A refused upload has to leave
//! nothing behind, and the copy that was already there has to still be there.

use std::{net::SocketAddr, path::Path, sync::Arc};

use sqlx::{Connection as _, SqliteConnection, sqlite::SqliteConnectOptions};
use tempfile::TempDir;
use tokio::net::TcpListener;
use youwin_offsite::{config::Config, http};

/// The credential both ends of the real deployment hold.
const AUTH: &str = "Bearer a-shared-secret";

/// A receiver on a loopback port, with its own empty directory.
struct Serving {
    base: String,
    dir: TempDir,
}

impl Serving {
    async fn new(keep: usize, max_bytes: u64) -> Self {
        let dir = TempDir::new().expect("tempdir");

        // Constructed rather than read from the environment: `Config::from_env`
        // is process-global, and these tests run in parallel in one process.
        let cfg = Config {
            bind: "127.0.0.1:0".parse::<SocketAddr>().expect("bind"),
            dir: dir.path().to_owned(),
            auth: AUTH.to_owned(),
            keep,
            max_bytes,
        };

        let listener = TcpListener::bind(cfg.bind).await.expect("listen");
        let base = format!("http://{}", listener.local_addr().expect("addr"));

        let router = http::router(Arc::new(http::Receiver::new(&cfg)));
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        Self { base, dir }
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.dir.path().join(name)
    }

    /// Every file in the directory, sorted. The assertion that matters after a
    /// refusal: nothing new, and nothing missing.
    fn contents(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.dir.path())
            .expect("read_dir")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    async fn put(&self, name: &str, auth: Option<&str>, body: Vec<u8>) -> (u16, String) {
        let mut request = reqwest::Client::new()
            .put(format!("{}/{name}", self.base))
            .header("content-type", "application/octet-stream")
            .body(body);

        if let Some(auth) = auth {
            request = request.header("authorization", auth);
        }

        let response = request.send().await.expect("the receiver should answer");
        let status = response.status().as_u16();
        (status, response.text().await.unwrap_or_default())
    }

    /// The ordinary case: the right credential.
    async fn upload(&self, name: &str, body: Vec<u8>) -> (u16, String) {
        self.put(name, Some(AUTH), body).await
    }
}

/// A real SQLite database with `posts` in it, as bytes on the wire.
///
/// Built with SQLite rather than a fixture, because the thing under test is
/// whether SQLite will open what arrives. `tests/contract.rs` goes further and
/// uses a genuine `VACUUM INTO` snapshot from the sending half.
async fn archive(posts: usize) -> Vec<u8> {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("archive.db");

    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let mut conn = SqliteConnection::connect_with(&options).await.expect("create");

    sqlx::query("CREATE TABLE posts (id INTEGER PRIMARY KEY, body TEXT)")
        .execute(&mut conn)
        .await
        .expect("schema");

    for n in 0..posts {
        sqlx::query("INSERT INTO posts (body) VALUES (?1)")
            .bind(format!("post {n}"))
            .execute(&mut conn)
            .await
            .expect("insert");
    }

    conn.close().await.expect("close");
    std::fs::read(&path).expect("read back")
}

fn export(posts: usize) -> Vec<u8> {
    let entries: Vec<String> = (0..posts).map(|n| format!(r#"{{"id":{n}}}"#)).collect();
    format!("[{}]", entries.join(",")).into_bytes()
}

#[tokio::test]
async fn a_verified_snapshot_lands_under_the_name_it_was_sent_as() {
    let serving = Serving::new(90, 1 << 20).await;
    let body = archive(3).await;

    let (status, text) = serving.upload("youwin-2026-08-09.db", body.clone()).await;

    assert_eq!(status, 201, "{text}");
    // The count is in the response and the journal, because a snapshot that
    // suddenly holds a tenth of the posts is the failure no status code shows.
    assert!(text.contains("3 posts"), "the reply should say what arrived: {text}");

    assert_eq!(serving.contents(), ["youwin-2026-08-09.db"]);
    assert_eq!(
        std::fs::read(serving.path("youwin-2026-08-09.db")).expect("the stored file"),
        body,
        "the bytes on disk differ from the bytes sent",
    );
}

#[tokio::test]
async fn an_export_lands_too_and_is_parsed_first() {
    let serving = Serving::new(90, 1 << 20).await;

    let (status, text) = serving.upload("youwin-2026-08-09.json", export(4)).await;
    assert_eq!(status, 201, "{text}");
    assert_eq!(serving.contents(), ["youwin-2026-08-09.json"]);

    // Truncated mid-flight — the shape of a connection dropped halfway, and the
    // one thing a file server cannot notice.
    let (status, text) = serving
        .upload("youwin-2026-08-10.json", br#"[{"id":0},{"id"#.to_vec())
        .await;
    assert_eq!(status, 422, "{text}");
    assert!(text.contains("not well-formed JSON"), "{text}");
    assert_eq!(
        serving.contents(),
        ["youwin-2026-08-09.json"],
        "a refused export must leave nothing behind, not even a .part",
    );
}

#[tokio::test]
async fn a_corrupt_snapshot_is_refused_and_the_last_good_one_survives() {
    let serving = Serving::new(90, 1 << 20).await;
    let good = archive(5).await;

    assert_eq!(serving.upload("youwin-2026-08-09.db", good.clone()).await.0, 201);

    // The night the backup goes bad. Same name, so a file server would overwrite
    // a working snapshot with this and return 201 — which is exactly the outcome
    // this program exists to prevent.
    let (status, text) = serving
        .upload("youwin-2026-08-09.db", b"not a database at all".to_vec())
        .await;

    assert_eq!(status, 422, "{text}");
    assert!(text.contains("file is not a database"), "{text}");
    assert!(
        text.contains("previous copy is untouched"),
        "the reply should say the old copy is safe: {text}",
    );

    assert_eq!(serving.contents(), ["youwin-2026-08-09.db"]);
    assert_eq!(
        std::fs::read(serving.path("youwin-2026-08-09.db")).expect("still there"),
        good,
        "a refused upload must not have replaced the good snapshot",
    );
}

#[tokio::test]
async fn a_well_formed_database_that_is_not_the_archive_is_refused() {
    let serving = Serving::new(90, 1 << 20).await;

    // Passes `PRAGMA integrity_check` and is somebody else's database. This is
    // the case where "it is valid SQLite" and "it is the backup" come apart.
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("other.db");
    let options = SqliteConnectOptions::new().filename(&path).create_if_missing(true);
    let mut conn = SqliteConnection::connect_with(&options).await.expect("create");
    sqlx::query("CREATE TABLE notes (id INTEGER PRIMARY KEY)")
        .execute(&mut conn)
        .await
        .expect("schema");
    conn.close().await.expect("close");

    let (status, text) = serving
        .upload("youwin-2026-08-09.db", std::fs::read(&path).expect("read"))
        .await;

    assert_eq!(status, 422, "{text}");
    assert!(text.contains("posts"), "the reply should name what was missing: {text}");
    assert!(serving.contents().is_empty());
}

#[tokio::test]
async fn nothing_is_written_without_the_credential() {
    let serving = Serving::new(90, 1 << 20).await;
    let body = archive(1).await;

    for offered in [None, Some("Bearer wrong"), Some(""), Some("Basic YTpi")] {
        let (status, text) = serving.put("youwin-2026-08-09.db", offered, body.clone()).await;
        assert_eq!(status, 401, "{offered:?} must not be accepted: {text}");
        // Terse on purpose: anybody can reach this hostname, and only the sender
        // should learn anything from it — not which names it wants, not whether
        // one already exists.
        assert_eq!(text.trim(), "unauthorized");
    }

    assert!(
        serving.contents().is_empty(),
        "an unauthorized PUT must not create so much as a staging file",
    );
}

#[tokio::test]
async fn only_the_two_names_the_sender_generates_are_written() {
    let serving = Serving::new(90, 1 << 20).await;
    let body = archive(1).await;

    for hostile in [
        "..%2f..%2fetc%2fpasswd",
        "youwin-2026-08-09.db.part",
        "youwin-2026-8-9.db",
        "secrets.env",
        "youwin-2026-13-01.db",
    ] {
        let (status, text) = serving.upload(hostile, body.clone()).await;
        assert_eq!(status, 400, "{hostile:?} should be refused: {text}");
    }

    assert!(serving.contents().is_empty());
}

#[tokio::test]
async fn a_url_with_a_path_on_the_end_says_so() {
    let serving = Serving::new(90, 1 << 20).await;

    // The mistake this message exists for: YOUWIN_OFFSITE_URL set to
    // https://backup.youwin.dev/youwin instead of the bare host. Without the
    // hint it is a bare 404 with no body, on a machine you are not looking at.
    let response = reqwest::Client::new()
        .put(format!("{}/youwin/youwin-2026-08-09.db", serving.base))
        .header("authorization", AUTH)
        .body(archive(1).await)
        .send()
        .await
        .expect("answer");

    assert_eq!(response.status().as_u16(), 400);
    let text = response.text().await.expect("body");
    assert!(
        text.contains("YOUWIN_OFFSITE_URL has no path component"),
        "the reply should name the likely cause: {text}",
    );
    assert!(serving.contents().is_empty());
}

#[tokio::test]
async fn a_body_over_the_limit_is_refused_before_it_is_kept() {
    let serving = Serving::new(90, 1024).await;

    let (status, text) = serving.upload("youwin-2026-08-09.db", archive(200).await).await;

    assert_eq!(status, 413, "{text}");
    assert!(
        serving.contents().is_empty(),
        "an oversized upload must not leave a partial file on the disk it was going to fill",
    );
}

#[tokio::test]
async fn a_retry_on_the_same_day_replaces_the_file() {
    let serving = Serving::new(90, 1 << 20).await;

    // The sender's timer can fire twice, and a failed run gets repeated by hand.
    // Re-sending a name is routine, not exceptional.
    assert_eq!(serving.upload("youwin-2026-08-09.db", archive(1).await).await.0, 201);

    let second = archive(9).await;
    let (status, text) = serving.upload("youwin-2026-08-09.db", second.clone()).await;

    assert_eq!(status, 201, "{text}");
    assert!(text.contains("9 posts"), "{text}");
    assert_eq!(serving.contents(), ["youwin-2026-08-09.db"]);
    assert_eq!(
        std::fs::read(serving.path("youwin-2026-08-09.db")).expect("stored"),
        second,
    );
}

#[tokio::test]
async fn the_name_decides_how_it_is_verified_not_the_content_type() {
    let serving = Serving::new(90, 1 << 20).await;

    // The sender sends application/octet-stream for one and application/json for
    // the other, but a header is a claim and the name is the contract. Sending
    // an archive labelled as JSON must still be verified as a database.
    let response = reqwest::Client::new()
        .put(format!("{}/youwin-2026-08-09.db", serving.base))
        .header("authorization", AUTH)
        .header("content-type", "application/json")
        .body(archive(2).await)
        .send()
        .await
        .expect("answer");

    assert_eq!(response.status().as_u16(), 201);
    assert!(response.text().await.expect("body").contains("2 posts"));
}

#[tokio::test]
async fn retention_sweeps_each_kind_as_its_pair_arrives() {
    let serving = Serving::new(2, 1 << 20).await;

    for day in 1..=4 {
        let date = format!("2026-08-0{day}");
        assert_eq!(
            serving.upload(&format!("youwin-{date}.db"), archive(day).await).await.0,
            201,
        );
        assert_eq!(
            serving.upload(&format!("youwin-{date}.json"), export(day)).await.0,
            201,
        );
    }

    assert_eq!(
        serving.contents(),
        [
            "youwin-2026-08-03.db",
            "youwin-2026-08-03.json",
            "youwin-2026-08-04.db",
            "youwin-2026-08-04.json",
        ],
    );
}

#[tokio::test]
async fn a_hand_placed_file_is_never_swept_up() {
    let serving = Serving::new(1, 1 << 20).await;

    // The copy somebody took before a migration, by hand, on purpose. Retention
    // must be incapable of reaching it — see `store::prune`.
    let kept = serving.path("youwin-before-the-migration.db");
    std::fs::write(&kept, b"mine").expect("write");

    for day in 1..=3 {
        serving
            .upload(&format!("youwin-2026-08-0{day}.db"), archive(day).await)
            .await;
    }

    assert!(Path::new(&kept).is_file(), "a hand-placed file must survive retention");
    assert_eq!(
        serving.contents(),
        ["youwin-2026-08-03.db", "youwin-before-the-migration.db"],
    );
}

#[tokio::test]
async fn nothing_but_put_is_answered() {
    let serving = Serving::new(90, 1 << 20).await;
    let client = reqwest::Client::new();

    // Unreachable in production — `handle { abort }` in the Caddy block closes
    // these before they get here — but a GET that quietly listed the directory,
    // or a DELETE that worked, would be a different service than the one
    // documented. So the shape is pinned even though Caddy is the real gate.
    //
    // 405 rather than the fallback's 400: the name is one this service knows,
    // and the method is the problem. That is what `Allow: PUT` is for, and it is
    // a more useful thing to find in a log than a generic refusal.
    for build in [
        client.get(format!("{}/youwin-2026-08-09.db", serving.base)),
        client.delete(format!("{}/youwin-2026-08-09.db", serving.base)),
        client.post(format!("{}/youwin-2026-08-09.db", serving.base)),
    ] {
        let response = build.header("authorization", AUTH).send().await.expect("answer");
        assert_eq!(response.status().as_u16(), 405);
    }

    // A path this service has no route for at all goes to the fallback, which
    // explains itself.
    let response = client
        .get(serving.base.clone())
        .header("authorization", AUTH)
        .send()
        .await
        .expect("answer");
    assert_eq!(response.status().as_u16(), 400);

    assert!(serving.contents().is_empty());
}
