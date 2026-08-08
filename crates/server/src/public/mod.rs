//! The public archive at `youwin.dev`.
//!
//! Server-rendered HTML, no JavaScript, no cookies — which is what lets the
//! whole surface sit in Cloudflare's cache and lets the CSP say
//! `default-src 'none'`.

pub mod assets;
pub mod routes;
pub mod view;

use std::{path::Path, sync::Arc};

use axum::{Router, routing::get};
use sqlx::SqlitePool;
use tower_http::{services::ServeDir, trace::TraceLayer};

use assets::Assets;

use crate::familiar::Familiar;

/// State for the public listener.
///
/// Deliberately holds ONLY the read pool. There is no handle to the writer
/// anywhere on this surface, so "the public site wrote to the database" is not a
/// reachable state — it is a type error, not a code review finding.
#[derive(Clone)]
pub struct PublicState {
    pub read: SqlitePool,
    pub assets: Assets,
    /// Absolute origin, for canonical URLs and Atom links.
    pub origin: String,
    /// The pet's five-minute snapshot. Shared rather than cloned per request —
    /// the whole point is that it survives between them. It is derived state
    /// with no schema and no writes, which is what lets it live on this side of
    /// the boundary at all.
    pub familiar: Arc<Familiar>,
}

pub fn router(read: SqlitePool, assets: Assets, origin: String, dist: &Path) -> Router {
    Router::new()
        .route("/", get(routes::feed))
        // axum 0.8 uses `{param}`; the 0.7 `:param` syntax now panics at startup.
        .route("/p/{public_id}", get(routes::permalink))
        .route("/search", get(routes::search))
        .route("/t/{tag}", get(routes::tag_page))
        .route("/tags", get(routes::tag_index))
        .route("/about", get(routes::about))
        .route("/familiar", get(routes::familiar))
        .route("/feed.xml", get(routes::feed_xml))
        .route("/health", get(routes::health))
        // In production Caddy's `handle /assets/*` block matches first, so this
        // service is never reached and the claim that the app never touches CSS
        // bytes still holds. It exists so `cargo run` on its own yields a styled
        // site — without it the request falls through to the 404 handler, and the
        // browser drops a stylesheet served as text/html. It also means a
        // misconfigured Caddy degrades to "correct but uncached" rather than
        // "unstyled".
        .nest_service("/assets", ServeDir::new(dist.join("assets")))
        .fallback(routes::fallback)
        .layer(TraceLayer::new_for_http())
        .with_state(PublicState {
            read,
            assets,
            origin,
            familiar: Arc::new(Familiar::new()),
        })
}
