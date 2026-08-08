//! The authoring API at `write.youwin.dev`.
//!
//! JSON only, plus the one authenticated HTML route (`/preview/:id`) that
//! renders through the public templates. Caddy serves the SPA shell off disk, so
//! nothing here returns the app's HTML. M2 adds auth; M3 the write routes.

use axum::{Router, extract::State, routing::get};
use tower_http::trace::TraceLayer;

use crate::{db::Db, error::AppError};

/// State for the authoring listener: both pools. Session validation reads, and
/// refreshing `last_seen_at` writes.
#[derive(Clone)]
pub struct WriteState {
    pub db: Db,
}

pub fn router(db: Db) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .layer(TraceLayer::new_for_http())
        .with_state(WriteState { db })
}

/// Exercises the write pool specifically — the read pool's `query_only` pragma
/// means this would fail there, which is the point.
async fn health(State(state): State<WriteState>) -> Result<&'static str, AppError> {
    sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.db.write)
        .await?;
    Ok("ok")
}
