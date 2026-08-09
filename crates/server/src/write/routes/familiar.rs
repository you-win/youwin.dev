//! `GET /api/familiar`, and `POST /api/familiar/draft` — the pet, and the pet as
//! the thing you are typing would leave it.
//!
//! The familiar has always rendered on `youwin.dev`, which is where the archive
//! is read and not where it is written. As a reason to keep posting that is
//! backwards: the loop closes on a surface the author visits occasionally. This
//! is the same creature, computed by the same state machine, put next to the
//! composer — and answering a question the public site cannot ask, which is what
//! the post you have not made yet would do to it.
//!
//! JSON rather than the rendered widget, unlike `/preview/{id}`. There the point
//! is that the preview *cannot* drift from the published page, so it calls the
//! public template; here there is no published page to drift from — the pet is
//! drawn by `render::kaomoji` on both sides and the SPA is laying out three
//! strings, not reimplementing a template.

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

use crate::{
    clock::now_millis,
    db::posts::Visibility,
    error::AppError,
    familiar::{Morsel, Reading, render, stats},
    mood::Mood,
    render::markdown,
    write::WriteState,
};

/// The pet, flattened for a client that draws it rather than reasons about it.
///
/// Every enum arrives as the same lowercase word the public site prints, so the
/// SPA never maps one vocabulary onto another and a new mood does not need a
/// matching entry over there before it can be displayed.
#[derive(Debug, Serialize)]
pub struct FamiliarDto {
    /// The kaomoji, one string per line, top to bottom.
    lines: Vec<String>,
    /// The picture as a sentence, for assistive technology.
    description: String,
    stage: &'static str,
    form: &'static str,
    mood: &'static str,
    level: &'static str,
    phase: &'static str,
    topic: &'static str,
    posts: usize,
    /// Current energy as a percentage, matching the sheet's `VIT`.
    energy: u8,
    streak_days: i64,
    streak_alive: bool,
    /// The typical gap between sittings, in hours. See `familiar::baseline`.
    cadence_hours: f64,
    /// Absent for an adult, which has nowhere left to grow.
    growth: Option<GrowthDto>,
}

#[derive(Debug, Serialize)]
pub struct GrowthDto {
    toward: &'static str,
    percent: u8,
}

impl From<&Reading> for FamiliarDto {
    fn from(reading: &Reading) -> Self {
        let state = &reading.state;

        Self {
            lines: render::kaomoji(state).lines,
            description: render::describe(state),
            stage: state.stage.label(),
            form: state.form.label(),
            mood: state.mood.as_str(),
            level: state.level.label(),
            phase: state.phase.label(),
            topic: state.topic.label(),
            posts: state.posts,
            energy: stats::percent(state.energy),
            streak_days: reading.vitals.streak_days,
            streak_alive: reading.vitals.streak_alive,
            cadence_hours: reading.vitals.cadence_hours,
            growth: state
                .stage
                .next()
                .zip(state.stage.progress(state.posts))
                .map(|(toward, (_, percent))| GrowthDto {
                    toward: toward.label(),
                    percent,
                }),
        }
    }
}

/// What the composer currently holds. The same three fields `POST /api/posts`
/// takes, minus the parent — a reply feeds the pet exactly like anything else,
/// so there is nothing to say about it here.
#[derive(Debug, Deserialize)]
pub struct DraftRequest {
    body: String,
    #[serde(default = "default_visibility")]
    visibility: Visibility,
    #[serde(default)]
    mood: Option<Mood>,
}

fn default_visibility() -> Visibility {
    Visibility::Public
}

pub async fn show(State(state): State<WriteState>) -> Result<Json<FamiliarDto>, AppError> {
    let reading = state.familiar.read(&state.db.read, now_millis()).await?;

    Ok(Json(FamiliarDto::from(&reading)))
}

/// The pet as this draft would leave it.
///
/// Deliberately not validated the way `POST /api/posts` validates: asking what a
/// half-written note would do is not asking to publish it, and a 400 in the
/// middle of typing would be the composer telling you off for being unfinished.
/// Anything unpostable simply previews as no change.
pub async fn draft(
    State(state): State<WriteState>,
    Json(request): Json<DraftRequest>,
) -> Result<Json<FamiliarDto>, AppError> {
    let now = now_millis();

    // Only public posts feed the familiar, so a draft or an unlisted note has to
    // preview as *no change at all*. That is a true and slightly surprising fact
    // about the pet, and the composer is the one place it can be discovered
    // before the fact rather than by watching nothing happen afterwards.
    if request.visibility != Visibility::Public || request.body.trim().is_empty() {
        return show(State(state)).await;
    }

    // Through the real markdown pipeline, because the pet reads `body_text` and
    // matching keywords against raw markdown would count `*hike*` as something
    // other than a hike. `created_at` is replaced by `with_draft`, which is the
    // only thing that knows when a hypothetical is allowed to have happened.
    let morsel = Morsel {
        created_at: now,
        body_text: markdown::render(&request.body).text,
        mood: request.mood,
    };

    let reading = state
        .familiar
        .with_draft(&state.db.read, now, morsel)
        .await?;

    Ok(Json(FamiliarDto::from(&reading)))
}
