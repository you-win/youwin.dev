//! `youwin-server seed` — writes a handful of posts for local development.
//!
//! Goes through `posts::insert`, and therefore through the real markdown
//! pipeline, rather than raw SQL. Hand-written `body_html` in a fixture would
//! drift from what the renderer actually produces, and then the thing you are
//! looking at in the browser would not be the thing the site serves.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};

use crate::{
    db::{
        Db,
        posts::{self, Visibility},
    },
    mood::Mood,
};

const DAY_MILLIS: i64 = 24 * 60 * 60 * 1000;

pub async fn run(db: &Db) -> Result<()> {
    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM posts")
        .fetch_one(&db.write)
        .await?;

    if existing > 0 {
        bail!("database already has {existing} posts; refusing to seed on top of them");
    }

    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before 1970")
            .as_millis(),
    )
    .expect("timestamp fits in i64 until the year 292 million");

    // Deliberately exercises every branch the templates have: a plain post, one
    // with formatting, a thread, an unlisted post, a draft, and enough hashtags
    // that /tags and /t/:tag have something to show. If the feed looks right
    // after seeding, the feed is right.
    //
    // Moods are set on some and left off others on purpose, so the familiar has
    // both a picked mood and an inferred one to work from.
    let root = posts::insert(
        &db.write,
        "Rebuilt this site as a microblog. #Rust on the back, no JavaScript on the front.\n\
         The whole public surface is server-rendered and sits in cache.",
        None,
        Visibility::Public,
        Some(Mood::Excited),
        now - 3 * DAY_MILLIS,
    )
    .await?;

    posts::insert(
        &db.write,
        "The part I keep coming back to: `SoftBreak` → `HardBreak` in the event stream. \
         CommonMark collapses a single newline into a space, which is wrong for short posts \
         — every newline you typed is a newline you meant.",
        Some(root.id),
        Visibility::Public,
        None,
        now - 3 * DAY_MILLIS + 20 * 60 * 1000,
    )
    .await?;

    posts::insert(
        &db.write,
        "Second thing: #sqlite takes exactly one writer. Two pools — one connection for \
         writes, four for reads — and the collision stops being possible instead of being \
         retried. #rust makes that a type-level guarantee rather than a convention.",
        Some(root.id),
        Visibility::Public,
        Some(Mood::Contemplative),
        now - 3 * DAY_MILLIS + 55 * 60 * 1000,
    )
    .await?;

    posts::insert(
        &db.write,
        "Reading *Thinking Forth* again. Holds up: https://thinking-forth.sourceforge.net/ \
         — the chapter on factoring is worth the whole book.",
        None,
        Visibility::Public,
        Some(Mood::Content),
        now - 2 * DAY_MILLIS,
    )
    .await?;

    posts::insert(
        &db.write,
        "Formatting check — **bold**, *italic*, ~~struck~~, `inline code`, and a list:\n\n\
         - one\n- two\n- three\n\n\
         > Mist has no hard edges.\n\n\
         Tag handling too: #web-dev is one tag, `#notatag` in code is none, and C# is \
         not a tag at all.",
        None,
        Visibility::Public,
        // Left unset, so the familiar infers one from the text.
        None,
        now - DAY_MILLIS,
    )
    .await?;

    let unlisted = posts::insert(
        &db.write,
        "Unlisted: reachable if you have the link, never in the feed, never indexed.",
        None,
        Visibility::Unlisted,
        None,
        now - 6 * 60 * 60 * 1000,
    )
    .await?;

    posts::insert(
        &db.write,
        "A draft. Should 404 on the public site, indistinguishably from a bad id.",
        None,
        Visibility::Draft,
        Some(Mood::Neutral),
        now - 60 * 60 * 1000,
    )
    .await?;

    tracing::info!("seeded 7 posts");
    println!("Seeded. Thread root: /p/{}", root.public_id);
    println!("Unlisted (not in the feed): /p/{}", unlisted.public_id);

    Ok(())
}
