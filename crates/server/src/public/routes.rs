//! Handlers for the public archive.

use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use maud::Markup;
use serde::Deserialize;

use crate::{
    db::{
        posts::{self, Cursor},
        search, tags,
    },
    error::AppError,
    public::{
        PublicState,
        view::{atom, pages},
    },
    render::markdown,
};

/// Posts per page. Bodies render in full, so this is a page of reading rather
/// than a page of headlines.
const PAGE_SIZE: i64 = 20;

/// Entries in the Atom document.
const FEED_ENTRIES: i64 = 20;

/// Characters of body text used for `og:description`.
const SUMMARY_CHARS: usize = 160;

/// Characters used for the `<title>` prefix. Much shorter than the description:
/// search results and browser tabs truncate around 60, and a title cut off
/// mid-sentence reads worse than a short one.
const TITLE_CHARS: usize = 70;

#[derive(Debug, Deserialize)]
pub struct FeedParams {
    /// Opaque keyset cursor. A malformed value is treated as "no cursor" rather
    /// than an error — a mangled URL should show the feed, not a 400.
    before: Option<String>,
}

pub async fn feed(
    State(state): State<PublicState>,
    Query(params): Query<FeedParams>,
) -> Result<Markup, AppError> {
    let cursor = params
        .before
        .as_deref()
        .and_then(Cursor::decode)
        .unwrap_or(Cursor::START);

    let is_first_page = cursor == Cursor::START;
    let (rows, older) = posts::feed_page(&state.read, cursor, PAGE_SIZE).await?;

    Ok(pages::feed(
        &state.assets,
        &state.origin,
        &rows,
        older,
        is_first_page,
    ))
}

pub async fn permalink(
    State(state): State<PublicState>,
    Path(public_id): Path<String>,
) -> Result<Response, AppError> {
    let Some(focused) = posts::by_public_id(&state.read, &public_id).await? else {
        // Drafts, deletions, and nonsense ids all land here, indistinguishably.
        return Ok(not_found(&state).into_response());
    };

    let thread = posts::thread(&state.read, focused.root_id).await?;

    Ok(pages::permalink(
        &state.assets,
        &state.origin,
        &focused,
        &thread,
        markdown::summarize(&focused.body_text, TITLE_CHARS),
        markdown::summarize(&focused.body_text, SUMMARY_CHARS),
    )
    .into_response())
}

/// Longest query echoed back into the page. Anything past this is not a search,
/// and a title tag built from 8 kB of query string is its own small problem.
const MAX_QUERY_CHARS: usize = 120;

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    q: Option<String>,
    before: Option<String>,
}

pub async fn search(
    State(state): State<PublicState>,
    Query(params): Query<SearchParams>,
) -> Result<Markup, AppError> {
    let typed: String = params
        .q
        .unwrap_or_default()
        .trim()
        .chars()
        .take(MAX_QUERY_CHARS)
        .collect();

    let cursor = params
        .before
        .as_deref()
        .and_then(Cursor::decode)
        .unwrap_or(Cursor::START);
    let is_first_page = cursor == Cursor::START;

    // `None` means there was nothing tokenizable — an empty box, or someone who
    // typed only punctuation. Both render as "no results" without touching the
    // database, which also keeps `?q=%22%22` off the query planner.
    let hits = match search::fts_query(&typed) {
        Some(query) => search::public(&state.read, &query, cursor, PAGE_SIZE).await?,
        None => (Vec::new(), None),
    };

    Ok(pages::search(
        &state.assets,
        &state.origin,
        &typed,
        &hits.0,
        hits.1,
        is_first_page,
    ))
}

pub async fn tag_page(
    State(state): State<PublicState>,
    Path(name): Path<String>,
    Query(params): Query<FeedParams>,
) -> Result<Response, AppError> {
    // A tag nothing has ever used is a 404, not an empty page: without that,
    // `/t/<anything>` is an unbounded space of valid URLs for a crawler to walk.
    // A tag whose posts were all deleted keeps its row, so that page stays a 200
    // — the URL meant something once and may again.
    let Some(display) = tags::display_name(&state.read, &name).await? else {
        return Ok(not_found(&state).into_response());
    };

    let cursor = params
        .before
        .as_deref()
        .and_then(Cursor::decode)
        .unwrap_or(Cursor::START);
    let is_first_page = cursor == Cursor::START;

    let (rows, older) = tags::feed_page(&state.read, &name, cursor, PAGE_SIZE).await?;

    Ok(pages::tag_page(
        &state.assets,
        &state.origin,
        &display,
        &rows,
        older,
        is_first_page,
    )
    .into_response())
}

pub async fn tag_index(State(state): State<PublicState>) -> Result<Markup, AppError> {
    let all = tags::all(&state.read).await?;
    Ok(pages::tag_index(&state.assets, &state.origin, &all))
}

pub async fn about(State(state): State<PublicState>) -> Markup {
    pages::about(&state.assets, &state.origin)
}

pub async fn feed_xml(State(state): State<PublicState>) -> Result<Response, AppError> {
    let (rows, _) = posts::feed_page(&state.read, Cursor::START, FEED_ENTRIES).await?;
    let body = atom::render(&state.origin, &rows);

    Ok((
        [(header::CONTENT_TYPE, "application/atom+xml; charset=utf-8")],
        body,
    )
        .into_response())
}

/// Touches the pool rather than returning a constant, so a green health check
/// proves the database is reachable and not merely that axum is listening.
pub async fn health(State(state): State<PublicState>) -> Result<&'static str, AppError> {
    sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.read)
        .await?;
    Ok("ok")
}

/// The catch-all, and the body for unknown ids. Renders the themed 404 page
/// rather than axum's bare text, and keeps the 404 status so Cloudflare and
/// crawlers treat it correctly.
pub async fn fallback(State(state): State<PublicState>) -> Response {
    not_found(&state).into_response()
}

fn not_found(state: &PublicState) -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        pages::not_found(&state.assets, &state.origin),
    )
}
