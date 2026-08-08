//! Create, read, edit, and delete — the authoring surface over `posts`.
//!
//! Responses are explicit DTOs rather than serialized database structs, so the
//! internal rowid never reaches the wire. The client knows posts only by their
//! `public_id`.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    clock::now_millis,
    db::{
        posts::{self, AuthoredRow, Cursor, Visibility},
        search,
    },
    error::AppError,
    write::WriteState,
};

/// Hard ceiling on a post, enforced server-side. The composer shows a softer
/// limit; this is the one that actually rejects.
const MAX_BODY_CHARS: usize = 4000;

const PAGE_SIZE: i64 = 20;
const MAX_PAGE_SIZE: i64 = 100;

#[derive(Debug, Serialize)]
pub struct PostDto {
    /// The public id. The rowid is deliberately absent.
    id: String,
    body: String,
    body_html: String,
    visibility: &'static str,
    is_reply: bool,
    reply_count: i64,
    created_at: i64,
    edited_at: Option<i64>,
}

impl From<&AuthoredRow> for PostDto {
    fn from(row: &AuthoredRow) -> Self {
        Self {
            id: row.post.public_id.clone(),
            body: row.body.clone(),
            body_html: row.post.body_html.clone(),
            visibility: row.post.visibility.as_str(),
            is_reply: row.post.parent_id.is_some(),
            reply_count: row.reply_count,
            created_at: row.post.created_at,
            edited_at: row.post.edited_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct FeedDto {
    posts: Vec<PostDto>,
    /// Opaque cursor for the next page; absent on the last one.
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FeedParams {
    cursor: Option<String>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    body: String,
    /// Public id of the post being replied to. Absent starts a new thread.
    parent_id: Option<String>,
    #[serde(default = "default_visibility")]
    visibility: Visibility,
}

fn default_visibility() -> Visibility {
    Visibility::Public
}

#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    /// Absent means "leave the text alone" — distinct from an empty string,
    /// which would be a request to blank the post and is rejected.
    body: Option<String>,
    visibility: Option<Visibility>,
}

fn validate_body(body: &str) -> Result<(), AppError> {
    if body.trim().is_empty() {
        return Err(AppError::Invalid("A post needs a body."));
    }
    if body.chars().count() > MAX_BODY_CHARS {
        return Err(AppError::Invalid("That post is too long."));
    }
    Ok(())
}

pub async fn feed(
    State(state): State<WriteState>,
    Query(params): Query<FeedParams>,
) -> Result<Json<FeedDto>, AppError> {
    let cursor = params
        .cursor
        .as_deref()
        .and_then(Cursor::decode)
        .unwrap_or(Cursor::START);
    let limit = params.limit.unwrap_or(PAGE_SIZE).clamp(1, MAX_PAGE_SIZE);

    let (rows, next) = posts::authored_feed(&state.db.read, cursor, limit).await?;

    Ok(Json(FeedDto {
        posts: rows.iter().map(PostDto::from).collect(),
        next: next.map(Cursor::encode),
    }))
}

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    q: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
}

/// Search across everything the author can see — drafts and unlisted included.
///
/// Same `FeedDto` as the feed, so the app renders a result the same way it
/// renders any other post: full body, with the same edit and delete controls. A
/// result you cannot act on would be the wrong shape for the one surface where
/// finding a half-written draft is the point.
pub async fn search(
    State(state): State<WriteState>,
    Query(params): Query<SearchParams>,
) -> Result<Json<FeedDto>, AppError> {
    let cursor = params
        .cursor
        .as_deref()
        .and_then(Cursor::decode)
        .unwrap_or(Cursor::START);
    let limit = params.limit.unwrap_or(PAGE_SIZE).clamp(1, MAX_PAGE_SIZE);

    let (rows, next) = match search::fts_query(params.q.as_deref().unwrap_or_default()) {
        Some(query) => search::authored(&state.db.read, &query, cursor, limit).await?,
        None => (Vec::new(), None),
    };

    Ok(Json(FeedDto {
        posts: rows.iter().map(PostDto::from).collect(),
        next: next.map(Cursor::encode),
    }))
}

pub async fn drafts(State(state): State<WriteState>) -> Result<Json<FeedDto>, AppError> {
    let rows = posts::drafts(&state.db.read).await?;

    Ok(Json(FeedDto {
        posts: rows.iter().map(PostDto::from).collect(),
        next: None,
    }))
}

/// A post plus its whole thread, drafts included.
pub async fn show(
    State(state): State<WriteState>,
    Path(public_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let Some(focused) = posts::authored_by_public_id(&state.db.read, &public_id).await? else {
        return Err(AppError::NotFound);
    };

    let thread = posts::authored_thread(&state.db.read, focused.post.root_id).await?;

    Ok(Json(json!({
        "post": PostDto::from(&focused),
        "thread": thread.iter().map(PostDto::from).collect::<Vec<_>>(),
    })))
}

pub async fn create(
    State(state): State<WriteState>,
    Json(body): Json<CreateRequest>,
) -> Result<impl IntoResponse, AppError> {
    validate_body(&body.body)?;

    // Resolve the parent's public id to a rowid here rather than letting the
    // client send one — rowids are not part of the API.
    let parent_id = match body.parent_id.as_deref() {
        Some(public_id) => match posts::id_for_public_id(&state.db.read, public_id).await? {
            Some(id) => Some(id),
            None => return Err(AppError::Invalid("That post no longer exists.")),
        },
        None => None,
    };

    let post = posts::insert(
        &state.db.write,
        &body.body,
        parent_id,
        body.visibility,
        now_millis(),
    )
    .await?;

    tracing::info!(id = %post.public_id, visibility = post.visibility.as_str(), "created post");
    state.purger.purge_everything();

    // Re-read so the response carries reply_count and the stored body, matching
    // exactly what a subsequent fetch would return.
    let row = posts::authored_by_public_id(&state.db.read, &post.public_id)
        .await?
        .ok_or(AppError::NotFound)?;

    Ok((StatusCode::CREATED, Json(PostDto::from(&row))))
}

pub async fn update(
    State(state): State<WriteState>,
    Path(public_id): Path<String>,
    Json(body): Json<UpdateRequest>,
) -> Result<Json<PostDto>, AppError> {
    if body.body.is_none() && body.visibility.is_none() {
        return Err(AppError::Invalid("Nothing to change."));
    }
    if let Some(text) = &body.body {
        validate_body(text)?;
    }

    let updated = posts::update(
        &state.db.write,
        &public_id,
        body.body.as_deref(),
        body.visibility,
        now_millis(),
    )
    .await?;

    let Some(row) = updated else {
        return Err(AppError::NotFound);
    };

    tracing::info!(id = %public_id, visibility = row.post.visibility.as_str(), "updated post");

    // Fired even for a draft-to-draft edit, which cannot have been cached. The
    // alternative is comparing the visibility before and after and reasoning
    // about which transitions matter; at a handful of writes a day, one wasted
    // purge is cheaper than that rule being subtly wrong.
    state.purger.purge_everything();

    Ok(Json(PostDto::from(&row)))
}

pub async fn destroy(
    State(state): State<WriteState>,
    Path(public_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let Some(removed) = posts::soft_delete(&state.db.write, &public_id, now_millis()).await? else {
        return Err(AppError::NotFound);
    };

    tracing::info!(id = %public_id, removed, "deleted post");
    state.purger.purge_everything();

    // `removed` exceeds 1 when a thread root took its replies with it — the
    // client shows that count so the blast radius is never a surprise.
    Ok(Json(json!({ "deleted": removed })))
}
