//! State into glyphs.
//!
//! Compositional, not templated. Eyes, mouth, crown and base are looked up
//! independently and assembled, so a new mood costs five table entries rather
//! than the 700-odd hand-drawn kaomoji that every combination would otherwise
//! need. Everything here is a total function of [`PetState`] — no allocation
//! beyond the three output lines, no ordering, no I/O.

use crate::familiar::{Form, Level, Mood, PetState, Phase, Stage};

/// Weight a secondary topic needs before an adult picks up its trinket.
///
/// Below this it is background noise, and the pet would swap accessories every
/// time a stray post tipped the blend.
const TRINKET_THRESHOLD: f64 = 0.25;


/// The assembled pet: one to three lines, top to bottom, already trimmed.
///
/// Lines are held separately rather than as one string with newlines because the
/// caller centres them — a `<pre>` with `text-align: center` sidesteps having to
/// pad by character count, which would be wrong anyway: half these glyphs are
/// East Asian Ambiguous and render at a width the server cannot know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kaomoji {
    pub lines: Vec<String>,
}

impl Kaomoji {
    /// The whole pet as one string, newline separated. For tests and for the
    /// `title` attribute.
    pub fn flat(&self) -> String {
        self.lines.join("\n")
    }
}

/// Draws the pet.
pub fn kaomoji(state: &PetState) -> Kaomoji {
    let (left, right) = eyes(state.mood, state.level);
    let face = format!("({left}{}{right})", mouth(state.mood, state.level));

    let (celebrate_left, celebrate_right) = celebration(state);
    let ears = crown(state.form, state.stage, state.level);
    let body = base(state.form, state.stage, state.level);
    let face_line = format!("{face}{}", sleep(state));

    let mut lines = Vec::with_capacity(3);
    push(&mut lines, &[celebrate_left, ears, celebrate_right]);
    push(&mut lines, &[face_line.as_str()]);
    push(&mut lines, &[motion(state), body, trinket(state)]);

    Kaomoji { lines }
}

/// A description for `aria-label`. A screen reader announcing `( ◕ ω ◕ )`
/// character by character is noise, so the picture gets read out as a sentence
/// and the glyphs are hidden.
pub fn describe(state: &PetState) -> String {
    if state.stage == Stage::Egg {
        return "The familiar, still an egg: nothing has been written yet.".to_owned();
    }

    format!(
        "The familiar: a {} {}, {} and {}, in its {} hours.",
        state.stage.label(),
        state.form.label(),
        state.mood.as_str(),
        state.level.label(),
        state.phase.label(),
    )
}

/// Joins the non-empty parts with single spaces, dropping the line entirely if
/// nothing survives.
fn push(lines: &mut Vec<String>, parts: &[&str]) {
    let line = parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    if !line.is_empty() {
        lines.push(line);
    }
}

/// The eyes. The single most expressive slot, and the only one that is not
/// symmetric — `chaos` sets the two eyes differently, which is most of what
/// makes it read as chaos rather than as surprise.
fn eyes(mood: Mood, level: Level) -> (&'static str, &'static str) {
    use Level::*;

    if mood == Mood::Chaos {
        return match level {
            Hyper => ("⊙", "◉"),
            Active => ("×", "⊙"),
            Normal => ("×", "×"),
            Lethargic => ("◉", "×"),
            Bored => ("·", "_"),
        };
    }

    let eye = match mood {
        Mood::Content => match level {
            Hyper => "★",
            Active => "◕",
            Normal => "ᵔ",
            Lethargic => "˘",
            Bored => "·",
        },
        Mood::Contemplative => match level {
            Hyper => "◎",
            Active => "‾",
            Normal => "￣",
            Lethargic => "˘",
            Bored => "·",
        },
        Mood::Tired => match level {
            Hyper => "×",
            Active => "=",
            Normal => "˘",
            Lethargic => "u",
            Bored => "·",
        },
        Mood::Excited => match level {
            Hyper => "★",
            Active => "◎",
            Normal | Lethargic => "◕",
            Bored => "·",
        },
        Mood::Melancholy => match level {
            Hyper | Active => "；",
            Normal => ";",
            Lethargic => "˘",
            Bored => "·",
        },
        Mood::Neutral => match level {
            Hyper => "◕",
            Active | Normal | Lethargic => "・",
            Bored => "·",
        },
        // Handled above; listed so adding a mood is a compile error here.
        Mood::Chaos => unreachable!("chaos returns early"),
    };

    (eye, eye)
}

fn mouth(mood: Mood, level: Level) -> &'static str {
    use Level::*;

    match mood {
        Mood::Content => match level {
            Hyper => "∀",
            Active | Normal => "ω",
            Lethargic | Bored => "‿",
        },
        Mood::Contemplative => match level {
            Hyper => "□",
            Active | Normal | Lethargic => "ε",
            Bored => "·",
        },
        Mood::Tired => match level {
            Hyper => "皿",
            Active => "ω",
            Normal | Lethargic => "³",
            Bored => "z",
        },
        Mood::Excited => match level {
            Hyper => "□",
            Active => "∀",
            Normal | Lethargic => "ω",
            Bored => "·",
        },
        Mood::Melancholy => match level {
            Hyper => "益",
            Active | Normal | Lethargic => ";",
            Bored => ":",
        },
        Mood::Chaos => match level {
            Hyper => "益",
            Active => "皿",
            Normal | Lethargic => "‿",
            Bored => "_",
        },
        Mood::Neutral => match level {
            Hyper => "□",
            Active | Normal | Lethargic => "ω",
            Bored => "·",
        },
    }
}

/// Ears, antennae, whatever the form grows on top.
///
/// An egg has none, and a hatchling has one fixed pair rather than five energy
/// variants — a pet that has been written to three times has no rhythm to read.
fn crown(form: Form, stage: Stage, level: Level) -> &'static str {
    use Level::*;

    match stage {
        Stage::Egg => "",
        Stage::Hatchling => match form {
            Form::Hexapod => "┬ ┬",
            Form::Tendril => "╱ ╲",
            Form::Biped => "∩ ∩",
            Form::Orb => "",
            Form::Drift => "∵ ∴",
        },
        Stage::Juvenile | Stage::Adult => match form {
            Form::Hexapod => match level {
                Hyper => "┻ ┻",
                Active | Normal => "┬ ┬",
                Lethargic => "╷ ╷",
                Bored => "· ·",
            },
            Form::Tendril => match level {
                Hyper => "┳ ┳",
                Active | Normal => "╱ ╲",
                Lethargic => "╷ ╷",
                Bored => "· ·",
            },
            Form::Biped => match level {
                Hyper | Active | Normal => "∩ ∩",
                Lethargic => "⌣ ⌣",
                Bored => "· ·",
            },
            // An orb has no ears. What it has instead is a spark, and only once
            // it is grown enough to hold one.
            Form::Orb => match (stage, level) {
                (Stage::Adult, Hyper) => "★",
                (Stage::Adult, Active) | (Stage::Juvenile, Hyper) => "*",
                (_, Bored) => "·",
                _ => "",
            },
            Form::Drift => match level {
                Hyper => "⋰ ⋱",
                Active | Normal => "∵ ∴",
                Lethargic => "∵ ∵",
                Bored => "· ·",
            },
        },
    }
}

/// Body, arms, legs — whatever holds the pet up.
///
/// Nothing until juvenile: the design's hatchling is a face with ears and no
/// body, and an adult keeps its full posture down to lower energy than a
/// juvenile can.
fn base(form: Form, stage: Stage, level: Level) -> &'static str {
    use Level::*;

    if matches!(stage, Stage::Egg | Stage::Hatchling) {
        return "";
    }
    let adult = stage == Stage::Adult;

    match form {
        Form::Hexapod => match level {
            Hyper | Active => "╱│╲",
            Normal if adult => "╱│╲",
            Normal => "││",
            Lethargic => "│",
            Bored => "",
        },
        Form::Tendril => match level {
            Hyper => "╱╲╱╲",
            Active => "╲ ╱",
            Normal if adult => "╲ ╱",
            Normal => "│",
            Lethargic => "╵",
            Bored => "",
        },
        Form::Biped => match level {
            Hyper => "／ ＼",
            Active if adult => "／|＼",
            Active => "／ ＼",
            Normal if adult => "／|＼",
            Normal => "╰╯",
            Lethargic => "╯",
            Bored => "",
        },
        Form::Orb => match level {
            Hyper if adult => "~~~~~",
            Hyper => "~~~",
            Active if adult => "~~~",
            Active => "~~",
            Normal if adult => "~~",
            Normal => "~",
            Lethargic if adult => "~",
            Lethargic | Bored => "",
        },
        Form::Drift => match level {
            Hyper if adult => "~~~~",
            Hyper => "~~~",
            Active if adult => "~~~",
            Active => "~~",
            Normal if adult => "~~",
            Normal => "~",
            Lethargic if adult => "~",
            Lethargic => "·",
            Bored => "",
        },
    }
}

/// The pet is asleep — deep in the hours it never writes in, with nothing left
/// in the tank.
fn sleep(state: &PetState) -> &'static str {
    match (state.phase, state.level) {
        (Phase::Deep, Level::Bored | Level::Lethargic) => " zZ",
        _ => "",
    }
}

/// Corner marks. The big star is an adult at full tilt with something to be
/// excited about, and is a reading of the present. The other two are
/// [`Spark`]s — events that happened and are fading — and are the only thing
/// here that depends on when something occurred rather than on how things are.
///
/// Three glyphs rather than one because they mean different things: `★` is the
/// pet at its best, `*` is a round number, `✧` is the pet waking up from an
/// absence. A shared glyph would make the rarest of the three unrecognisable.
///
/// **This is the only place the precedence lives.** There is one pair of corners
/// and up to three things that could go in it, so something has to win — a
/// milestone over a return, because crossing fifty happens once in an archive's
/// life and there will be another chance to see a rekindling. That is a fact
/// about corners, not about the archive, which is why [`super::spark`] reports
/// both and this picks: `speech` ranks on its own terms and has no business
/// inheriting a constraint on how many glyphs fit.
fn celebration(state: &PetState) -> (&'static str, &'static str) {
    if state.stage == Stage::Adult && state.level == Level::Hyper && state.mood == Mood::Excited {
        return ("★", "★");
    }

    if state.sparks.milestone.is_some() {
        return ("*", "*");
    }
    if state.sparks.rekindled {
        return ("✧", "✧");
    }
    ("", "")
}

/// Visible vibration.
fn motion(state: &PetState) -> &'static str {
    if state.level == Level::Hyper { "⚡" } else { "" }
}

/// What the pet is carrying, from the strongest topic that is not its form.
///
/// Adults only. A juvenile is still becoming one thing; giving it a souvenir
/// from a second would read as clutter rather than character.
fn trinket(state: &PetState) -> &'static str {
    if state.stage != Stage::Adult {
        return "";
    }

    state
        .blend
        .secondary(state.topic)
        .filter(|(_, weight)| *weight >= TRINKET_THRESHOLD)
        .map_or("", |(topic, _)| topic.trinket())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::familiar::{Blend, Sparks, Topic, compute, fixture::START};

    /// A state to poke holes in. Every field is set, so a test can change one
    /// and be sure it is the only thing that moved.
    fn state() -> PetState {
        PetState {
            form: Form::Hexapod,
            stage: Stage::Adult,
            mood: Mood::Content,
            topic: Topic::Tech,
            blend: Blend::from_weights([1.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
            base_energy: 0.6,
            energy: 0.6,
            level: Level::Active,
            phase: Phase::Active,
            posts: 60,
            sparks: Sparks::default(),
            at: START,
        }
    }

    #[test]
    fn an_egg_is_a_face_and_nothing_else() {
        let egg = compute(&[], None, START);
        let drawn = kaomoji(&egg);
        assert_eq!(drawn.lines, vec!["(・ω・)"], "{drawn:?}");
    }

    #[test]
    fn a_hatchling_has_ears_but_no_body() {
        let mut hatchling = state();
        hatchling.stage = Stage::Hatchling;
        hatchling.posts = 4;

        let drawn = kaomoji(&hatchling);
        assert_eq!(drawn.lines.len(), 2, "{drawn:?}");
        assert_eq!(drawn.lines[0], "┬ ┬");
        assert_eq!(drawn.lines[1], "(◕ω◕)");
    }

    const LEVELS: [Level; 5] = [
        Level::Hyper,
        Level::Active,
        Level::Normal,
        Level::Lethargic,
        Level::Bored,
    ];

    #[test]
    fn only_chaos_ever_mismatches_its_eyes() {
        for mood in Mood::ALL {
            if mood == Mood::Chaos {
                continue;
            }
            for level in LEVELS {
                let (left, right) = eyes(mood, level);
                assert_eq!(left, right, "{mood:?} at {level:?} is not symmetric");
            }
        }

        // And it does mismatch — at every level but one. The design's table has
        // chaos symmetric at normal energy, which is the moment it looks merely
        // cross-eyed rather than deranged.
        let mismatched = LEVELS
            .into_iter()
            .filter(|level| {
                let (left, right) = eyes(Mood::Chaos, *level);
                left != right
            })
            .count();
        assert_eq!(mismatched, 4);
    }

    #[test]
    fn the_full_adult_pose_stacks_every_trigger() {
        let mut loud = state();
        loud.mood = Mood::Excited;
        loud.level = Level::Hyper;
        // Tech leads, nature is a strong second — so the pet carries a seedling.
        loud.blend = Blend::from_weights([0.6, 0.4, 0.0, 0.0, 0.0, 0.0]);

        let drawn = kaomoji(&loud);
        assert_eq!(drawn.lines[0], "★ ┻ ┻ ★", "celebration wraps the crown");
        assert_eq!(drawn.lines[1], "(★□★)");
        assert_eq!(drawn.lines[2], "⚡ ╱│╲ (🌱)", "motion, body, trinket");
    }

    #[test]
    fn a_trinket_needs_a_quarter_of_the_blend_and_an_adult_to_carry_it() {
        let mut adult = state();
        adult.blend = Blend::from_weights([0.8, 0.2, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(trinket(&adult), "", "a fifth is background noise");

        adult.blend = Blend::from_weights([0.7, 0.3, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(trinket(&adult), "(🌱)");

        let mut juvenile = adult;
        juvenile.stage = Stage::Juvenile;
        assert_eq!(trinket(&juvenile), "", "juveniles carry nothing");
    }

    #[test]
    fn the_pet_sleeps_only_when_it_is_both_late_and_empty() {
        let mut tired = state();
        tired.phase = Phase::Deep;
        tired.level = Level::Bored;
        assert!(kaomoji(&tired).flat().contains("zZ"));

        tired.level = Level::Lethargic;
        assert!(kaomoji(&tired).flat().contains("zZ"));

        // Deep hours but wide awake — someone is up late writing.
        tired.level = Level::Hyper;
        assert!(!kaomoji(&tired).flat().contains("zZ"));

        // Empty, but during the hours it usually writes in.
        tired.phase = Phase::Peak;
        tired.level = Level::Bored;
        assert!(!kaomoji(&tired).flat().contains("zZ"));
    }

    #[test]
    fn each_kind_of_spark_gets_its_own_mark() {
        let mut counting = state();
        counting.sparks = Sparks { milestone: Some(50), rekindled: false };
        assert_eq!(kaomoji(&counting).lines[0], "* ┬ ┬ *");

        // A return is not a round number, and reads as neither.
        counting.sparks = Sparks { milestone: None, rekindled: true };
        assert_eq!(kaomoji(&counting).lines[0], "✧ ┬ ┬ ✧");

        counting.sparks = Sparks::default();
        assert_eq!(kaomoji(&counting).lines[0], "┬ ┬");
    }

    #[test]
    fn the_corners_pick_one_when_both_happened() {
        // Both are true and there is one pair of corners. The milestone takes
        // them — but the return is still *known*, which is the whole reason the
        // choice is made here and not where the events are found.
        let mut both = state();
        both.sparks = Sparks { milestone: Some(50), rekindled: true };

        assert_eq!(kaomoji(&both).lines[0], "* ┬ ┬ *");
        assert!(both.sparks.rekindled, "the return must survive not being drawn");
    }

    #[test]
    fn the_pet_at_its_best_outranks_any_spark() {
        // The big star is a reading of the present and the marks are events; when
        // both are true the present wins, because a pet that is currently
        // delighted is the more interesting fact.
        let mut loud = state();
        loud.mood = Mood::Excited;
        loud.level = Level::Hyper;
        loud.sparks = Sparks { milestone: None, rekindled: true };

        assert_eq!(kaomoji(&loud).lines[0], "★ ┻ ┻ ★");
    }

    #[test]
    fn every_combination_draws_something_with_a_face() {
        // The point of compositional rendering is that there are no holes. Walk
        // the whole product and check each one produces a face line.
        let mut checked = 0;

        for form in [Form::Hexapod, Form::Tendril, Form::Biped, Form::Orb, Form::Drift] {
            for stage in [Stage::Egg, Stage::Hatchling, Stage::Juvenile, Stage::Adult] {
                for mood in Mood::ALL {
                    for level in LEVELS {
                        for phase in [Phase::Peak, Phase::Active, Phase::Dusk, Phase::Quiet, Phase::Deep] {
                            let mut probe = state();
                            (probe.form, probe.stage, probe.mood, probe.level, probe.phase) =
                                (form, stage, mood, level, phase);

                            let drawn = kaomoji(&probe);
                            assert!(!drawn.lines.is_empty(), "{probe:?} drew nothing");
                            assert!(
                                drawn.lines.iter().any(|line| line.contains('(')),
                                "{probe:?} drew {drawn:?} with no face",
                            );
                            assert!(drawn.lines.len() <= 3, "{probe:?} drew {drawn:?}");
                            checked += 1;
                        }
                    }
                }
            }
        }

        assert_eq!(checked, 5 * 4 * 7 * 5 * 5);
    }

    #[test]
    fn the_description_reads_as_a_sentence() {
        let described = describe(&state());
        assert!(described.starts_with("The familiar:"), "{described}");
        assert!(described.contains("hexapod"), "{described}");
        assert!(described.contains("content"), "{described}");
        assert!(!described.contains('('), "no glyphs in the label: {described}");

        let egg = compute(&[], None, START);
        assert!(describe(&egg).contains("egg"));
    }
}
