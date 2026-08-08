//! Login, logout, and the session probe.

use std::time::Duration;

use axum::{
    Extension, Json,
    extract::State,
    http::HeaderMap,
    response::IntoResponse,
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    auth::{self, middleware::AuthedSession, password, session},
    clock::now_millis,
    db::sessions,
    error::AppError,
    write::WriteState,
};

/// Floor on how long a login takes, whatever the outcome.
///
/// argon2 verification is deliberately slow, but *rejecting* can return early —
/// a rate-limit hit or a missing hash costs nothing. Holding every response to
/// the same floor keeps timing from distinguishing those cases.
const MIN_LOGIN_DURATION: Duration = Duration::from_millis(250);

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    password: String,
}

#[derive(Debug, Serialize)]
pub struct MeResponse {
    authenticated: bool,
    session_started: i64,
    active_sessions: i64,
}

pub async fn login(
    State(state): State<WriteState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    let started = tokio::time::Instant::now();
    let ip = auth::client_ip(&headers);
    let now = now_millis();

    let result = attempt(&state, &body.password, &ip, &headers, now).await;

    // Every path through this handler — throttled, wrong password, correct
    // password, database error — leaves through here at the same minimum
    // duration.
    if let Some(remaining) = MIN_LOGIN_DURATION.checked_sub(started.elapsed()) {
        tokio::time::sleep(remaining).await;
    }

    let token = result?;
    let cookie = session::build_cookie(token, state.auth.cookie_secure);

    Ok((jar.add(cookie), Json(json!({ "authenticated": true }))))
}

/// The decision, separated so the timing floor above wraps all of it.
async fn attempt(
    state: &WriteState,
    candidate: &str,
    ip: &str,
    headers: &HeaderMap,
    now: i64,
) -> Result<String, AppError> {
    if let Err(retry_after) = state.limiter.check(ip, now) {
        tracing::warn!(%ip, retry_after, "login attempt while throttled");
        return Err(AppError::TooManyAttempts(retry_after));
    }

    if !password::verify(candidate, &state.auth.password_hash) {
        state.limiter.record_failure(ip, now);
        tracing::warn!(%ip, "failed login");
        return Err(AppError::Unauthorized);
    }

    state.limiter.reset(ip);

    let token = session::new_token();
    sessions::create(
        &state.db.write,
        &token.hash,
        now,
        now + session::TTL_MILLIS,
        auth::user_agent(headers).as_deref(),
        Some(ip),
    )
    .await?;

    tracing::info!(%ip, "login");
    Ok(token.value)
}

pub async fn logout(
    State(state): State<WriteState>,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    // The guard already proved this cookie names a live session; deleting by its
    // hash is what makes the token dead server-side and not merely dropped by
    // one browser.
    if let Some(cookie) = jar.get(session::cookie_name(state.auth.cookie_secure)) {
        sessions::delete(&state.db.write, &session::hash_token(cookie.value())).await?;
    }

    let cleared = session::clearing_cookie(state.auth.cookie_secure);
    Ok((jar.add(cleared), Json(json!({ "authenticated": false }))))
}

pub async fn logout_all(
    State(state): State<WriteState>,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    let removed = sessions::delete_all(&state.db.write).await?;
    tracing::info!(removed, "logged out everywhere");

    let cleared = session::clearing_cookie(state.auth.cookie_secure);
    Ok((
        jar.add(cleared),
        Json(json!({ "authenticated": false, "sessions_ended": removed })),
    ))
}

/// The SPA's auth probe: 401 when unauthenticated, courtesy of the guard.
pub async fn me(
    State(state): State<WriteState>,
    Extension(session): Extension<AuthedSession>,
) -> Result<Json<MeResponse>, AppError> {
    let active_sessions = sessions::count_active(&state.db.read, now_millis()).await?;

    Ok(Json(MeResponse {
        authenticated: true,
        session_started: session.created_at,
        active_sessions,
    }))
}
