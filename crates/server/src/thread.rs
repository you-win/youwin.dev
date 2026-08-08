//! Turning a thread's flat rows into the shape a reader sees.
//!
//! `parent_id` has been recorded since the first migration and read by nothing:
//! both surfaces selected a thread by `root_id`, ordered it by time, and printed
//! it. A reply to the third post therefore landed at the bottom, under whatever
//! had been written since, with no way to tell what it answered.
//!
//! This is the one place that reads it back. Deliberately shared rather than
//! written twice: the public archive and the authoring app must agree about the
//! shape of a thread, or the composer shows you something other than what you
//! are publishing — and the interesting cases here (a reply whose parent was
//! deleted) are exactly the ones two implementations would drift on.

use std::collections::HashMap;

use crate::db::posts::{AuthoredRow, Post};

/// What [`nest`] needs of a row: who it is, and who it answered.
///
/// A trait rather than two copies of the algorithm, because the public site
/// reads [`Post`] and the authoring API reads [`AuthoredRow`], and the whole
/// point is that they cannot disagree.
pub trait Threaded {
    fn id(&self) -> i64;
    fn parent_id(&self) -> Option<i64>;
}

impl Threaded for Post {
    fn id(&self) -> i64 {
        self.id
    }

    fn parent_id(&self) -> Option<i64> {
        self.parent_id
    }
}

impl Threaded for AuthoredRow {
    fn id(&self) -> i64 {
        self.post.id
    }

    fn parent_id(&self) -> Option<i64> {
        self.post.parent_id
    }
}

/// A post in reading order, with how deep in the thread it sits.
#[derive(Debug, Clone, Copy)]
pub struct Placed<'a, T> {
    pub post: &'a T,
    /// 0 for the thread root, and for anything that answered a post which is no
    /// longer visible. See [`nest`].
    pub depth: usize,
}

/// Orders a thread depth-first: every reply immediately under what it answered,
/// siblings in the order they were written.
///
/// Input must already be sorted oldest-first — every query that feeds this one
/// orders by `created_at ASC, id ASC` — which is what makes siblings come out in
/// the order they were written without a second sort here.
///
/// Two cases are worth stating, because both are reachable today and both would
/// otherwise lose posts:
///
/// **Orphans.** Deleting a *root* takes its whole thread with it, but deleting a
/// mid-thread reply deletes only that row, and its children survive pointing at
/// a tombstone. Those come back at depth 0. Reconstructing where they used to
/// hang would mean selecting deleted rows to walk the chain through them, which
/// puts deleted posts in front of the view layer to render something nobody can
/// read the context of anyway. Back to the top level is what the flat rendering
/// effectively did, and it is honest: the post they answered is gone.
///
/// **Anything unreachable.** `parent_id` is set once at insert and never
/// updated, so a cycle should not be constructible — but a post silently
/// vanishing from its own thread is a far worse failure than one being
/// mis-indented, so whatever the walk does not reach is appended at depth 0
/// rather than dropped. The returned slice always has as many entries as the
/// input.
pub fn nest<T: Threaded>(posts: &[T]) -> Vec<Placed<'_, T>> {
    // Children keyed by parent, each list in input order.
    let mut children: HashMap<i64, Vec<usize>> = HashMap::new();
    let present: HashMap<i64, usize> = posts
        .iter()
        .enumerate()
        .map(|(index, post)| (post.id(), index))
        .collect();

    // Top level is the root plus every orphan — a post whose parent is not in
    // the visible set is not distinguishable here from one that never had a
    // parent, and both belong at depth 0.
    let mut roots: Vec<usize> = Vec::new();

    for (index, post) in posts.iter().enumerate() {
        match post.parent_id() {
            Some(parent) if present.contains_key(&parent) => {
                children.entry(parent).or_default().push(index);
            }
            _ => roots.push(index),
        }
    }

    let mut placed: Vec<Placed<'_, T>> = Vec::with_capacity(posts.len());
    let mut seen = vec![false; posts.len()];

    // An explicit stack rather than recursion: nothing bounds a thread's depth
    // except how many replies were written, and this is reachable from a public
    // URL.
    let mut stack: Vec<(usize, usize)> = roots
        .iter()
        .rev()
        .map(|&index| (index, 0usize))
        .collect();

    while let Some((index, depth)) = stack.pop() {
        if std::mem::replace(&mut seen[index], true) {
            continue;
        }
        placed.push(Placed {
            post: &posts[index],
            depth,
        });

        if let Some(kids) = children.get(&posts[index].id()) {
            // Reversed on the way in, so they come off the stack oldest-first.
            for &child in kids.iter().rev() {
                stack.push((child, depth + 1));
            }
        }
    }

    // The cycle guard described above. Input order, at the top level.
    for (index, post) in posts.iter().enumerate() {
        if !seen[index] {
            placed.push(Placed { post, depth: 0 });
        }
    }

    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for a row, so these tests state the tree and nothing else.
    struct Row {
        id: i64,
        parent_id: Option<i64>,
    }

    impl Threaded for Row {
        fn id(&self) -> i64 {
            self.id
        }

        fn parent_id(&self) -> Option<i64> {
            self.parent_id
        }
    }

    /// `(id, parent)` pairs in the order the query would return them.
    fn rows(pairs: &[(i64, Option<i64>)]) -> Vec<Row> {
        pairs
            .iter()
            .map(|&(id, parent_id)| Row { id, parent_id })
            .collect()
    }

    fn shape(posts: &[Row]) -> Vec<(i64, usize)> {
        nest(posts)
            .into_iter()
            .map(|placed| (placed.post.id, placed.depth))
            .collect()
    }

    #[test]
    fn a_flat_thread_is_unchanged() {
        // Every reply answers the root, which is what the composer produced
        // before there was anywhere else to reply from.
        let posts = rows(&[(1, None), (2, Some(1)), (3, Some(1)), (4, Some(1))]);
        assert_eq!(shape(&posts), [(1, 0), (2, 1), (3, 1), (4, 1)]);
    }

    #[test]
    fn a_reply_sits_under_what_it_answered_not_at_the_end() {
        // The reported case: 4 answers 2, and was written last.
        let posts = rows(&[(1, None), (2, Some(1)), (3, Some(1)), (4, Some(2))]);
        assert_eq!(shape(&posts), [(1, 0), (2, 1), (4, 2), (3, 1)]);
    }

    #[test]
    fn siblings_keep_the_order_they_were_written() {
        let posts = rows(&[(1, None), (2, Some(1)), (3, Some(1)), (4, Some(1))]);
        let order: Vec<i64> = shape(&posts).iter().map(|&(id, _)| id).collect();
        assert_eq!(order, [1, 2, 3, 4]);
    }

    #[test]
    fn depth_keeps_going_for_a_long_chain() {
        // Nothing clamps here; the view does, so that a deep thread is still
        // readable on a phone without this losing the real structure.
        let posts = rows(&[(1, None), (2, Some(1)), (3, Some(2)), (4, Some(3)), (5, Some(4))]);
        assert_eq!(shape(&posts), [(1, 0), (2, 1), (3, 2), (4, 3), (5, 4)]);
    }

    #[test]
    fn a_reply_to_a_deleted_post_comes_back_at_the_top_level() {
        // 2 was deleted, so it is not in the visible set; 3 answered it and 4
        // answered 3. The subtree survives, re-rooted rather than dropped.
        let posts = rows(&[(1, None), (3, Some(2)), (4, Some(3))]);
        assert_eq!(shape(&posts), [(1, 0), (3, 0), (4, 1)]);
    }

    #[test]
    fn orphans_land_after_the_whole_surviving_tree() {
        // 2 was deleted. 5 answered it, and 6 answered 5. The root still has
        // live replies of its own (3, and 4 under it), and those come first —
        // an orphan joins the top level, so it sorts after the root, and the
        // root drags its entire subtree along before anything else is reached.
        let posts = rows(&[
            (1, None),
            (3, Some(1)),
            (4, Some(3)),
            (5, Some(2)),
            (6, Some(5)),
        ]);
        assert_eq!(shape(&posts), [(1, 0), (3, 1), (4, 2), (5, 0), (6, 1)]);
    }

    #[test]
    fn several_orphans_keep_their_own_order() {
        // Both 4 and 5 answered the deleted 2. They stay in the order they were
        // written rather than in whatever order the map happened to iterate.
        let posts = rows(&[(1, None), (4, Some(2)), (5, Some(2))]);
        assert_eq!(shape(&posts), [(1, 0), (4, 0), (5, 0)]);
    }

    #[test]
    fn nothing_is_ever_dropped_even_by_a_cycle() {
        // Not constructible through `insert` — parent_id is written once and
        // never updated — but the guard is the reason a corrupt row cannot make
        // a post disappear from its own permalink.
        let posts = rows(&[(1, None), (2, Some(3)), (3, Some(2))]);
        let shaped = shape(&posts);
        assert_eq!(shaped.len(), posts.len(), "{shaped:?}");

        let mut ids: Vec<i64> = shaped.iter().map(|&(id, _)| id).collect();
        ids.sort_unstable();
        assert_eq!(ids, [1, 2, 3]);
    }

    #[test]
    fn an_empty_thread_is_empty() {
        assert!(nest(&rows(&[])).is_empty());
    }

    #[test]
    fn a_lone_root_is_one_post_at_depth_zero() {
        assert_eq!(shape(&rows(&[(1, None)])), [(1, 0)]);
    }
}
