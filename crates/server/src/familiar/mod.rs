//! The Familiar — a kaomoji that reads the archive's temperature.
//!
//! A virtual pet whose face, posture and energy are derived entirely from the
//! posts already in the database. Nothing here is stored: every field is a pure
//! function of `(posts, previous state, now)`, which is what lets the whole
//! thing live on the public listener, behind the read pool, with no schema of
//! its own. See `familiar-design.md` for the design it implements.
//!
//! Five dimensions, in order of how fast they move:
//!
//! | Dimension | Reads | Shows as |
//! |---|---|---|
//! | [`Form`] | the dominant topic | the silhouette |
//! | [`Stage`] | total posts | how much silhouette there is |
//! | [`Mood`] | the most recent strong emotional signal | eyes and mouth |
//! | [`Level`] | recency and cadence of posting | frame size, motion, sleep |
//! | [`Phase`] | this hour against the learned posting rhythm | an energy offset |
//!
//! The module is split so each piece can be tested on its own: [`topics`] and
//! [`mood`] read text, [`baseline`] measures this writer's own habits and
//! [`energy`] reads clocks against them, [`render`] turns state into glyphs,
//! [`speech`] finds the one thing worth saying about all of it, [`stats`] derives
//! the numbers under the picture, and [`cache`] holds the five-minute snapshot
//! the public site actually serves.
//!
//! Every "how long has it been" question in here is asked of [`baseline`] rather
//! than of the clock, because the answer only means anything relative to the
//! person writing: eight hours is an ordinary afternoon for one archive and a
//! disappearance for another.

pub mod baseline;
pub mod cache;
pub mod energy;
pub mod mood;
pub mod render;
pub mod speech;
pub mod stats;
pub mod topics;

pub use baseline::Baseline;
pub use cache::{Familiar, Reading};

/// The mood a post was written in. Owned by the crate, not by the pet — see
/// [`crate::mood`].
pub use crate::mood::Mood;

/// One post, reduced to what the familiar reads.
///
/// Carries `FromRow` so the state machine's own input type is the row shape too,
/// while the SQL that fills it stays in `db::familiar` with every other
/// statement.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Morsel {
    pub created_at: i64,
    pub body_text: String,
    /// What the writer picked in the composer, or `None` if they picked nothing
    /// — in which case [`mood::infer`] reads the text instead.
    pub mood: Option<Mood>,
}

/// The five silhouettes. One per topic that has one — `daily` deliberately does
/// not (see [`Topic::form`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    Hexapod,
    Tendril,
    Biped,
    Orb,
    Drift,
}

impl Form {
    pub fn label(self) -> &'static str {
        match self {
            Self::Hexapod => "hexapod",
            Self::Tendril => "tendril",
            Self::Biped => "biped",
            Self::Orb => "orb",
            Self::Drift => "drift",
        }
    }
}

/// What the archive is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    Tech,
    Nature,
    Games,
    Art,
    Abstract,
    Daily,
}

impl Topic {
    pub const ALL: [Self; 6] = [
        Self::Tech,
        Self::Nature,
        Self::Games,
        Self::Art,
        Self::Abstract,
        Self::Daily,
    ];

    /// Position in a [`Blend`]'s array. Kept private so the array layout is not
    /// something callers can depend on.
    const fn index(self) -> usize {
        match self {
            Self::Tech => 0,
            Self::Nature => 1,
            Self::Games => 2,
            Self::Art => 3,
            Self::Abstract => 4,
            Self::Daily => 5,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Tech => "tech",
            Self::Nature => "nature",
            Self::Games => "games",
            Self::Art => "art",
            Self::Abstract => "abstract",
            Self::Daily => "daily",
        }
    }

    /// The silhouette this topic produces, or `None` for `daily`.
    ///
    /// `daily` is a diffuser, not a form. Everyday posts are the connective
    /// tissue of a microblog and would otherwise dominate the blend and flip the
    /// pet's shape every time someone wrote about coffee; without a form of its
    /// own it dilutes whatever is dominant instead, which is the intended effect.
    pub fn form(self) -> Option<Form> {
        match self {
            Self::Tech => Some(Form::Hexapod),
            Self::Nature => Some(Form::Tendril),
            Self::Games => Some(Form::Biped),
            Self::Art => Some(Form::Orb),
            Self::Abstract => Some(Form::Drift),
            Self::Daily => None,
        }
    }

    /// What an adult picks up when this topic is a strong secondary.
    pub fn trinket(self) -> &'static str {
        match self {
            Self::Tech => "[⌨]",
            Self::Nature => "(🌱)",
            Self::Games => "(🎮)",
            Self::Art => "(🖌)",
            Self::Abstract => "(∞)",
            Self::Daily => "(☕)",
        }
    }
}

/// A topic distribution summing to 1.0 — or to 0.0, when nothing matched.
///
/// A fixed array rather than a map: there are exactly six topics, the set is
/// closed, and `Copy` means [`PetState`] can be too.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Blend([f64; Topic::ALL.len()]);

impl Blend {
    pub fn from_weights(weights: [f64; Topic::ALL.len()]) -> Self {
        let total: f64 = weights.iter().sum();
        if total <= 0.0 {
            return Self::default();
        }
        Self(weights.map(|w| w / total))
    }

    pub fn weight(self, topic: Topic) -> f64 {
        self.0[topic.index()]
    }

    pub fn is_empty(self) -> bool {
        self.0.iter().all(|w| *w <= 0.0)
    }

    /// The heaviest topic that has a form. `None` when nothing matched, or when
    /// everything that matched was `daily`.
    pub fn primary(self) -> Option<Topic> {
        Topic::ALL
            .into_iter()
            .filter(|topic| topic.form().is_some() && self.weight(*topic) > 0.0)
            .max_by(|a, b| self.weight(*a).total_cmp(&self.weight(*b)))
    }

    /// The heaviest topic other than `primary`, whatever its weight. `daily`
    /// counts here — it has no form but it does have a trinket.
    pub fn secondary(self, primary: Topic) -> Option<(Topic, f64)> {
        Topic::ALL
            .into_iter()
            .filter(|topic| *topic != primary && self.weight(*topic) > 0.0)
            .map(|topic| (topic, self.weight(topic)))
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
    }

    /// Topics with any weight at all, heaviest first.
    pub fn ranked(self) -> Vec<(Topic, f64)> {
        let mut ranked: Vec<_> = Topic::ALL
            .into_iter()
            .map(|topic| (topic, self.weight(topic)))
            .filter(|(_, weight)| *weight > 0.0)
            .collect();
        ranked.sort_by(|(_, a), (_, b)| b.total_cmp(a));
        ranked
    }

    /// Shannon entropy in bits — how spread out the diet is. 0 when every post
    /// is about one thing; `log2(6)` when all six are equally represented.
    pub fn entropy(self) -> f64 {
        -self
            .0
            .iter()
            .filter(|w| **w > 0.0)
            .map(|w| w * w.log2())
            .sum::<f64>()
    }
}

/// Whether `needle` occurs in `haystack` at the start of a word.
///
/// `haystack` must already be lowercased. Shared by [`topics`] and [`mood`],
/// which both match short keywords against post text and both need the same
/// answer to "does "code" appear in "barcode"".
///
/// Matching the *start* of a word rather than the whole of it keeps plurals and
/// simple inflections — "bugs", "deploys", "hiking" — without a stemmer, while
/// refusing the mid-word collisions that make bare substring matching useless at
/// these keyword lengths.
///
/// A keyword that does not begin with an alphanumeric — an emoji — has no
/// meaningful word boundary in front of it, so it matches anywhere.
fn mentions(haystack: &str, needle: &str) -> bool {
    let Some(first) = needle.chars().next() else {
        return false;
    };
    if !first.is_alphanumeric() {
        return haystack.contains(needle);
    }

    haystack.match_indices(needle).any(|(at, _)| {
        haystack[..at]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_alphanumeric())
    })
}

/// Continuous energy, bucketed for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Hyper,
    Active,
    Normal,
    Lethargic,
    Bored,
}

impl Level {
    /// The thresholds from the design's energy table.
    pub fn of(energy: f64) -> Self {
        if energy >= 0.75 {
            Self::Hyper
        } else if energy >= 0.50 {
            Self::Active
        } else if energy >= 0.30 {
            Self::Normal
        } else if energy >= 0.15 {
            Self::Lethargic
        } else {
            Self::Bored
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hyper => "hyper",
            Self::Active => "active",
            Self::Normal => "normal",
            Self::Lethargic => "lethargic",
            Self::Bored => "bored",
        }
    }
}

/// This hour, judged against the hours the archive is usually written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Peak,
    Active,
    Dusk,
    Quiet,
    Deep,
}

impl Phase {
    /// Added to the decayed energy at render time — never folded into the stored
    /// value. See [`energy::step`] for why that distinction is load-bearing.
    pub fn modifier(self) -> f64 {
        match self {
            Self::Peak => 0.10,
            Self::Active => 0.00,
            Self::Dusk => -0.05,
            Self::Quiet => -0.10,
            Self::Deep => -0.15,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Peak => "peak",
            Self::Active => "active",
            Self::Dusk => "dusk",
            Self::Quiet => "quiet",
            Self::Deep => "deep",
        }
    }
}

/// How much pet there is, by total posts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Egg,
    Hatchling,
    Juvenile,
    Adult,
}

impl Stage {
    pub fn of(posts: usize) -> Self {
        match posts {
            0 => Self::Egg,
            1..=10 => Self::Hatchling,
            11..=50 => Self::Juvenile,
            _ => Self::Adult,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Egg => "egg",
            Self::Hatchling => "hatchling",
            Self::Juvenile => "juvenile",
            Self::Adult => "adult",
        }
    }

    /// The stage after this one. `None` for an adult, which is the last.
    ///
    /// The ladder is spelled out once here rather than in each place that wants
    /// to name what comes next — the feed widget, the character sheet and the
    /// authoring API all ask, and three copies of an ordering is three chances
    /// for one of them to be wrong.
    pub fn next(self) -> Option<Self> {
        match self {
            Self::Egg => Some(Self::Hatchling),
            Self::Hatchling => Some(Self::Juvenile),
            Self::Juvenile => Some(Self::Adult),
            Self::Adult => None,
        }
    }

    /// Posts needed to reach the next stage, and how far along this one is.
    /// `None` for an adult, which is the last one.
    pub fn progress(self, posts: usize) -> Option<(usize, u8)> {
        let target = match self {
            Self::Egg => 1,
            Self::Hatchling => 11,
            Self::Juvenile => 51,
            Self::Adult => return None,
        };
        let percent = (posts * 100 / target).min(100) as u8;
        Some((target, percent))
    }
}

/// Everything the renderer needs, and the whole of what gets cached.
///
/// `Copy`, deliberately: a state is five enums and three numbers, so a snapshot
/// costs nothing to hand around and there is no shared mutable pet anywhere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PetState {
    pub form: Form,
    pub stage: Stage,
    pub mood: Mood,
    pub topic: Topic,
    pub blend: Blend,
    /// Decay and bursts only, with no circadian offset applied. This is the
    /// value that carries forward to the next fast-forward.
    pub base_energy: f64,
    /// `base_energy` plus the phase modifier, clamped. What the pet looks like.
    pub energy: f64,
    pub level: Level,
    pub phase: Phase,
    pub posts: usize,
    /// Unix millis this state describes.
    pub at: i64,
}

/// Posts consulted for the topic blend. Long, so the silhouette does not flip on
/// a single off-topic post.
const TOPIC_WINDOW: usize = 50;

/// Posts consulted for mood. Short, because mood is the fast-moving channel.
const MOOD_WINDOW: usize = 10;

/// Computes the pet's state at `now`.
///
/// `posts` must be sorted oldest first, which is how `db::familiar::all` returns
/// them. Anything dated after `now` is ignored, so a caller can replay history by
/// walking `now` forward over a fixed slice — which is exactly what the tests do.
///
/// `previous` is the last state this process computed. Passing `None` is a cold
/// start: energy is estimated from the last post's age rather than carried
/// forward, which is what happens after a restart and is meant to look the same.
pub fn compute(posts: &[Morsel], previous: Option<&PetState>, now: i64) -> PetState {
    let visible = &posts[..posts.partition_point(|post| post.created_at <= now)];

    let blend = topics::classify(tail(visible, TOPIC_WINDOW));
    let topic = blend.primary().unwrap_or(Topic::Tech);
    let form = topic.form().unwrap_or(Form::Hexapod);
    let mood = mood::latest(tail(visible, MOOD_WINDOW));

    let phase = energy::phase_at(visible, now);
    let base_energy = energy::step(visible, previous, now);

    // The phase modifier is applied here, to the value about to be drawn, and is
    // never written back into `base_energy`. Folding it in would make energy a
    // function of how many times the page was loaded: every recompute in a deep
    // phase would subtract another 0.15, and every recompute during peak hours
    // would add another 0.10, so a pet could be pumped to hyper by refreshing.
    let energy = (base_energy + phase.modifier()).clamp(energy::FLOOR, energy::CEILING);

    PetState {
        form,
        stage: Stage::of(visible.len()),
        mood,
        topic,
        blend,
        base_energy,
        energy,
        level: Level::of(energy),
        phase,
        posts: visible.len(),
        at: now,
    }
}

/// The last `count` posts, or all of them.
fn tail(posts: &[Morsel], count: usize) -> &[Morsel] {
    &posts[posts.len().saturating_sub(count)..]
}

#[cfg(test)]
pub(crate) mod fixture {
    use super::Morsel;

    pub const HOUR: i64 = 3_600_000;
    pub const DAY: i64 = 24 * HOUR;

    /// 2026-08-01T00:00:00Z — a Saturday midnight, so hour-of-day arithmetic in
    /// tests reads as the hour it says.
    pub const START: i64 = 1_785_888_000_000;

    /// A post with no mood picked, so the familiar infers one from the text.
    pub fn post(at: i64, body_text: &str) -> Morsel {
        Morsel {
            created_at: at,
            body_text: body_text.to_owned(),
            mood: None,
        }
    }

    /// A post whose mood was chosen in the composer.
    pub fn post_feeling(at: i64, body_text: &str, mood: super::Mood) -> Morsel {
        Morsel {
            mood: Some(mood),
            ..post(at, body_text)
        }
    }

    /// `count` posts one hour apart from `from`, all with the same body.
    pub fn run(from: i64, count: usize, body_text: &str) -> Vec<Morsel> {
        (0..count)
            .map(|i| post(from + i as i64 * HOUR, body_text))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{fixture::*, *};

    #[test]
    fn keywords_match_at_word_starts_only() {
        assert!(mentions("shipped the rust server", "rust"));
        assert!(mentions("(rust)", "rust"), "punctuation is a boundary");
        assert!(mentions("fixing bugs today", "bug"), "a prefix match is still a hit");
        assert!(mentions("deploying at noon", "deploy"));
        assert!(mentions("shipped it 🎉", "🎉"), "an emoji has no word boundary");

        // The collisions bare substring matching could not avoid.
        assert!(!mentions("i often forget", "fn"));
        assert!(!mentions("years of experience", "xp"));
        assert!(!mentions("read the specs", "ecs"));
        assert!(!mentions("a barcode scanner", "code"));
    }

    #[test]
    fn stages_step_on_the_documented_counts() {
        assert_eq!(Stage::of(0), Stage::Egg);
        assert_eq!(Stage::of(1), Stage::Hatchling);
        assert_eq!(Stage::of(10), Stage::Hatchling);
        assert_eq!(Stage::of(11), Stage::Juvenile);
        assert_eq!(Stage::of(50), Stage::Juvenile);
        assert_eq!(Stage::of(51), Stage::Adult);
    }

    #[test]
    fn energy_levels_step_on_the_documented_thresholds() {
        assert_eq!(Level::of(1.0), Level::Hyper);
        assert_eq!(Level::of(0.75), Level::Hyper);
        assert_eq!(Level::of(0.74), Level::Active);
        assert_eq!(Level::of(0.50), Level::Active);
        assert_eq!(Level::of(0.49), Level::Normal);
        assert_eq!(Level::of(0.30), Level::Normal);
        assert_eq!(Level::of(0.29), Level::Lethargic);
        assert_eq!(Level::of(0.15), Level::Lethargic);
        assert_eq!(Level::of(0.14), Level::Bored);
        assert_eq!(Level::of(0.0), Level::Bored);
    }

    #[test]
    fn an_empty_archive_is_an_egg_and_does_not_panic() {
        let state = compute(&[], None, START);
        assert_eq!(state.stage, Stage::Egg);
        assert_eq!(state.posts, 0);
        assert!(state.blend.is_empty());
        // Something has to be drawn, and tech is the documented fallback.
        assert_eq!(state.topic, Topic::Tech);
        assert_eq!(state.form, Form::Hexapod);
    }

    #[test]
    fn posts_dated_after_now_are_not_visible_yet() {
        let posts = run(START, 20, "shipped the rust server");
        // Halfway through the run: ten posts exist, which is still a hatchling.
        let state = compute(&posts, None, START + 9 * HOUR);
        assert_eq!(state.posts, 10);
        assert_eq!(state.stage, Stage::Hatchling);

        let later = compute(&posts, None, START + 19 * HOUR);
        assert_eq!(later.posts, 20);
        assert_eq!(later.stage, Stage::Juvenile);
    }

    #[test]
    fn the_dominant_topic_picks_the_silhouette() {
        let posts = [
            post(START, "long hike up the mountain, the forest was wet"),
            post(START + HOUR, "rain on the trail, moss everywhere"),
            post(START + 2 * HOUR, "one bug in the deploy config"),
        ];
        let state = compute(&posts, None, START + 3 * HOUR);
        assert_eq!(state.topic, Topic::Nature);
        assert_eq!(state.form, Form::Tendril);
    }

    #[test]
    fn daily_never_becomes_the_form() {
        // Every post is squarely `daily`, so there is no primary with a form —
        // and the pet still has to be shaped like something.
        let posts = run(START, 5, "coffee at home today, weather is weather");
        let state = compute(&posts, None, START + 5 * HOUR);
        assert!(state.blend.weight(Topic::Daily) > 0.0);
        assert_eq!(state.topic, Topic::Tech, "the documented fallback");
    }

    #[test]
    fn refreshing_the_page_does_not_move_the_pet() {
        // The bug this exists to prevent: a phase modifier folded into the
        // stored value compounds once per recompute, so energy becomes a
        // function of traffic. Two hundred recomputes over the same minute must
        // land within rounding of one.
        let posts = run(START, 12, "rust deploy config refactor");
        let at = START + 12 * HOUR;

        let once = compute(&posts, None, at);

        let mut state = once;
        for _ in 0..200 {
            state = compute(&posts, Some(&state), at);
        }

        assert!(
            (state.energy - once.energy).abs() < 1e-9,
            "energy drifted from {} to {} after 200 recomputes",
            once.energy,
            state.energy
        );
        assert_eq!(state.level, once.level);
    }

    #[test]
    fn blend_entropy_spans_one_topic_to_all_six() {
        let single = Blend::from_weights([1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(single.entropy(), 0.0);

        let even = Blend::from_weights([1.0; 6]);
        assert!((even.entropy() - 6f64.log2()).abs() < 1e-9);
    }

    #[test]
    fn stage_progress_counts_toward_the_next_stage() {
        assert_eq!(Stage::Juvenile.progress(25), Some((51, 49)));
        assert_eq!(Stage::Hatchling.progress(5), Some((11, 45)));
        assert_eq!(Stage::Adult.progress(400), None);
    }

    #[test]
    fn the_ladder_runs_out_at_exactly_the_stage_progress_does() {
        // Two functions describing one ordering, and every caller pairs them —
        // a stage with somewhere to go must also have a percentage to show.
        for stage in [Stage::Egg, Stage::Hatchling, Stage::Juvenile, Stage::Adult] {
            assert_eq!(
                stage.next().is_some(),
                stage.progress(0).is_some(),
                "{stage:?} disagrees with itself",
            );
        }

        assert_eq!(Stage::Egg.next(), Some(Stage::Hatchling));
        assert_eq!(Stage::Juvenile.next(), Some(Stage::Adult));
        assert_eq!(Stage::Adult.next(), None);
    }
}
