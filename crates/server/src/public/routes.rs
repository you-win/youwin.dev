//! Handlers for the public archive.

use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use maud::Markup;
use serde::Deserialize;

use crate::{
    calendar::{MonthDay, YearMonth},
    clock::now_millis,
    db::{
        archive,
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

    // Only the first page pays for it, and only on a cache miss — see
    // `familiar::cache`.
    let familiar = if is_first_page {
        Some(state.familiar.read(&state.read, now_millis()).await?)
    } else {
        None
    };

    Ok(pages::feed(
        &state.assets,
        &state.origin,
        &rows,
        older,
        is_first_page,
        familiar.as_ref(),
    ))
}

/// The familiar's own page.
pub async fn familiar(State(state): State<PublicState>) -> Result<Markup, AppError> {
    let reading = state.familiar.read(&state.read, now_millis()).await?;
    Ok(pages::familiar(&state.assets, &state.origin, &reading))
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

/// The spine: every month with posts, grouped by year.
pub async fn archive_index(State(state): State<PublicState>) -> Result<Markup, AppError> {
    let months = archive::months(&state.read).await?;

    // Today, only to point the "on this day" link somewhere. It moves at
    // midnight UTC and the page is cached for five minutes, so the link can name
    // yesterday for that long — which costs a reader nothing and saves this page
    // from being uncacheable.
    let today = MonthDay::of(now_millis()).unwrap_or(MonthDay { month: 1, day: 1 });

    Ok(pages::archive_index(
        &state.assets,
        &state.origin,
        &months,
        today,
    ))
}

/// `/archive/2026` — a truncated URL, not a page of its own.
///
/// The index already lists every month of every year, so a year page would be
/// the same content under a second URL. A redirect to that year's section is the
/// honest answer to what the reader was reaching for. Temporary rather than
/// permanent: a 308 is cached by browsers effectively forever, and this is a
/// judgement about page structure rather than a fact about the URL.
pub async fn archive_year(Path(year): Path<String>) -> Response {
    // Validated through `YearMonth` so `/archive/../etc` cannot reach the
    // fragment: only a real year, in canonical form, is ever echoed back.
    let Some(parsed) = YearMonth::parse(&year, "1") else {
        return StatusCode::NOT_FOUND.into_response();
    };

    (
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("/archive#y{:04}", parsed.year),
        )],
    )
        .into_response()
}

/// One calendar month.
pub async fn archive_month(
    State(state): State<PublicState>,
    Path((year, month)): Path<(String, String)>,
    Query(params): Query<FeedParams>,
) -> Result<Response, AppError> {
    let Some(month) = YearMonth::parse(&year, &month).filter(|m| m.bounds().is_some()) else {
        return Ok(not_found(&state).into_response());
    };
    let (start, end) = month.bounds().expect("filtered above");

    let cursor = params
        .before
        .as_deref()
        .and_then(Cursor::decode)
        .unwrap_or(Cursor::START);
    let is_first_page = cursor == Cursor::START;

    let (rows, older) = archive::month_page(&state.read, start, end, cursor, PAGE_SIZE).await?;

    // A month nothing was ever written in is a 404, for the same reason an
    // unused tag is: without it, `/archive/1804/03` and every other month of
    // every year is a valid URL, and the archive becomes an unbounded space of
    // empty pages for a crawler to walk. Only the first page decides this — a
    // valid cursor pointing past the end is an empty page, not a missing month.
    if is_first_page && rows.is_empty() {
        return Ok(not_found(&state).into_response());
    }

    Ok(pages::archive_month(
        &state.assets,
        &state.origin,
        month,
        &rows,
        older,
        is_first_page,
    )
    .into_response())
}

/// One day of the year, in every year.
pub async fn on_this_day(
    State(state): State<PublicState>,
    Path((month, day)): Path<(String, String)>,
    Query(params): Query<FeedParams>,
) -> Result<Response, AppError> {
    // An impossible date — 31 April, 30 February — is a 404. A *possible* one
    // with nothing on it is a 200: there are 366 of those and no more, so unlike
    // the month pages this is not a space that can be walked indefinitely.
    let Some(day) = MonthDay::parse(&month, &day) else {
        return Ok(not_found(&state).into_response());
    };

    let cursor = params
        .before
        .as_deref()
        .and_then(Cursor::decode)
        .unwrap_or(Cursor::START);
    let is_first_page = cursor == Cursor::START;

    let (rows, older) = archive::day_page(&state.read, &day.key(), cursor, PAGE_SIZE).await?;

    Ok(pages::on_this_day(
        &state.assets,
        &state.origin,
        day,
        &rows,
        older,
        is_first_page,
    )
    .into_response())
}

/// `/random` — somewhere in the archive, chosen fresh every time.
///
/// `no-store` rather than a short TTL: a cached redirect is not random, it is
/// one post with extra steps. Caddy sets `Cache-Control` on everything it
/// proxies, so this header only survives because the site block has a matcher
/// that leaves `/random` alone — see `deploy/youwin.dev.caddy`.
///
/// An empty archive redirects to the feed instead of 404ing. The URL is not
/// broken, it just has nothing to offer yet, and the feed says so in words.
pub async fn random(State(state): State<PublicState>) -> Result<Response, AppError> {
    let target = match posts::random_public_id(&state.read).await? {
        Some(public_id) => format!("/p/{public_id}"),
        None => "/".to_owned(),
    };

    Ok((
        StatusCode::FOUND,
        [
            (header::LOCATION, target),
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
    )
        .into_response())
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
