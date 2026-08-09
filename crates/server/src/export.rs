//! `youwin-server export <dir>` — the archive that outlives this program.
//!
//! Two forms of the same data, because they insure against different things.
//! `posts.json` is complete and machine-readable: every column, deletions
//! included, enough to rebuild the database exactly. The markdown tree is what
//! you can still read in ten years with no Rust toolchain, no SQLite, and no
//! memory of how any of this worked.
//!
//! Writes are additive. Nothing here deletes or prunes, so pointing two runs at
//! the same directory refreshes it rather than rebuilding it, and a partial run
//! never destroys a good earlier export.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};
use serde::Serialize;

use crate::{
    clock::now_millis,
    db::{Db, posts, tags},
    offsite::Uploader,
    public::view::time_fmt,
};

/// One post as it appears in `posts.json`.
#[derive(Debug, Serialize)]
struct Entry {
    #[serde(flatten)]
    post: posts::ExportRow,
    tags: Vec<String>,
}

pub async fn run(db: &Db, dir: &Path, offsite: &Uploader) -> Result<()> {
    let rows = posts::export_all(&db.read)
        .await
        .context("reading posts for export")?;

    let mut by_post: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    for (post_id, display) in tags::by_post(&db.read).await.context("reading tags")? {
        by_post.entry(post_id).or_default().push(display);
    }

    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let markdown_dir = dir.join("markdown");
    fs::create_dir_all(&markdown_dir)
        .with_context(|| format!("creating {}", markdown_dir.display()))?;

    let entries: Vec<Entry> = rows
        .into_iter()
        .map(|post| {
            let tags = by_post.get(&post.id).cloned().unwrap_or_default();
            Entry { post, tags }
        })
        .collect();

    let json_path = dir.join("posts.json");
    let json = serde_json::to_string_pretty(&entries).context("serializing posts")?;
    fs::write(&json_path, &json).with_context(|| format!("writing {}", json_path.display()))?;

    let mut written = 0;
    let mut skipped = 0;

    for entry in &entries {
        // Deleted posts stay in the JSON but not in the readable tree: the tree
        // is "what I wrote", and a deletion is a statement that something is no
        // longer part of that. It remains recoverable from posts.json.
        if entry.post.deleted_at.is_some() {
            skipped += 1;
            continue;
        }

        let path = markdown_dir.join(filename(entry));
        fs::write(&path, document(entry)).with_context(|| format!("writing {}", path.display()))?;
        written += 1;
    }

    println!(
        "Exported {} posts to {} ({written} markdown files, {skipped} deleted and JSON-only).",
        entries.len(),
        dir.display()
    );

    // `posts.json` alone goes off-site, and it goes dated.
    //
    // Dated, unlike the local copy, because the local directory is refreshed in
    // place — which is right for a working export and wrong for the copy that
    // has to survive the machine. If a bad run overwrote `posts.json` here, an
    // off-site object of the same name would be overwritten with it.
    //
    // Alone, because the markdown tree is a rendering of this file: everything
    // in it is derivable from the JSON, and uploading a directory means either a
    // tar dependency or one request per post. JSON with no schema and no
    // toolchain is already readable in ten years, which was the tree's whole
    // argument for existing.
    if offsite.is_enabled() {
        let name = format!("youwin-{}.json", time_fmt::date(now_millis()));
        offsite
            .put(&name, "application/json", json.into_bytes())
            .await?;
    }

    Ok(())
}

/// `2026-08-08-3fK9pQ2mXvT1aB7c.md`.
///
/// Date first so the directory sorts chronologically in any file browser; the
/// public id second because it is unique and already URL- and filename-safe
/// (base64url is `A-Za-z0-9-_`), so no escaping is needed and no two posts can
/// collide.
fn filename(entry: &Entry) -> PathBuf {
    PathBuf::from(format!(
        "{}-{}.md",
        time_fmt::date(entry.post.created_at),
        entry.post.public_id
    ))
}

/// Front matter plus the markdown source, exactly as it was typed.
///
/// The front matter is deliberately flat — no nested structures, no quoting
/// rules to remember — so it is readable as text and parseable by essentially
/// any static site generator, which is the most likely destination if this
/// archive is ever reused.
fn document(entry: &Entry) -> String {
    let post = &entry.post;
    let mut out = String::with_capacity(post.body.len() + 256);

    out.push_str("---\n");
    out.push_str(&format!("id: {}\n", post.public_id));
    out.push_str(&format!("date: {}\n", time_fmt::rfc3339(post.created_at)));
    out.push_str(&format!("visibility: {}\n", post.visibility.as_str()));

    if let Some(mood) = post.mood {
        out.push_str(&format!("mood: {}\n", mood.as_str()));
    }
    if let Some(edited) = post.edited_at {
        out.push_str(&format!("edited: {}\n", time_fmt::rfc3339(edited)));
    }
    if let Some(parent) = &post.parent_public_id {
        out.push_str(&format!("reply_to: {parent}\n"));
        out.push_str(&format!("thread: {}\n", post.root_public_id));
    }
    if !entry.tags.is_empty() {
        out.push_str(&format!("tags: {}\n", entry.tags.join(", ")));
    }

    out.push_str("---\n\n");
    out.push_str(&post.body);
    if !post.body.ends_with('\n') {
        out.push('\n');
    }

    out
}
