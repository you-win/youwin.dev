//! Threads as a reader sees them on the public archive.
//!
//! `thread::nest` has its own unit tests over the tree itself; these cover the
//! wiring, which is the part that goes quietly wrong. A permalink that stopped
//! calling `nest` would render a perfectly good page — just the old flat one —
//! and nothing else in the suite would notice.

use std::path::Path;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use sqlx::SqlitePool;
use tower::ServiceExt as _;
use youwin_server::{
    db::{
        self,
        posts::{Post, Visibility},
    },
    public::{self, assets::Assets},
};

/// 2026-08-05T00:00:00Z. Posts are written a minute apart so "oldest first" is
/// unambiguous and never depends on the rowid tie-break.
///
/// (The date is arbitrary — nothing here depends on it. It said 08-01 for a
/// while, which is four days out, and that comment was copied into code where
/// the exact date did matter.)
const T0: i64 = 1_785_888_000_000;
const MINUTE: i64 = 60_000;

fn app(pool: SqlitePool) -> Router {
    public::router(
        pool,
        Assets {
            css: "/assets/test.css".to_owned(),
        },
        "https://youwin.dev".to_owned(),
        // Never read: nothing here requests /assets.
        Path::new("web/dist/public"),
    )
}

async fn get(app: &Router, uri: &str) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router");

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");

    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn post_at(pool: &SqlitePool, body: &str, minutes: i64, parent: Option<&Post>) -> Post {
    db::posts::insert(
        pool,
        body,
        parent.map(|p| p.id),
        Visibility::Public,
        None,
        T0 + minutes * MINUTE,
    )
    .await
    .expect("insert")
}

/// Where a post's card starts in the document. Panics rather than returning an
/// Option: every caller is asserting the post is on the page.
fn position(html: &str, post: &Post) -> usize {
    let anchor = format!(r#"id="p-{}""#, post.public_id);
    html.find(&anchor)
        .unwrap_or_else(|| panic!("{} is not on the page:\n{html}", post.public_id))
}

/// The class list on the wrapper `<div>` a post's card sits in — which is the
/// indent, and the only thing that says how deep the reply is.
fn indent_of(html: &str, post: &Post) -> String {
    let at = position(html, post);
    let open = r#"<div class=""#;
    let start = html[..at]
        .rfind(open)
        .expect("every card is wrapped in a div")
        + open.len();
    let end = start + html[start..].find('"').expect("unterminated class");
    html[start..end].to_owned()
}

/// The order posts appear in the rendered document.
fn reading_order(html: &str, posts: &[&Post]) -> Vec<String> {
    let mut ordered: Vec<&&Post> = posts.iter().collect();
    ordered.sort_by_key(|post| position(html, post));
    ordered.iter().map(|p| p.public_id.clone()).collect()
}

#[sqlx::test]
async fn a_reply_to_an_earlier_post_renders_under_it_rather_than_at_the_end(pool: SqlitePool) {
    let root = post_at(&pool, "root", 0, None).await;
    let first = post_at(&pool, "first reply", 1, Some(&root)).await;
    let second = post_at(&pool, "second reply", 2, Some(&root)).await;
    // Written last, but it answers `first` — the case that used to land at the
    // bottom of the page with nothing saying what it replied to.
    let late = post_at(&pool, "answering the first one", 3, Some(&first)).await;

    let (status, html) = get(&app(pool), &format!("/p/{}", root.public_id)).await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(
        reading_order(&html, &[&root, &first, &second, &late]),
        vec![
            root.public_id.clone(),
            first.public_id.clone(),
            late.public_id.clone(),
            second.public_id.clone(),
        ],
        "the late reply belongs directly under the post it answered"
    );

    // And it is visibly deeper than the sibling it now sits above.
    assert_eq!(indent_of(&html, &root), "");
    assert_ne!(indent_of(&html, &first), "");
    assert_ne!(indent_of(&html, &late), indent_of(&html, &first));
    assert_eq!(indent_of(&html, &second), indent_of(&html, &first));
}

#[sqlx::test]
async fn a_thread_that_never_branched_looks_exactly_as_it_did(pool: SqlitePool) {
    // Every reply answers the root, which is every thread written before there
    // was anywhere else to reply from. Nesting must not disturb these.
    let root = post_at(&pool, "root", 0, None).await;
    let first = post_at(&pool, "first", 1, Some(&root)).await;
    let second = post_at(&pool, "second", 2, Some(&root)).await;

    let (_, html) = get(&app(pool), &format!("/p/{}", root.public_id)).await;

    assert_eq!(
        reading_order(&html, &[&root, &first, &second]),
        vec![
            root.public_id.clone(),
            first.public_id.clone(),
            second.public_id.clone(),
        ]
    );
    assert_eq!(indent_of(&html, &first), indent_of(&html, &second));
}

#[sqlx::test]
async fn a_reply_whose_parent_was_deleted_still_renders(pool: SqlitePool) {
    let root = post_at(&pool, "root", 0, None).await;
    let doomed = post_at(&pool, "this one goes away", 1, Some(&root)).await;
    let orphan = post_at(&pool, "answering the one that goes away", 2, Some(&doomed)).await;

    // Deleting a *reply* takes only that row; its children keep pointing at it.
    db::posts::soft_delete(&pool, &doomed.public_id, T0 + 10 * MINUTE)
        .await
        .expect("delete")
        .expect("the reply existed");

    let (status, html) = get(&app(pool), &format!("/p/{}", root.public_id)).await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        !html.contains(&format!(r#"id="p-{}""#, doomed.public_id)),
        "the deleted reply is gone"
    );
    // The one that answered it must not vanish with it — it comes back at the
    // top level, since the post that gave it context no longer exists.
    assert_eq!(indent_of(&html, &orphan), "");
    assert!(position(&html, &orphan) > position(&html, &root));
}

#[sqlx::test]
async fn the_indent_stops_so_a_deep_chain_stays_readable(pool: SqlitePool) {
    // Six deep. `nest` reports the true depth; the view is what clamps, so past
    // the cap consecutive replies share an indent and keep only their order.
    let root = post_at(&pool, "0", 0, None).await;
    let mut chain = vec![root.clone()];
    for step in 1..=6 {
        let parent = chain.last().unwrap().clone();
        chain.push(post_at(&pool, &step.to_string(), step, Some(&parent)).await);
    }

    let (_, html) = get(&app(pool), &format!("/p/{}", root.public_id)).await;

    let refs: Vec<&Post> = chain.iter().collect();
    let order = reading_order(&html, &refs);
    assert_eq!(
        order,
        chain.iter().map(|p| p.public_id.clone()).collect::<Vec<_>>(),
        "a straight chain reads top to bottom"
    );

    let indents: Vec<String> = chain.iter().map(|post| indent_of(&html, post)).collect();
    assert_eq!(indents[0], "", "the root is not indented");
    assert_eq!(
        indents[5], indents[6],
        "past the cap the indent stops growing: {indents:#?}"
    );
    assert_ne!(indents[3], indents[4], "and it does grow before that");
}
