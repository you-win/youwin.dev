//! The HTML pages: feed, permalink, search, tag, tag index, about, 404.

use maud::{Markup, html};

use crate::{
    calendar::{MonthDay, YearMonth},
    db::{
        archive::MonthCount,
        posts::{Cursor, FeedRow, Post},
        search,
        tags::TagCount,
    },
    familiar::Reading,
    public::{assets::Assets, view::post},
    tag, url,
};

/// Previous/next links, shared by every paginated page.
///
/// Takes ready-made hrefs rather than assembling them: the three callers differ
/// in how their base URL is built (a bare path, a tag path, a query string that
/// has to survive percent-encoding), and threading that through here would be
/// more conditional than the markup.
fn pager(newest: &str, older: Option<String>, is_first_page: bool) -> Markup {
    html! {
        @if older.is_some() || !is_first_page {
            nav class="mt-8 flex justify-between text-sm" {
                @if !is_first_page {
                    a href=(newest) { "← newest" }
                } @else {
                    span {}
                }
                @if let Some(href) = older {
                    a href=(href) { "older →" }
                }
            }
        }
    }
}

/// The feed.
///
/// Pagination is a link, not a scroll sentinel: crawlable, linkable, and
/// back-button-correct. `older` is `None` on the last page.
///
/// `familiar` is `Some` only on the first page. The pet reads the *whole*
/// archive and says nothing about the twenty posts under it, so repeating it
/// down the pagination would be twenty copies of one fact — and the second page
/// is `noindex` scaffolding, not somewhere anyone lands.
pub fn feed(
    assets: &Assets,
    origin: &str,
    rows: &[FeedRow],
    older: Option<Cursor>,
    is_first_page: bool,
    familiar: Option<&Reading>,
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
        @if let Some(reading) = familiar {
            div class="mb-6" { (super::familiar::widget(reading)) }
        }

        @if rows.is_empty() {
            p class="text-secondary" { "Nothing here yet." }
        } @else {
            div class="flex flex-col gap-4" {
                @for row in rows {
                    (post::feed_item(row))
                }
            }
        }

        (pager("/", older.map(|c| format!("/?before={}", c.encode())), is_first_page))
    };

    super::layout::render(assets, &page, content)
}

/// The familiar's own page: the pet at full size, with the numbers under it.
pub fn familiar(assets: &Assets, origin: &str, reading: &Reading) -> Markup {
    let page = super::layout::Page::new(
        "The Familiar — youwin.dev",
        "A kaomoji that reads the archive's temperature.",
        format!("{origin}/familiar"),
    );

    let content = html! {
        div class="mb-6 flex items-baseline justify-between" {
            h1 class="text-lg font-medium" { "The Familiar" }
            a href="/" class="text-sm text-secondary" { "← the feed" }
        }

        (super::familiar::sheet(reading))

        p class="mt-8 text-sm text-secondary" {
            "Everything above is derived from the posts on this site — what they are "
            "about, how they read, and how long it has been since the last one. "
            "It changes on its own, about every five minutes."
        }
    };

    super::layout::render(assets, &page, content)
}

/// Search results.
///
/// Always `noindex`: these pages are generated from whatever anyone types, so
/// letting a crawler in creates an unbounded set of thin pages that compete with
/// the posts themselves.
pub fn search(
    assets: &Assets,
    origin: &str,
    query: &str,
    hits: &[search::Hit],
    older: Option<Cursor>,
    is_first_page: bool,
) -> Markup {
    let title = if query.is_empty() {
        "Search — youwin.dev".to_owned()
    } else {
        format!("Search: {query} — youwin.dev")
    };

    let mut page = super::layout::Page::new(&title, "Search youwin.dev.", format!("{origin}/search"));
    page.noindex = true;
    page.search = query;

    let encoded = url::encode_component(query);
    let base = format!("/search?q={encoded}");

    let content = html! {
        @if query.is_empty() {
            p class="text-secondary" { "Type something into the box above." }
        } @else if hits.is_empty() {
            p class="text-secondary" {
                "Nothing matches “" (query) "”."
            }
        } @else {
            @if is_first_page {
                p class="mb-4 text-sm text-secondary" {
                    "Results for “" (query) "”, newest first."
                }
            }
            div class="flex flex-col gap-4" {
                @for hit in hits { (post::search_item(hit)) }
            }
        }

        (pager(
            &base,
            older.map(|c| format!("{base}&before={}", c.encode())),
            is_first_page,
        ))
    };

    super::layout::render(assets, &page, content)
}

/// Everything carrying one hashtag.
pub fn tag_page(
    assets: &Assets,
    origin: &str,
    display: &str,
    rows: &[FeedRow],
    older: Option<Cursor>,
    is_first_page: bool,
) -> Markup {
    let href = tag::href(display);
    let title = format!("#{display} — youwin.dev");
    let description = format!("Posts tagged #{display}.");

    let mut page = super::layout::Page::new(
        &title,
        &description,
        // Canonical is the lowercased path, so `/t/Rust` and `/t/rust` are not
        // two pages with the same content.
        format!("{origin}{href}"),
    );
    // An empty tag page is a URL that was meaningful once and may be again —
    // worth serving, not worth indexing.
    page.noindex = !is_first_page || rows.is_empty();

    let content = html! {
        div class="mb-6 flex items-baseline justify-between" {
            h1 class="text-lg font-medium" { "#" (display) }
            a href="/tags" class="text-sm text-secondary" { "all tags" }
        }

        @if rows.is_empty() {
            p class="text-secondary" { "Nothing here." }
        } @else {
            div class="flex flex-col gap-4" {
                @for row in rows { (post::feed_item(row)) }
            }
        }

        (pager(
            &href,
            older.map(|c| format!("{href}?before={}", c.encode())),
            is_first_page,
        ))
    };

    super::layout::render(assets, &page, content)
}

/// Every tag in use, most-used first. Without this a tag page is reachable only
/// by noticing a hashtag inside a post.
pub fn tag_index(assets: &Assets, origin: &str, all: &[TagCount]) -> Markup {
    let page = super::layout::Page::new(
        "Tags — youwin.dev",
        "Every tag in use on youwin.dev.",
        format!("{origin}/tags"),
    );

    let content = html! {
        h1 class="mb-6 text-lg font-medium" { "Tags" }

        @if all.is_empty() {
            p class="text-secondary" { "No tags yet." }
        } @else {
            ul class="flex flex-wrap gap-2" {
                @for entry in all {
                    li {
                        // The space between the two spans is a real space, not a
                        // flex `gap`. A gap separates them on screen but leaves
                        // the text content as "#rust2", which is what a screen
                        // reader announces and what a copy-paste produces.
                        a href=(tag::href(&entry.tag))
                          class="inline-flex items-baseline rounded-box border border-base-300 \
                                 bg-base-200 px-3 py-1.5 text-sm no-underline hover:border-primary/50" {
                            span { "#" (entry.display) }
                            span class="text-secondary" { " " (entry.posts) }
                        }
                    }
                }
            }
        }
    };

    super::layout::render(assets, &page, content)
}

/// A month chip, shared by the archive index and nothing else yet.
///
/// Deliberately the same shape as a tag chip: both are "a bounded set of links
/// with a count", and a reader who has used one has used the other.
fn month_chip(month: YearMonth, posts: i64) -> Markup {
    html! {
        li {
            a href=(month.href())
              class="inline-flex items-baseline rounded-box border border-base-300 \
                     bg-base-200 px-3 py-1.5 text-sm no-underline hover:border-primary/50" {
                span { (month.month_label()) }
                // A real space, not a flex gap — see `tag_index`, which learned
                // this the same way.
                span class="text-secondary" { " " (posts) }
            }
        }
    }
}

/// The spine: every month that has posts, grouped under its year.
///
/// One page rather than a year index that leads to a month index. A personal
/// archive gains twelve rows a year, so the complete thing stays small for
/// decades — and complete is the property worth having here, since the whole
/// reason this page exists is that the feed's cursor pagination cannot answer
/// "what was I writing three years ago" without twenty clicks.
///
/// `today` is what the "on this day" link points at, so the one view that needs
/// no navigation to be useful has somewhere to be found from.
pub fn archive_index(
    assets: &Assets,
    origin: &str,
    months: &[MonthCount],
    today: MonthDay,
) -> Markup {
    let page = super::layout::Page::new(
        "Archive — youwin.dev",
        "Every month of youwin.dev, by year.",
        format!("{origin}/archive"),
    );

    // The query returns months newest-first, so consecutive rows of the same
    // year are already adjacent — grouping is a fold, not a sort. A row whose
    // key will not parse is dropped rather than rendered as a broken link;
    // `strftime` cannot produce one, so this is a guard, not a case.
    let mut years: Vec<(i32, Vec<(YearMonth, i64)>)> = Vec::new();
    for entry in months {
        let Some(month) = YearMonth::from_key(&entry.month) else {
            continue;
        };
        match years.last_mut() {
            Some((year, list)) if *year == month.year => list.push((month, entry.posts)),
            _ => years.push((month.year, vec![(month, entry.posts)])),
        }
    }

    let content = html! {
        div class="mb-6 flex flex-wrap items-baseline justify-between gap-2" {
            h1 class="text-lg font-medium" { "Archive" }
            a href=(today.href()) class="text-sm text-secondary" {
                "every " (today.label()) " →"
            }
        }

        @if years.is_empty() {
            p class="text-secondary" { "Nothing written yet." }
        } @else {
            div class="flex flex-col gap-6" {
                @for (year, list) in &years {
                    // The anchor `/archive/{year}` redirects to. A truncated URL
                    // lands on the year it named rather than the top of the page.
                    section id=(format!("y{year}")) {
                        h2 class="mb-2 text-sm text-secondary" { (year) }
                        ul class="flex flex-wrap gap-2" {
                            @for (month, posts) in list { (month_chip(*month, *posts)) }
                        }
                    }
                }
            }
        }
    };

    super::layout::render(assets, &page, content)
}

/// One calendar month.
pub fn archive_month(
    assets: &Assets,
    origin: &str,
    month: YearMonth,
    rows: &[FeedRow],
    older: Option<Cursor>,
    is_first_page: bool,
) -> Markup {
    let label = month.label();
    let href = month.href();
    let title = format!("{label} — youwin.dev");
    let description = format!("Posts from {label}.");

    let mut page = super::layout::Page::new(
        &title,
        &description,
        // The padded path, so `/archive/2026/8` and `/archive/2026/08` are one
        // page rather than two with the same content.
        format!("{origin}{href}"),
    );
    page.noindex = !is_first_page;

    let content = html! {
        div class="mb-6 flex items-baseline justify-between" {
            h1 class="text-lg font-medium" { (label) }
            a href="/archive" class="text-sm text-secondary" { "all months" }
        }

        div class="flex flex-col gap-4" {
            @for row in rows { (post::feed_item(row)) }
        }

        (pager(
            &href,
            older.map(|c| format!("{href}?before={}", c.encode())),
            is_first_page,
        ))
    };

    super::layout::render(assets, &page, content)
}

/// One day of the year, across every year the archive covers.
///
/// Unlike a month, this page is worth serving empty: there are 366 of them and
/// no more, so it is not a space a crawler can walk into an unbounded set of
/// thin pages — which is the reason an unused *tag* is a 404. "Nothing on this
/// day yet" is also a true and stable answer that becomes false by itself.
pub fn on_this_day(
    assets: &Assets,
    origin: &str,
    day: MonthDay,
    rows: &[FeedRow],
    older: Option<Cursor>,
    is_first_page: bool,
) -> Markup {
    let label = day.label();
    let href = day.href();
    let title = format!("{label} — youwin.dev");
    let description = format!("Everything written on {label}, in any year.");

    let mut page = super::layout::Page::new(&title, &description, format!("{origin}{href}"));
    page.noindex = !is_first_page || rows.is_empty();

    let content = html! {
        div class="mb-6 flex items-baseline justify-between" {
            h1 class="text-lg font-medium" { (label) }
            a href="/archive" class="text-sm text-secondary" { "archive" }
        }

        @if rows.is_empty() {
            p class="text-secondary" {
                "Nothing written on " (label) " — yet."
            }
        } @else {
            @if is_first_page {
                p class="mb-4 text-sm text-secondary" {
                    "Every " (label) ", newest first."
                }
            }
            div class="flex flex-col gap-4" {
                @for row in rows { (post::feed_item(row)) }
            }
        }

        (pager(
            &href,
            older.map(|c| format!("{href}?before={}", c.encode())),
            is_first_page,
        ))
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
    // Anything not public stays out of the index. Unlisted is reachable by link
    // but must never be indexed — that is the entire difference from `public`.
    // Drafts only reach this template through the authenticated `/preview`
    // route, and must carry it too.
    page.noindex = focused.visibility != Visibility::Public;

    let content = html! {
        div class="flex flex-col gap-3" {
            // Depth-first, so a reply sits under what it answered. The rows come
            // out of the database oldest-first; `nest` is what turns that into a
            // shape rather than a list.
            @for placed in crate::thread::nest(thread) {
                (post::thread_item(placed.post, placed.post.id == focused.id, placed.depth))
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
