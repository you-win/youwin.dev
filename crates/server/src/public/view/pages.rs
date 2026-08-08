//! The three HTML pages: feed, permalink, about.

use maud::{Markup, html};

use crate::{
    db::posts::{Cursor, FeedRow, Post},
    public::{assets::Assets, view::post},
};

/// The feed.
///
/// Pagination is a link, not a scroll sentinel: crawlable, linkable, and
/// back-button-correct. `older` is `None` on the last page.
pub fn feed(
    assets: &Assets,
    origin: &str,
    rows: &[FeedRow],
    older: Option<Cursor>,
    is_first_page: bool,
) -> Markup {
    let canonical = if is_first_page {
        format!("{origin}/")
    } else {
        // Later pages are transient — the same cursor points somewhere else once
        // enough posts land above it. Point the canonical at the front page and
        // let `noindex` below keep them out of the index entirely.
        format!("{origin}/")
    };

    let mut page = super::layout::Page::new(
        "youwin.dev",
        "Notes, mostly about software.",
        canonical,
    );
    page.noindex = !is_first_page;

    let content = html! {
        @if rows.is_empty() {
            p class="text-secondary" { "Nothing here yet." }
        } @else {
            div class="flex flex-col gap-4" {
                @for row in rows {
                    (post::feed_item(row))
                }
            }
        }

        @if older.is_some() || !is_first_page {
            nav class="mt-8 flex justify-between text-sm" {
                @if !is_first_page {
                    a href="/" { "← newest" }
                } @else {
                    span {}
                }
                @if let Some(cursor) = older {
                    a href=(format!("/?before={}", cursor.encode())) { "older →" }
                }
            }
        }
    };

    super::layout::render(assets, &page, content)
}

/// A permalink: the whole thread, with the requested post marked.
pub fn permalink(
    assets: &Assets,
    origin: &str,
    focused: &Post,
    thread: &[Post],
    short_summary: String,
    description: String,
) -> Markup {
    use crate::db::posts::Visibility;

    // Posts have no titles, so the opening words stand in — kept short, because
    // a tab or a search result truncates long ones mid-word.
    let title = if short_summary.is_empty() {
        "Post — youwin.dev".to_owned()
    } else {
        format!("{short_summary} — youwin.dev")
    };

    let mut page = super::layout::Page::new(
        &title,
        &description,
        format!("{origin}/p/{}", focused.public_id),
    );
    page.og_type = "article";
    page.published = Some(super::time_fmt::rfc3339(focused.created_at));
    // An unlisted post is reachable by link but must never be indexed — that is
    // the entire difference between `unlisted` and `public`.
    page.noindex = focused.visibility == Visibility::Unlisted;

    let content = html! {
        div class="flex flex-col gap-3" {
            @for item in thread {
                (post::thread_item(item, item.id == focused.id))
            }
        }

        nav class="mt-8 text-sm" {
            a href="/" { "← back to the feed" }
        }
    };

    super::layout::render(assets, &page, content)
}

pub fn about(assets: &Assets, origin: &str) -> Markup {
    let page = super::layout::Page::new(
        "About — youwin.dev",
        "About youwin.dev.",
        format!("{origin}/about"),
    );

    let content = html! {
        div class="post-body" {
            p { "I'm youwin. This is where I keep short notes, mostly about software." }
            p {
                "Code lives on "
                a href="https://github.com/you-win" rel="me noopener" { "GitHub" }
                ". There is an "
                a href="/feed.xml" { "Atom feed" }
                "."
            }
        }
    };

    super::layout::render(assets, &page, content)
}

/// 404. Also what a draft or a deleted post returns — a visitor cannot tell
/// those apart from a bad id, which is the intent.
pub fn not_found(assets: &Assets, origin: &str) -> Markup {
    let mut page = super::layout::Page::new(
        "Not found — youwin.dev",
        "That page doesn't exist.",
        format!("{origin}/"),
    );
    page.noindex = true;

    let content = html! {
        p class="text-secondary" { "That page doesn't exist." }
        nav class="mt-8 text-sm" { a href="/" { "← back to the feed" } }
    };

    super::layout::render(assets, &page, content)
}
