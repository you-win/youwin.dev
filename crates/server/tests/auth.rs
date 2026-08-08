//! End-to-end tests for the authoring host's auth surface, driven through the
//! real router so the middleware stack is exercised rather than mocked.
//!
//! The claim under test is structural: every route on `write.youwin.dev` except
//! login and health is unreachable without a live session. A test that only
//! called handlers directly would prove nothing about that.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use sqlx::SqlitePool;
use tower::ServiceExt as _;
use youwin_server::{
    auth::password,
    db::Db,
    write::{self, AuthConfig},
};

const PASSWORD: &str = "correct horse battery staple";
const ORIGIN: &str = "https://write.youwin.dev";

fn app(pool: SqlitePool) -> Router {
    // Auth does not exercise the read/write split — `tests/pools.rs` covers that
    // — so one pool stands in for both here.
    let db = Db {
        read: pool.clone(),
        write: pool,
    };

    write::router(
        db,
        AuthConfig {
            password_hash: password::hash(PASSWORD).expect("hash"),
            cookie_secure: false,
            origin: ORIGIN.to_owned(),
        },
    )
}

struct Reply {
    status: StatusCode,
    set_cookie: Vec<String>,
    retry_after: Option<String>,
    body: String,
}

async fn send(app: &Router, request: Request<Body>) -> Reply {
    let response = app.clone().oneshot(request).await.expect("router");

    let status = response.status();
    let set_cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(str::to_owned)
        .collect();
    let retry_after = response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");

    Reply {
        status,
        set_cookie,
        retry_after,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn login_request(password: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, ORIGIN)
        .body(Body::from(format!(r#"{{"password":"{password}"}}"#)))
        .unwrap()
}

fn get_with_cookie(uri: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder.body(Body::empty()).unwrap()
}

/// Extracts `name=value` from a Set-Cookie header, for replaying as a Cookie.
fn cookie_pair(set_cookie: &str) -> String {
    set_cookie
        .split(';')
        .next()
        .expect("cookie has a value")
        .to_owned()
}

async fn login(app: &Router) -> String {
    let reply = send(app, login_request(PASSWORD)).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    cookie_pair(reply.set_cookie.first().expect("Set-Cookie"))
}

#[sqlx::test]
async fn guarded_routes_are_unreachable_without_a_session(pool: SqlitePool) {
    let app = app(pool);

    for uri in ["/api/auth/me"] {
        let reply = send(&app, get_with_cookie(uri, None)).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{uri} must be guarded");
        assert!(reply.body.contains("unauthorized"), "{}", reply.body);
    }

    // Health is deliberately open — it is how systemd and Caddy check liveness.
    let reply = send(&app, get_with_cookie("/api/health", None)).await;
    assert_eq!(reply.status, StatusCode::OK);
}

#[sqlx::test]
async fn an_unknown_path_404s_rather_than_401ing(pool: SqlitePool) {
    // route_layer, not layer: the guard must not run for unmatched paths, or a
    // 401 would confirm which routes exist to anyone probing.
    let reply = send(&app(pool), get_with_cookie("/api/nope", None)).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn a_wrong_password_is_rejected_and_issues_no_cookie(pool: SqlitePool) {
    let reply = send(&app(pool), login_request("wrong")).await;

    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(reply.set_cookie.is_empty(), "{:?}", reply.set_cookie);
}

#[sqlx::test]
async fn a_correct_password_issues_a_usable_session_cookie(pool: SqlitePool) {
    let app = app(pool);
    let reply = send(&app, login_request(PASSWORD)).await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let raw = reply.set_cookie.first().expect("Set-Cookie");

    // cookie_secure is false in this harness, so the __Host- prefix is dropped
    // with Secure — the attributes that do not depend on that must still hold.
    assert!(raw.contains("yw_session="), "{raw}");
    assert!(raw.contains("HttpOnly"), "{raw}");
    assert!(raw.contains("SameSite=Lax"), "{raw}");
    assert!(raw.contains("Path=/"), "{raw}");
    assert!(!raw.contains("Domain"), "a Domain attribute would void __Host-: {raw}");

    let me = send(&app, get_with_cookie("/api/auth/me", Some(&cookie_pair(raw)))).await;
    assert_eq!(me.status, StatusCode::OK, "{}", me.body);
    assert!(me.body.contains(r#""authenticated":true"#), "{}", me.body);
}

#[sqlx::test]
async fn a_forged_cookie_does_not_authenticate(pool: SqlitePool) {
    let app = app(pool);

    for forged in [
        "yw_session=",
        "yw_session=not-a-real-token",
        "yw_session=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    ] {
        let reply = send(&app, get_with_cookie("/api/auth/me", Some(forged))).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{forged} authenticated!");
    }
}

#[sqlx::test]
async fn the_token_is_never_stored_in_plaintext(pool: SqlitePool) {
    let app = app(pool.clone());
    let cookie = login(&app).await;
    let token = cookie.split_once('=').expect("name=value").1;

    // A database leak must not hand over live sessions.
    let stored: Vec<Vec<u8>> = sqlx::query_scalar("SELECT token_hash FROM sessions")
        .fetch_all(&pool)
        .await
        .unwrap();

    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].len(), 32, "SHA-256 digest");
    assert!(
        !stored[0].windows(token.len().min(32)).any(|w| w == &token.as_bytes()[..w.len()]),
        "the raw token appears in the sessions table"
    );
}

#[sqlx::test]
async fn logout_kills_the_session_server_side(pool: SqlitePool) {
    let app = app(pool);
    let cookie = login(&app).await;

    let logout = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/api/auth/logout")
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(logout.status, StatusCode::OK, "{}", logout.body);

    let cleared = logout.set_cookie.first().expect("clearing Set-Cookie");
    assert!(cleared.contains("Max-Age=0"), "{cleared}");

    // The real assertion: replaying the same cookie must fail. Clearing it
    // browser-side would be worthless if the token still worked.
    let replay = send(&app, get_with_cookie("/api/auth/me", Some(&cookie))).await;
    assert_eq!(replay.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn logout_all_ends_every_session_not_just_this_one(pool: SqlitePool) {
    let app = app(pool);
    let phone = login(&app).await;
    let laptop = login(&app).await;
    assert_ne!(phone, laptop);

    let reply = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/api/auth/logout-all")
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, &laptop)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);

    // The point of the lever: the *other* device is logged out too.
    for cookie in [&phone, &laptop] {
        let replay = send(&app, get_with_cookie("/api/auth/me", Some(cookie))).await;
        assert_eq!(replay.status, StatusCode::UNAUTHORIZED, "{cookie} survived");
    }
}

#[sqlx::test]
async fn an_expired_session_is_rejected(pool: SqlitePool) {
    let app = app(pool.clone());
    let cookie = login(&app).await;

    sqlx::query("UPDATE sessions SET expires_at = 1")
        .execute(&pool)
        .await
        .unwrap();

    let reply = send(&app, get_with_cookie("/api/auth/me", Some(&cookie))).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn repeated_failures_are_throttled_with_a_retry_after(pool: SqlitePool) {
    let app = app(pool);

    // Five failures are allowed; the sixth attempt is refused outright.
    for attempt in 0..5 {
        let reply = send(&app, login_request("wrong")).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "attempt {attempt}");
    }

    let throttled = send(&app, login_request("wrong")).await;
    assert_eq!(throttled.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(throttled.retry_after.is_some(), "Retry-After must be set");

    // Crucially, the *correct* password is refused too — otherwise throttling
    // would not slow a guessing run down at all.
    let correct = send(&app, login_request(PASSWORD)).await;
    assert_eq!(correct.status, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test]
async fn cross_origin_writes_are_refused(pool: SqlitePool) {
    let app = app(pool);

    let hostile = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/api/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, "https://evil.example")
            .body(Body::from(format!(r#"{{"password":"{PASSWORD}"}}"#)))
            .unwrap(),
    )
    .await;
    assert_eq!(hostile.status, StatusCode::FORBIDDEN);
    assert!(hostile.set_cookie.is_empty());

    // A missing Origin is allowed on purpose: browsers always send it on
    // cross-origin writes, so its absence means a non-browser client with no
    // ambient cookie to abuse. Rejecting it would break curl for no gain.
    let cli = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/api/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(format!(r#"{{"password":"{PASSWORD}"}}"#)))
            .unwrap(),
    )
    .await;
    assert_eq!(cli.status, StatusCode::OK, "{}", cli.body);
}
