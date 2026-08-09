//! `GET /api/moods` — how each month was written.
//!
//! Mood is collected on every post and read by exactly one thing, the familiar,
//! which spends it on a kaomoji and an aggregate. This is the other direction:
//! the field handed back to the person filling it in, as a shape they can see.
//!
//! It stays on the authoring host, where the composer that records it lives.
//! Mood never renders on youwin.dev — that is a deliberate rule, not an
//! oversight — and a public chart of it would be exactly the disclosure the rule
//! exists to prevent.

use axum::{Json, extract::State};

use crate::{
    calendar::YearMonth,
    db::archive,
    error::AppError,
    mood::Mood,
    write::WriteState,
};

#[derive(Debug, serde::Serialize)]
pub struct MonthDto {
    /// `YYYY-MM` — stable, sortable, and what a client would key on.
    month: String,
    /// `August 2026`. Formatted here so the client needs no month-name table and
    /// cannot disagree with the public site about what a month is called.
    label: String,
    /// Every post written that month, whatever its mood or visibility.
    total: i64,
    /// One entry per mood, always all seven, always in `Mood::ALL` order.
    ///
    /// Zeros included so the client can index straight into it: a chart that has
    /// to cope with absent keys ends up with colours that shift between months,
    /// which is the one thing a timeline must not do.
    moods: Vec<MoodDto>,
    /// Posts with no mood picked.
    ///
    /// Deliberately not an eighth entry in `moods`. "Did not say" is not a mood
    /// — it is the absence of one, and the familiar treats it as permission to
    /// infer rather than as a value. Flattening it into the list would put that
    /// distinction one refactor away from being lost.
    unsaid: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct MoodDto {
    mood: &'static str,
    posts: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct MoodsDto {
    /// Newest month first, matching every other list in this API.
    months: Vec<MonthDto>,
}

pub async fn show(State(state): State<WriteState>) -> Result<Json<MoodsDto>, AppError> {
    let rows = archive::moods_by_month(&state.db.read).await?;

    // The query returns months newest-first with each month's moods adjacent, so
    // this is a fold rather than a group-by — the same shape the public archive
    // index uses, for the same reason.
    let mut months: Vec<MonthDto> = Vec::new();

    for row in rows {
        if months.last().map(|m| m.month.as_str()) != Some(row.month.as_str()) {
            let label = YearMonth::from_key(&row.month)
                .map(YearMonth::label)
                // `strftime` cannot produce a key this fails to parse; falling
                // back to the raw key beats dropping a month of someone's
                // writing over a formatting problem.
                .unwrap_or_else(|| row.month.clone());

            months.push(MonthDto {
                month: row.month.clone(),
                label,
                total: 0,
                moods: Mood::ALL
                    .into_iter()
                    .map(|mood| MoodDto {
                        mood: mood.as_str(),
                        posts: 0,
                    })
                    .collect(),
                unsaid: 0,
            });
        }

        let month = months.last_mut().expect("just pushed");
        month.total += row.posts;

        match row.mood {
            Some(mood) => month.moods[mood.index()].posts += row.posts,
            None => month.unsaid += row.posts,
        }
    }

    Ok(Json(MoodsDto { months }))
}
