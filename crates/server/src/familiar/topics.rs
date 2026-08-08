//! What the archive is about, by keyword.
//!
//! Keywords rather than anything cleverer because this runs behind a five-minute
//! cache on a machine with no model to load: matching is linear in the text,
//! deterministic, and the table is legible enough that a wrong classification can
//! be explained by pointing at a line.

use crate::familiar::{Blend, Morsel, Topic, mentions};

/// The vocabularies, straight from the design's topic table.
///
/// Deliberately shorter than the prototype's lists. Those matched as bare
/// substrings, which quietly made `fn` fire on "often", `gen` on "gentle" and
/// `xp` on "experience" — the curated set below plus the word-boundary rule in
/// [`mentions`] costs a little recall and removes a whole class of nonsense.
fn keywords(topic: Topic) -> &'static [&'static str] {
    match topic {
        Topic::Tech => &[
            "rust", "code", "server", "deploy", "api", "cli", "git", "config", "refactor", "bug",
            "compile", "struct", "trait", "tokio",
        ],
        Topic::Nature => &[
            "hike", "trail", "tree", "rain", "garden", "plant", "mountain", "sky", "walk",
            "forest", "flower", "moss", "dawn",
        ],
        Topic::Games => &[
            "game", "boss", "grind", "raid", "roguelike", "pixel", "hecs", "ecs", "playtest",
            "spawn", "loot", "xp", "permadeath",
        ],
        Topic::Art => &[
            "draw", "sketch", "paint", "palette", "canvas", "song", "melody", "photo", "poem",
            "brush", "color", "shape", "tone", "render",
        ],
        Topic::Abstract => &[
            "think", "wonder", "meaning", "time", "dream", "consciousness", "infinite", "void",
            "pattern", "theory", "paradox", "recursion",
        ],
        Topic::Daily => &[
            "food", "coffee", "sleep", "home", "friend", "today", "weather",
        ],
    }
}

/// The topic blend across `posts`.
///
/// **One post, one vote.** Each post's keyword hits are normalized by that
/// post's own total before being added in, so a 500-word write-up about a hike
/// and a five-word note about a bug weigh the same. Raw counts would let one
/// long post set the pet's shape for the next fifty.
///
/// Posts that match nothing are skipped rather than counted as evenly spread:
/// "no signal" is not the same as "a bit of everything", and treating it as the
/// latter would drag every blend toward uniform.
pub fn classify(posts: &[Morsel]) -> Blend {
    let mut totals = [0.0f64; Topic::ALL.len()];

    for post in posts {
        let text = post.body_text.to_lowercase();
        let mut hits = [0.0f64; Topic::ALL.len()];
        let mut total = 0.0;

        for topic in Topic::ALL {
            let count = keywords(topic)
                .iter()
                .filter(|keyword| mentions(&text, keyword))
                .count() as f64;
            hits[topic.index()] = count;
            total += count;
        }

        if total == 0.0 {
            continue;
        }

        for (slot, hit) in totals.iter_mut().zip(hits) {
            *slot += hit / total;
        }
    }

    Blend::from_weights(totals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::familiar::fixture::{HOUR, START, post};

    #[test]
    fn every_post_gets_one_vote_regardless_of_length() {
        let short = classify(&[
            post(START, "a bug"),
            post(START + HOUR, "hike"),
        ]);

        let lopsided = classify(&[
            post(START, "bug in the config, the server, the api, the cli, and git"),
            post(START + HOUR, "hike"),
        ]);

        // Five tech words against one nature word, and the split is still even.
        assert!((short.weight(Topic::Tech) - 0.5).abs() < 1e-9);
        assert!((lopsided.weight(Topic::Tech) - 0.5).abs() < 1e-9, "{lopsided:?}");
        assert!((lopsided.weight(Topic::Nature) - 0.5).abs() < 1e-9, "{lopsided:?}");
    }

    #[test]
    fn a_post_matching_nothing_is_skipped_not_spread_around() {
        let with_noise = classify(&[
            post(START, "one bug"),
            post(START + HOUR, "mmm"),
            post(START + 2 * HOUR, "..."),
        ]);

        assert!((with_noise.weight(Topic::Tech) - 1.0).abs() < 1e-9, "{with_noise:?}");
    }

    #[test]
    fn an_empty_archive_blends_to_nothing() {
        let blend = classify(&[]);
        assert!(blend.is_empty());
        assert_eq!(blend.primary(), None);
        assert_eq!(blend.ranked(), vec![]);
    }

    #[test]
    fn a_mixed_diet_ranks_and_finds_a_secondary() {
        let blend = classify(&[
            post(START, "rust deploy"),
            post(START + HOUR, "rust config"),
            post(START + 2 * HOUR, "rust api"),
            post(START + 3 * HOUR, "a long walk in the forest"),
        ]);

        let primary = blend.primary().expect("something matched");
        assert_eq!(primary, Topic::Tech);
        assert_eq!(blend.secondary(primary).map(|(t, _)| t), Some(Topic::Nature));
        assert_eq!(
            blend.ranked().into_iter().map(|(t, _)| t).collect::<Vec<_>>(),
            vec![Topic::Tech, Topic::Nature],
        );
    }

    #[test]
    fn daily_is_a_diffuser_it_dilutes_but_never_leads() {
        let blend = classify(&[
            post(START, "coffee at home today"),
            post(START + HOUR, "coffee, weather, a friend"),
            post(START + 2 * HOUR, "one rust bug"),
        ]);

        assert!(blend.weight(Topic::Daily) > blend.weight(Topic::Tech), "daily dominates");
        // …and the pet is still shaped by tech, because daily has no form.
        assert_eq!(blend.primary(), Some(Topic::Tech));
    }
}
