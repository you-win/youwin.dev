//! The face: what the writer most recently sounded like.
//!
//! Two sources, in order. A mood picked in the composer is stored on the post
//! and wins outright. A post with none gets one inferred from its text, so an
//! archive written without ever touching the picker still has a pet with a face.

use crate::familiar::{Morsel, Mood, mentions};

/// Vocabularies for inferring a mood from a post that has none stored.
///
/// The prototype's lists, minus the words too common to mean anything —
/// `chaos` claimed "what" and "why", which between them fire on a large
/// fraction of ordinary sentences. `🔥` is left to `excited` alone rather than
/// being shared with `chaos`, where it only ever produced ties.
fn keywords(mood: Mood) -> &'static [&'static str] {
    match mood {
        Mood::Content => &["nice", "good", "happy", "love", "wonderful", "great", "beautiful", "❤"],
        Mood::Contemplative => &["think", "wonder", "maybe", "perhaps", "consider", "reflect", "hmm", "🤔"],
        Mood::Tired => &["tired", "exhausted", "slept", "finally", "bed", "😴", "zzz"],
        Mood::Excited => &["wow", "amazing", "incredible", "breakthrough", "shipped", "launch", "🎉", "🔥"],
        Mood::Melancholy => &["sad", "miss", "gone", "fade", "memories", "😔", "🌧"],
        Mood::Chaos => &["wtf", "broken", "crazy", "panic", "cursed", "error", "crash"],
        // The fallback. Nothing spells it — it is what is left when nothing else
        // matches.
        Mood::Neutral => &[],
    }
}

/// One post's mood.
///
/// A stored mood is the writer saying so, and no amount of cheerful vocabulary
/// should talk the pet out of it. That includes a stored `Neutral`, which means
/// "nothing to report" and deliberately suppresses inference — otherwise there
/// would be no way to tell the pet that a post about a crash was not a crisis.
pub fn detect(post: &Morsel) -> Mood {
    post.mood.unwrap_or_else(|| infer(&post.body_text))
}

/// The mood a post's text suggests, for a post that has none stored.
///
/// The strongest match wins; ties break toward the earlier mood in [`Mood::ALL`],
/// which is stable but arbitrary and not worth more machinery than that.
pub fn infer(body_text: &str) -> Mood {
    let text = body_text.to_lowercase();

    let mut best = Mood::Neutral;
    let mut best_score = 0;

    for mood in Mood::ALL {
        let score = keywords(mood)
            .iter()
            .filter(|keyword| mentions(&text, keyword))
            .count();
        if score > best_score {
            best_score = score;
            best = mood;
        }
    }

    best
}

/// The mood the pet wears, from the most recent posts.
///
/// **Last visible emotion, not an average.** Averaging produces "40% content,
/// 60% tired", which has no face — the eyes and mouth tables are keyed on one
/// mood, and a blend of two would have to render as neither. Taking the most
/// recent post that expressed anything at all means the pet is an honest
/// snapshot of where the writer just was, and a run of unremarkable posts
/// leaves the previous feeling on its face rather than washing it out.
pub fn latest(recent: &[Morsel]) -> Mood {
    recent
        .iter()
        .rev()
        .map(detect)
        .find(|mood| *mood != Mood::Neutral)
        .unwrap_or(Mood::Neutral)
}

/// How the archive's moods divide up, as fractions summing to 1.0, commonest
/// first. Empty for an empty archive.
///
/// Unlike [`latest`], this counts `Neutral` — most posts are not about a
/// feeling, and a distribution that hid them would claim otherwise.
pub fn distribution(posts: &[Morsel]) -> Vec<(Mood, f64)> {
    if posts.is_empty() {
        return Vec::new();
    }

    let mut counts = [0usize; Mood::ALL.len()];
    for post in posts {
        counts[detect(post).index()] += 1;
    }

    let total = posts.len() as f64;
    let mut ranked: Vec<_> = Mood::ALL
        .into_iter()
        .zip(counts)
        .filter(|(_, count)| *count > 0)
        .map(|(mood, count)| (mood, count as f64 / total))
        .collect();
    ranked.sort_by(|(_, a), (_, b)| b.total_cmp(a));
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::familiar::fixture::{HOUR, START, post, post_feeling};

    #[test]
    fn a_stored_mood_beats_the_words_around_it() {
        // Every content keyword in the language, and one picked mood that
        // overrules them.
        let picked = post_feeling(START, "a nice good happy wonderful great day", Mood::Tired);
        assert_eq!(detect(&picked), Mood::Tired);

        let unpicked = post(START, "a nice good happy wonderful great day");
        assert_eq!(detect(&unpicked), Mood::Content);
    }

    #[test]
    fn a_stored_neutral_turns_inference_off() {
        // The distinction the nullable column exists for: "I did not say" infers,
        // "I said nothing to report" does not. Without it there is no way to tell
        // the pet that a post about a broken deploy was not a crisis.
        let text = "the deploy is broken, everything is on fire";

        assert_eq!(detect(&post(START, text)), Mood::Chaos);
        assert_eq!(detect(&post_feeling(START, text, Mood::Neutral)), Mood::Neutral);
    }

    #[test]
    fn a_hashtag_no_longer_overrides_what_a_post_says() {
        // `#tired` used to win outright over any amount of contrary vocabulary.
        // It is an ordinary tag now, so the text decides.
        let tagged = post(START, "an amazing incredible breakthrough #tired");
        assert_eq!(detect(&tagged), Mood::Excited);

        // It does still count as the *word* it spells, like any other word in
        // the post — which is why an old `#tired` post usually still infers
        // tired, backfill or not.
        assert_eq!(detect(&post(START, "done for today #tired")), Mood::Tired);
    }

    #[test]
    fn the_most_recent_feeling_is_the_one_worn() {
        let posts = [
            post_feeling(START, "shipped it", Mood::Excited),
            post(START + HOUR, "exhausted, going to bed"),
            post(START + 2 * HOUR, "the build is at 3 of 40"),
        ];
        // The last post says nothing, so the pet keeps the tiredness before it
        // rather than resetting to neutral.
        assert_eq!(latest(&posts), Mood::Tired);
    }

    #[test]
    fn an_archive_with_nothing_to_say_is_neutral() {
        assert_eq!(latest(&[]), Mood::Neutral);
        assert_eq!(latest(&[post(START, "the build is at 3 of 40")]), Mood::Neutral);
    }

    #[test]
    fn the_distribution_counts_every_post_including_the_quiet_ones() {
        let posts = [
            post_feeling(START, "one", Mood::Excited),
            post_feeling(START + HOUR, "two", Mood::Excited),
            post(START + 2 * HOUR, "the build is at 3 of 40"),
            post(START + 3 * HOUR, "another ordinary note"),
        ];

        let spread = distribution(&posts);
        assert_eq!(spread.len(), 2);
        assert_eq!(spread[0].0, Mood::Excited);
        assert!((spread[0].1 - 0.5).abs() < 1e-9, "{spread:?}");
        assert_eq!(spread[1].0, Mood::Neutral);
        assert!((spread.iter().map(|(_, share)| share).sum::<f64>() - 1.0).abs() < 1e-9);

        assert!(distribution(&[]).is_empty());
    }
}
