//! Shared harness for the authoring-host integration tests.
//!
//! Requests go through the real router, so the middleware stack — guard, origin
//! check, tracing — is exercised rather than bypassed. A test that called
//! handlers directly would prove nothing about whether a route is reachable
//! without a session, which is the property that matters most here.

// Each test binary compiles this module separately, so whatever one file does
// not use looks dead to the compiler.
#![allow(dead_code)]

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
    public::assets::Assets,
    write::{self, AuthConfig},
};

pub const PASSWORD: &str = "correct horse battery staple";
pub const ORIGIN: &str = "https://write.youwin.dev";
pub const PUBLIC_ORIGIN: &str = "https://youwin.dev";

pub fn app(pool: SqlitePool) -> Router {
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
        // A stand-in for the Vite manifest lookup, which needs a real build on
        // disk and has nothing to do with these tests.
        Assets {
            css: "/assets/test.css".to_owned(),
        },
        PUBLIC_ORIGIN.to_owned(),
    )
}

pub struct Reply {
    pub status: StatusCode,
    pub set_cookie: Vec<String>,
    pub retry_after: Option<String>,
    pub body: String,
}

impl Reply {
    /// Parses the body as JSON. Panics with the raw body on failure, which is
    /// far more useful in a test failure than a serde error alone.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("expected JSON, got {:?}: {e}", self.body))
    }
}

pub async fn send(app: &Router, request: Request<Body>) -> Reply {
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

pub fn login_request(password: &str) -> Request<Body> {
    json_request("POST", "/api/auth/login", &format!(r#"{{"password":"{password}"}}"#), None)
}

/// A request carrying a JSON body, and optionally a session cookie.
pub fn json_request(method: &str, uri: &str, body: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, ORIGIN);

    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }

    builder.body(Body::from(body.to_owned())).unwrap()
}

pub fn get(uri: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder.body(Body::empty()).unwrap()
}

pub fn empty_request(method: &str, uri: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::ORIGIN, ORIGIN);

    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }

    builder.body(Body::empty()).unwrap()
}

/// Extracts `name=value` from a Set-Cookie header, for replaying as a Cookie.
pub fn cookie_pair(set_cookie: &str) -> String {
    set_cookie
        .split(';')
        .next()
        .expect("cookie has a value")
        .to_owned()
}

/// Logs in and returns the cookie to replay on subsequent requests.
pub async fn login(app: &Router) -> String {
    let reply = send(app, login_request(PASSWORD)).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    cookie_pair(reply.set_cookie.first().expect("Set-Cookie"))
}

/// Creates a post and returns its public id.
pub async fn create_post(app: &Router, cookie: &str, body: &str, visibility: &str) -> String {
    let reply = send(
        app,
        json_request(
            "POST",
            "/api/posts",
            &format!(r#"{{"body":{},"visibility":"{visibility}"}}"#, json_str(body)),
            Some(cookie),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply.json()["id"].as_str().expect("id").to_owned()
}

pub async fn create_reply(app: &Router, cookie: &str, parent: &str, body: &str) -> String {
    let reply = send(
        app,
        json_request(
            "POST",
            "/api/posts",
            &format!(
                r#"{{"body":{},"parent_id":"{parent}","visibility":"public"}}"#,
                json_str(body)
            ),
            Some(cookie),
        ),
    )
    .await;

    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply.json()["id"].as_str().expect("id").to_owned()
}

/// Escapes a string as a JSON literal, so test bodies can contain quotes and
/// newlines without hand-escaping them.
pub fn json_str(raw: &str) -> String {
    serde_json::Value::String(raw.to_owned()).to_string()
}
