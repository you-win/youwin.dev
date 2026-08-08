//! The face: what the writer most recently sounded like.

use crate::familiar::{Morsel, Mood, mentions};

/// Vocabularies for inferring a mood from a post that carries no tag.
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
        // The fallback. Nothing spells it, it is what is left when nothing else
        // matches — including when a post is explicitly tagged `#neutral`.
        Mood::Neutral => &[],
    }
}

/// One post's mood.
///
/// An explicit hashtag wins outright: `#tired` at the end of a post is the
/// writer saying so, and no amount of cheerful vocabulary should talk the pet
/// out of it. Only then does keyword inference run, and the strongest match
/// wins — ties break toward the earlier mood in [`Mood::ALL`], which is stable
/// but arbitrary and not worth more machinery than that.
pub fn detect(post: &Morsel) -> Mood {
    let text = post.body_text.to_lowercase();

    if let Some(tagged) = Mood::ALL.into_iter().find(|mood| has_tag(&text, mood.label())) {
        return tagged;
    }

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

/// Whether `text` carries `#name` as a whole tag.
///
/// `text` must already be lowercased. The tail check is what stops `#tiredness`
/// from reading as `#tired`; the rules match `render::markdown`'s, which is what
/// decided the post's real tags on the way in.
fn has_tag(text: &str, name: &str) -> bool {
    text.match_indices('#').any(|(at, _)| {
        text[at + 1..].strip_prefix(name).is_some_and(|tail| {
            tail.chars()
                .next()
                .is_none_or(|next| !next.is_alphanumeric() && next != '_' && next != '-')
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::familiar::fixture::{HOUR, START, post};

    #[test]
    fn an_explicit_tag_beats_the_words_around_it() {
        // Every content keyword in the language, and one tag that overrules them.
        let tagged = post(START, "a nice good happy wonderful great day #tired");
        assert_eq!(detect(&tagged), Mood::Tired);

        let untagged = post(START, "a nice good happy wonderful great day");
        assert_eq!(detect(&untagged), Mood::Content);
    }

    #[test]
    fn a_tag_must_be_the_whole_word() {
        assert!(has_tag("done #tired", "tired"));
        assert!(has_tag("#tired, finally", "tired"));
        assert!(!has_tag("#tiredness is real", "tired"));
        assert!(!has_tag("#tired-eyes", "tired"));
        assert!(!has_tag("just tired", "tired"), "no hash, no tag");
    }

    #[test]
    fn the_most_recent_feeling_is_the_one_worn() {
        let posts = [
            post(START, "shipped it #excited"),
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
            post(START, "#excited"),
            post(START + HOUR, "#excited"),
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
