//! The authoring API at `write.youwin.dev`.
//!
//! JSON only, plus the one authenticated HTML route (`/preview/:id`, M3) that
//! renders through the public templates. Caddy serves the SPA shell off disk, so
//! nothing here returns the app's HTML.

pub mod routes;

use std::sync::Arc;

use axum::{
    Router,
    extract::State,
    middleware,
    routing::{get, post},
};
use tower_http::trace::TraceLayer;

use crate::{
    auth::{
        middleware::{check_origin, require_session},
        ratelimit::LoginLimiter,
    },
    cache::Purger,
    db::Db,
    error::AppError,
    public::assets::Assets,
};

/// Auth settings, resolved once at startup.
pub struct AuthConfig {
    /// argon2id PHC string. Validated at startup — a malformed value should
    /// stop the process, not surface as a login that can never succeed.
    pub password_hash: String,
    pub cookie_secure: bool,
    /// Exact origin allowed to make state-changing requests.
    pub origin: String,
}

/// State for the authoring listener: both pools, because session validation
/// reads and refreshing `last_seen_at` writes.
#[derive(Clone)]
pub struct WriteState {
    pub db: Db,
    pub auth: Arc<AuthConfig>,
    pub limiter: Arc<LoginLimiter>,
    /// The public site's stylesheet, for `/preview` — which renders through the
    /// public templates and must therefore look exactly like the public site.
    pub assets: Assets,
    /// `https://youwin.dev`. The preview's canonical link and og:url point at
    /// where the post will live, not at this host.
    pub public_origin: String,
    /// Invalidates the public site's edge cache after a write. Disabled unless
    /// a Cloudflare zone and purge token are configured.
    pub purger: Arc<Purger>,
}

pub fn router(
    db: Db,
    auth: AuthConfig,
    assets: Assets,
    public_origin: String,
    purger: Purger,
) -> Router {
    let state = WriteState {
        db,
        auth: Arc::new(auth),
        limiter: Arc::new(LoginLimiter::default()),
        assets,
        public_origin,
        purger: Arc::new(purger),
    };

    // Authenticated by default. Everything reachable on this host lives in this
    // sub-router behind the guard, so adding a route is enough — there is no
    // per-handler annotation to forget, and forgetting one cannot leave a route
    // open.
    let guarded = Router::new()
        .route("/api/auth/me", get(routes::auth::me))
        .route("/api/auth/logout", post(routes::auth::logout))
        .route("/api/auth/logout-all", post(routes::auth::logout_all))
        .route(
            "/api/posts",
            get(routes::posts::feed).post(routes::posts::create),
        )
        .route(
            "/api/posts/{public_id}",
            get(routes::posts::show)
                .patch(routes::posts::update)
                .delete(routes::posts::destroy),
        )
        .route("/api/drafts", get(routes::posts::drafts))
        .route("/api/search", get(routes::posts::search))
        // HTML, not JSON — the one exception on this host. Rendered through the
        // public templates so a preview cannot drift from the published page.
        .route("/preview/{public_id}", get(routes::preview::show))
        // route_layer, not layer: this runs only on matched routes, so an
        // unknown path 404s instead of 401ing and confirming what does not exist.
        .route_layer(middleware::from_fn_with_state(state.clone(), require_session));

    // The only two routes that must work without a session.
    let open = Router::new()
        .route("/api/auth/login", post(routes::auth::login))
        .route("/api/health", get(health));

    guarded
        .merge(open)
        .layer(middleware::from_fn_with_state(state.clone(), check_origin))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Exercises the write pool specifically — the read pool's `query_only` pragma
/// means this would fail there, which is the point.
async fn health(State(state): State<WriteState>) -> Result<&'static str, AppError> {
    sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.db.write)
        .await?;
    Ok("ok")
}
