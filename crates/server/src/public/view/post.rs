//! Rendering a post, in the feed and in a thread.

use maud::{Markup, PreEscaped, html};

use crate::{
    db::{
        posts::{FeedRow, Post, Visibility},
        search,
    },
    public::view::time_fmt,
};

/// The rendered body.
///
/// `PreEscaped` is the single place in the entire codebase where escaping is
/// bypassed, which is why `body_html` is sanitized at write time and covered by
/// tests in `render::markdown`.
fn body(post: &Post) -> Markup {
    html! {
        div class="post-body" { (PreEscaped(&post.body_html)) }
    }
}

fn timestamp(post: &Post, precise: bool) -> Markup {
    let text = if precise {
        time_fmt::human_with_time(post.created_at)
    } else {
        time_fmt::human(post.created_at)
    };

    html! {
        time datetime=(time_fmt::rfc3339(post.created_at)) { (text) }
        @if post.edited_at.is_some() {
            span title="This post was edited after publishing." { " · edited" }
        }
        @if post.visibility == Visibility::Unlisted {
            span class="text-accent" { " · unlisted" }
        }
    }
}

/// A feed row: the whole post, linked, with a reply count when it has replies.
///
/// Posts are short, so the feed shows them in full — a microblog with "read
/// more" on a 300-character post would be absurd.
pub fn feed_item(row: &FeedRow) -> Markup {
    let href = format!("/p/{}", row.post.public_id);

    html! {
        article class="rounded-box border border-base-300 bg-base-200 p-4" {
            div class="mb-2 text-sm text-secondary" {
                a href=(href) class="no-underline hover:underline" { (timestamp(&row.post, false)) }
            }

            (body(&row.post))

            @if row.reply_count > 0 {
                a href=(href) class="mt-3 inline-block text-sm text-secondary hover:text-primary" {
                    (row.reply_count)
                    @if row.reply_count == 1 { " reply" } @else { " replies" }
                }
            }
        }
    }
}

/// A search hit: the matched fragment rather than the whole post.
///
/// The one place the site shows something other than a complete post. A page of
/// twenty full bodies is unreadable as a result list, and the fragment is the
/// thing that answers "is this the one I meant".
///
/// Each run of the snippet is escaped by maud like any other text. The marked
/// runs come back from FTS5 delimited by control characters — see
/// `db::search::MARK_OPEN` — precisely so that nothing here has to interpolate
/// database output as markup.
pub fn search_item(hit: &search::Hit) -> Markup {
    let href = format!("/p/{}", hit.post.public_id);

    html! {
        article class="rounded-box border border-base-300 bg-base-200 p-4" {
            div class="mb-2 text-sm text-secondary" {
                a href=(href) class="no-underline hover:underline" { (timestamp(&hit.post, false)) }
                @if hit.post.parent_id.is_some() {
                    span title="This post is a reply in a thread." { " · reply" }
                }
            }

            p class="post-body" {
                @for (matched, text) in search::segments(&hit.snippet) {
                    @if matched { mark { (text) } } @else { (text) }
                }
            }
        }
    }
}

/// How far the indent goes before it stops.
///
/// The column is `max-w-2xl` and most of the reading happens on a phone. Past
/// four levels the indent costs more width than the nesting is worth, so deeper
/// replies keep their place in the order and stop moving right. `thread::nest`
/// reports the true depth; this is the only thing that clamps it.
const MAX_INDENT: usize = 4;

/// The gutter for one nesting level.
///
/// Tailwind scans the source for class names and cannot see one assembled from
/// a number at runtime, so every step is written out. Tighter on a phone than
/// from `sm` up, because that is where the width actually runs out.
fn indent(depth: usize) -> &'static str {
    match depth.min(MAX_INDENT) {
        0 => "",
        1 => "ml-3 border-l border-base-300 pl-2 sm:ml-4 sm:pl-3",
        2 => "ml-6 border-l border-base-300 pl-2 sm:ml-8 sm:pl-3",
        3 => "ml-9 border-l border-base-300 pl-2 sm:ml-12 sm:pl-3",
        _ => "ml-12 border-l border-base-300 pl-2 sm:ml-16 sm:pl-3",
    }
}

/// One post inside a thread on a permalink page.
///
/// `focused` marks the post whose permalink this is — when you follow a link to
/// a reply, the whole thread renders and this is what tells you where you landed.
///
/// `depth` is how far under the root it hangs, from `thread::nest`. A reply is
/// indented under what it answered rather than appended to the end of the
/// thread, which is the only thing that distinguishes "answered the third post"
/// from "answered last".
pub fn thread_item(post: &Post, focused: bool, depth: usize) -> Markup {
    let classes = if focused {
        "rounded-box border border-primary/40 bg-base-200 p-4"
    } else {
        "rounded-box border border-base-300 bg-base-200/60 p-4"
    };

    html! {
        div class=(indent(depth)) {
            article class=(classes) id=(format!("p-{}", post.public_id)) {
                div class="mb-2 text-sm text-secondary" {
                    a href=(format!("/p/{}", post.public_id)) class="no-underline hover:underline" {
                        (timestamp(post, true))
                    }
                }
                (body(post))
            }
        }
    }
}
