//! The guard, and the CSRF origin check.

use axum::{
    extract::{Request, State},
    http::{Method, header},
    middleware::Next,
    response::Response,
};
use axum_extra::extract::cookie::CookieJar;

use crate::{
    auth::session,
    clock::now_millis,
    db::sessions,
    error::AppError,
    write::WriteState,
};

/// Marker placed in request extensions once a session is validated.
///
/// Handlers behind the guard can take `Extension<AuthedSession>` to prove they
/// are on the authenticated side; nothing here needs the session's contents yet.
#[derive(Debug, Clone, Copy)]
pub struct AuthedSession {
    pub created_at: i64,
}

/// Rejects anything without a live session.
///
/// Applied with `route_layer` to a sub-router holding every authenticated route,
/// so authentication is structural rather than a thing each handler remembers to
/// ask for. Adding a route to that sub-router is enough; there is no annotation
/// to forget.
pub async fn require_session(
    State(state): State<WriteState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let jar = CookieJar::from_headers(request.headers());
    let name = session::cookie_name(state.auth.cookie_secure);

    let Some(cookie) = jar.get(name) else {
        return Err(AppError::Unauthorized);
    };

    let token_hash = session::hash_token(cookie.value());
    let now = now_millis();

    let Some(found) = sessions::lookup(&state.db.read, &token_hash, now).await? else {
        // Expired or forged — the caller cannot tell which, and neither can we
        // without leaking whether the token ever existed.
        return Err(AppError::Unauthorized);
    };

    // Slide the window, but only when it is actually stale. Otherwise every
    // authenticated read would become a write.
    if now - found.last_seen_at > session::REFRESH_AFTER_MILLIS {
        sessions::touch(&state.db.write, &token_hash, now, now + session::TTL_MILLIS).await?;
    }

    request.extensions_mut().insert(AuthedSession {
        created_at: found.created_at,
    });

    Ok(next.run(request).await)
}

/// Defence in depth behind `SameSite=Lax`.
///
/// Lax already withholds the cookie from cross-site POST/PATCH/DELETE, which is
/// every mutating route here — so this is a second lock, not the only one.
///
/// A *missing* `Origin` is allowed. Browsers send it on every cross-origin
/// state-changing request, so its absence means a non-browser client (curl, a
/// script, a health check), which carries no ambient cookie to abuse. Rejecting
/// it would break command-line use for no security gain.
pub async fn check_origin(
    State(state): State<WriteState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let is_safe = matches!(request.method(), &Method::GET | &Method::HEAD | &Method::OPTIONS);

    if !is_safe
        && let Some(origin) = request.headers().get(header::ORIGIN)
    {
        let matches = origin
            .to_str()
            .is_ok_and(|value| value == state.auth.origin);

        if !matches {
            tracing::warn!(?origin, expected = %state.auth.origin, "rejected cross-origin write");
            return Err(AppError::Forbidden);
        }
    }

    Ok(next.run(request).await)
}
