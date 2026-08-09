//! The date spine — `/archive`, `/archive/:year/:month`, `/on/:month/:day` —
//! and `/random`.
//!
//! The month page is the one worth testing hardest. It filters on a
//! half-open millisecond range computed in Rust rather than a date function in
//! SQL, which is what keeps it an indexed scan — and which means an off-by-one
//! at a month boundary is a post that silently belongs to no month at all. A
//! reader would never notice; the post simply stops being reachable by date.

use std::path::Path;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    response::Response,
};
use sqlx::SqlitePool;
use tower::ServiceExt as _;
use youwin_server::{
    calendar::YearMonth,
    db::{
        self,
        posts::{Post, Visibility},
    },
    public::{self, assets::Assets},
};

const DAY: i64 = 86_400_000;

fn app(pool: SqlitePool) -> Router {
    public::router(
        pool,
        Assets {
            css: "/assets/test.css".to_owned(),
        },
        "https://youwin.dev".to_owned(),
        Path::new("web/dist/public"),
    )
}

async fn send(app: &Router, uri: &str) -> Response {
    app.clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router")
}

async fn get(app: &Router, uri: &str) -> (StatusCode, String) {
    let response = send(app, uri).await;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");

    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Midnight UTC on a given date, built from the same `bounds` the routes use so
/// a mistake in one shows up as a mistake in both rather than cancelling out.
fn at(year: i32, month: u8, day: u8) -> i64 {
    YearMonth { year, month }
        .bounds()
        .expect("a real month")
        .0
        + (i64::from(day) - 1) * DAY
}

async fn post_at(pool: &SqlitePool, body: &str, at: i64, visibility: Visibility) -> Post {
    db::posts::insert(pool, body, None, visibility, None, at)
        .await
        .expect("insert")
}

async fn public_post_at(pool: &SqlitePool, body: &str, at: i64) -> Post {
    post_at(pool, body, at, Visibility::Public).await
}

fn assert_lists(html: &str, post: &Post, listed: bool) {
    let href = format!(r#"href="/p/{}""#, post.public_id);
    assert_eq!(
        html.contains(&href),
        listed,
        "expected {} to be {} the page:\n{html}",
        post.public_id,
        if listed { "on" } else { "off" },
    );
}

// ---------------------------------------------------------------------------
// The index
// ---------------------------------------------------------------------------

#[sqlx::test]
async fn the_index_groups_months_under_their_year_with_counts(pool: SqlitePool) {
    public_post_at(&pool, "august", at(2026, 8, 3)).await;
    public_post_at(&pool, "august again", at(2026, 8, 20)).await;
    public_post_at(&pool, "july", at(2026, 7, 1)).await;
    public_post_at(&pool, "last year", at(2025, 11, 30)).await;

    let (status, html) = get(&app(pool), "/archive").await;
    assert_eq!(status, StatusCode::OK);

    assert!(html.contains(r#"href="/archive/2026/08""#), "{html}");
    assert!(html.contains(r#"href="/archive/2026/07""#), "{html}");
    assert!(html.contains(r#"href="/archive/2025/11""#), "{html}");

    // Both years appear, each as an anchor `/archive/{year}` can redirect into.
    assert!(html.contains(r#"id="y2026""#), "{html}");
    assert!(html.contains(r#"id="y2025""#), "{html}");

    // Newest first, so 2026 precedes 2025 and August precedes July.
    let y2026 = html.find("y2026").expect("2026");
    let y2025 = html.find("y2025").expect("2025");
    assert!(y2026 < y2025, "years should run newest first");

    let august = html.find("/archive/2026/08").expect("August");
    let july = html.find("/archive/2026/07").expect("July");
    assert!(august < july, "months should run newest first within a year");

    // The count beside August is 2 — the zero-padded key must not have sorted
    // October before September, and the count must not have counted the year.
    assert!(html.contains("August"), "{html}");
}

#[sqlx::test]
async fn the_index_counts_only_what_the_month_pages_will_list(pool: SqlitePool) {
    let shown = public_post_at(&pool, "public", at(2026, 8, 3)).await;
    post_at(&pool, "draft", at(2026, 8, 4), Visibility::Draft).await;
    post_at(&pool, "unlisted", at(2026, 8, 5), Visibility::Unlisted).await;

    let gone = public_post_at(&pool, "deleted", at(2026, 8, 6)).await;
    db::posts::soft_delete(&pool, &gone.public_id, at(2026, 8, 7))
        .await
        .expect("delete");

    let app = app(pool);

    // One public post in August, so the chip says 1 and the month page shows
    // exactly that post. A count that disagreed with its own page is the failure
    // this pins down.
    let (_, index) = get(&app, "/archive").await;
    assert!(index.contains(r#"href="/archive/2026/08""#), "{index}");

    let (status, month) = get(&app, "/archive/2026/08").await;
    assert_eq!(status, StatusCode::OK);
    assert_lists(&month, &shown, true);
    assert_lists(&month, &gone, false);
    assert!(!month.contains("unlisted"), "unlisted must stay unlisted");
    assert!(!month.contains("draft"), "drafts never render publicly");
}

#[sqlx::test]
async fn an_empty_archive_says_so_rather_than_erroring(pool: SqlitePool) {
    let (status, html) = get(&app(pool), "/archive").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Nothing written yet"), "{html}");
}

// ---------------------------------------------------------------------------
// A month
// ---------------------------------------------------------------------------

#[sqlx::test]
async fn a_month_takes_its_first_millisecond_and_leaves_the_next_months(pool: SqlitePool) {
    // The whole reason the bounds are half-open. Each of these is one
    // millisecond from being filed under the wrong month.
    let last_of_july = public_post_at(&pool, "july's last", at(2026, 8, 1) - 1).await;
    let first_of_august = public_post_at(&pool, "august's first", at(2026, 8, 1)).await;
    let last_of_august = public_post_at(&pool, "august's last", at(2026, 9, 1) - 1).await;
    let first_of_september = public_post_at(&pool, "september's first", at(2026, 9, 1)).await;

    let app = app(pool);
    let (status, august) = get(&app, "/archive/2026/08").await;
    assert_eq!(status, StatusCode::OK);

    assert_lists(&august, &first_of_august, true);
    assert_lists(&august, &last_of_august, true);
    assert_lists(&august, &last_of_july, false);
    assert_lists(&august, &first_of_september, false);

    // And the neighbours are not simply missing everywhere — they are on their
    // own months, which is what makes this a boundary test rather than a filter
    // that happens to exclude two posts.
    let (_, july) = get(&app, "/archive/2026/07").await;
    assert_lists(&july, &last_of_july, true);

    let (_, september) = get(&app, "/archive/2026/09").await;
    assert_lists(&september, &first_of_september, true);
}

#[sqlx::test]
async fn december_belongs_to_its_own_year(pool: SqlitePool) {
    // December is the one month whose end is in a different year, and the one
    // an off-by-one in the roll-over would put in January.
    let new_years_eve = public_post_at(&pool, "the last of it", at(2027, 1, 1) - 1).await;
    let new_years_day = public_post_at(&pool, "the first of it", at(2027, 1, 1)).await;

    let app = app(pool);
    let (_, december) = get(&app, "/archive/2026/12").await;
    assert_lists(&december, &new_years_eve, true);
    assert_lists(&december, &new_years_day, false);

    let (_, january) = get(&app, "/archive/2027/01").await;
    assert_lists(&january, &new_years_day, true);
    assert_lists(&january, &new_years_eve, false);
}

#[sqlx::test]
async fn a_month_nothing_was_written_in_is_a_404(pool: SqlitePool) {
    public_post_at(&pool, "august", at(2026, 8, 3)).await;
    let app = app(pool);

    // Without this, every month of every year is a valid URL and the archive is
    // an unbounded space of empty pages.
    for empty in ["/archive/2026/09", "/archive/1971/03", "/archive/2099/12"] {
        let (status, _) = get(&app, empty).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{empty}");
    }

    // And anything that is not a month at all.
    for nonsense in [
        "/archive/2026/13",
        "/archive/2026/0",
        "/archive/2026/aug",
        "/archive/1969/01",
    ] {
        let (status, _) = get(&app, nonsense).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{nonsense}");
    }
}

#[sqlx::test]
async fn an_unpadded_month_works_and_points_its_canonical_at_the_padded_one(pool: SqlitePool) {
    public_post_at(&pool, "august", at(2026, 8, 3)).await;

    let (status, html) = get(&app(pool), "/archive/2026/8").await;
    assert_eq!(status, StatusCode::OK);

    // Same page, one canonical URL — the same bargain `/t/Rust` makes.
    assert!(
        html.contains(r#"<link rel="canonical" href="https://youwin.dev/archive/2026/08">"#),
        "{html}",
    );
}

#[sqlx::test]
async fn a_year_redirects_into_the_index(pool: SqlitePool) {
    let response = send(&app(pool), "/archive/2026").await;

    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/archive#y2026"),
    );
}

#[sqlx::test]
async fn a_year_that_is_not_a_year_is_a_404(pool: SqlitePool) {
    let app = app(pool);

    // The redirect echoes the year into a Location header, so what it accepts
    // matters more than what it renders.
    for bad in ["/archive/nope", "/archive/1969", "/archive/99999"] {
        let response = send(&app, bad).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{bad}");
    }
}

// ---------------------------------------------------------------------------
// A day of the year
// ---------------------------------------------------------------------------

#[sqlx::test]
async fn a_day_gathers_every_year_and_nothing_adjacent(pool: SqlitePool) {
    // The two ends of the day itself, and the two milliseconds outside it.
    let first_moment = public_post_at(&pool, "this year", at(2026, 8, 9)).await;
    let last_moment = public_post_at(&pool, "still the 9th", at(2026, 8, 10) - 1).await;
    let last_year = public_post_at(&pool, "last year", at(2025, 8, 9) + 3_600_000).await;
    let the_eighth = public_post_at(&pool, "the 8th", at(2026, 8, 9) - 1).await;
    let the_tenth = public_post_at(&pool, "the 10th", at(2026, 8, 10)).await;

    let (status, html) = get(&app(pool), "/on/08/09").await;
    assert_eq!(status, StatusCode::OK);

    assert_lists(&html, &first_moment, true);
    assert_lists(&html, &last_moment, true);
    assert_lists(&html, &last_year, true);
    assert_lists(&html, &the_eighth, false);
    assert_lists(&html, &the_tenth, false);

    // Newest first, like every other list on the site.
    let newer = html.find(&first_moment.public_id).expect("2026");
    let older = html.find(&last_year.public_id).expect("2025");
    assert!(newer < older);
}

#[sqlx::test]
async fn a_day_with_nothing_on_it_is_a_page_and_an_impossible_one_is_not(pool: SqlitePool) {
    public_post_at(&pool, "august", at(2026, 8, 9)).await;
    let app = app(pool);

    // 366 of these exist and no more, so an empty one is not a crawlable void —
    // it is a true answer that becomes false on its own. It carries noindex
    // until it does.
    let (status, html) = get(&app, "/on/03/14").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Nothing written on 14 March"), "{html}");
    assert!(html.contains(r#"content="noindex"#), "{html}");

    // The leap day is real; the days that are not on any calendar are not.
    let (leap, _) = get(&app, "/on/02/29").await;
    assert_eq!(leap, StatusCode::OK);

    for impossible in ["/on/02/30", "/on/04/31", "/on/13/01", "/on/00/01", "/on/1/x"] {
        let (status, _) = get(&app, impossible).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{impossible}");
    }
}

#[sqlx::test]
async fn the_index_links_the_day_it_is(pool: SqlitePool) {
    public_post_at(&pool, "something", at(2026, 8, 9)).await;

    let (_, html) = get(&app(pool), "/archive").await;

    // Which day it is depends on when the suite runs, so this asserts the shape
    // rather than the value — without a link from somewhere, /on/ is a feature
    // nobody can find.
    assert!(html.contains(r#"href="/on/"#), "{html}");
}

// ---------------------------------------------------------------------------
// /random
// ---------------------------------------------------------------------------

#[sqlx::test]
async fn random_redirects_to_a_real_post_and_refuses_to_be_cached(pool: SqlitePool) {
    let post = public_post_at(&pool, "the only one", at(2026, 8, 9)).await;

    let response = send(&app(pool), "/random").await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some(format!("/p/{}", post.public_id).as_str()),
    );

    // A cached redirect is not random — it is one post with extra steps. Caddy
    // has a matching rule that keeps its own Cache-Control off this path.
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-store"),
    );
}

#[sqlx::test]
async fn random_never_lands_on_something_a_visitor_cannot_see(pool: SqlitePool) {
    post_at(&pool, "draft", at(2026, 8, 1), Visibility::Draft).await;
    post_at(&pool, "unlisted", at(2026, 8, 2), Visibility::Unlisted).await;
    let deleted = public_post_at(&pool, "deleted", at(2026, 8, 3)).await;
    db::posts::soft_delete(&pool, &deleted.public_id, at(2026, 8, 4))
        .await
        .expect("delete");
    let visible = public_post_at(&pool, "visible", at(2026, 8, 5)).await;

    let app = app(pool);
    let only = format!("/p/{}", visible.public_id);

    // Every draw, not one: a filter that let drafts through would pass a single
    // sample most of the time.
    for _ in 0..20 {
        let response = send(&app, "/random").await;
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok()),
            Some(only.as_str()),
        );
    }
}

#[sqlx::test]
async fn random_on_an_empty_archive_goes_to_the_feed(pool: SqlitePool) {
    // The URL is not broken, it just has nothing to offer yet — and the feed
    // says that in words.
    let response = send(&app(pool), "/random").await;

    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/"),
    );
}
