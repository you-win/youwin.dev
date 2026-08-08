//! `GET /preview/{public_id}` — how a post will look once published.
//!
//! This is the reason the public templates are plain functions rather than
//! template files owned by another service: the preview calls *the exact same*
//! `pages::permalink` the public site calls, so it cannot drift. An approximation
//! rendered by the SPA would agree today and disagree in three months.
//!
//! Authenticated like everything else on this host, so drafts are visible here
//! and nowhere else.

use axum::extract::{Path, State};
use maud::Markup;

use crate::{
    db::posts,
    error::AppError,
    public::view::pages,
    render::markdown,
    write::WriteState,
};

/// Matches the public site's permalink handler, so the preview's `<title>` and
/// description are cut at the same lengths.
const TITLE_CHARS: usize = 70;
const SUMMARY_CHARS: usize = 160;

pub async fn show(
    State(state): State<WriteState>,
    Path(public_id): Path<String>,
) -> Result<Markup, AppError> {
    let Some(focused) = posts::authored_by_public_id(&state.db.read, &public_id).await? else {
        return Err(AppError::NotFound);
    };

    let thread = posts::authored_thread(&state.db.read, focused.post.root_id).await?;

    // Rendered against the PUBLIC origin, not this one: the canonical link and
    // og:url must show where the post will actually live.
    let posts: Vec<_> = thread.into_iter().map(|row| row.post).collect();

    Ok(pages::permalink(
        &state.assets,
        &state.public_origin,
        &focused.post,
        &posts,
        markdown::summarize(&focused.post.body_text, TITLE_CHARS),
        markdown::summarize(&focused.post.body_text, SUMMARY_CHARS),
    ))
}
