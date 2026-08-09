//! Drawing the familiar.
//!
//! Two views of one snapshot: a block above the feed, and the character sheet at
//! `/familiar`. Both read the same [`Reading`], so the page can never contradict
//! the widget that linked to it.
//!
//! The kaomoji goes in a `<pre>` because it is genuinely fixed-width art. The
//! stats around it do **not** get the design's box-drawing frame: that frame is
//! 47 characters wide and cannot reflow, which on a phone means either a
//! horizontal scrollbar or a broken picture. The bar glyphs are kept — they are
//! the texture that makes this read as a terminal pet — and the frame is the
//! site's own card, which already knows how to be narrow.

use maud::{Markup, html};

use crate::familiar::{Reading, Stage, render, stats::Vitals};

/// Characters in a stat bar. The design's width, and about as wide as fits
/// beside a label on a 375px screen.
const BAR: usize = 14;

/// The block above the feed: the pet, three lines of context, and a link.
pub fn widget(reading: &Reading) -> Markup {
    let state = &reading.state;

    html! {
        // An explicit name rather than one assembled from the contents. The
        // picture is hidden from assistive technology and what is left is three
        // fragments — "tired · active", "juvenile hexapod · peak hours" — which
        // read as a name about as well as they sound.
        a href="/familiar" aria-label=(summary(reading))
          class="flex items-center gap-5 rounded-box border border-base-300 bg-base-200 \
                 p-4 no-underline hover:border-primary/50" {
            (art(reading, false))

            div class="min-w-0 text-sm" {
                @if state.stage == Stage::Egg {
                    p { "the familiar" }
                    p class="text-secondary" { "waiting for a first post" }
                } @else {
                    p { (state.mood.as_str()) " · " (state.level.label()) }
                    p class="text-secondary" {
                        (state.stage.label()) " " (state.form.label())
                        " · " (state.phase.label()) " hours"
                    }
                    p class="text-secondary" {
                        (reading.vitals.posts) " posts · " (cadence(reading.vitals.cadence_hours))
                        " cadence"
                    }
                }
            }
        }
    }
}

/// The character sheet at `/familiar`.
pub fn sheet(reading: &Reading) -> Markup {
    let state = &reading.state;
    let vitals = &reading.vitals;

    html! {
        div class="flex flex-col gap-4" {
            section class="rounded-box border border-base-300 bg-base-200 p-6" {
                (art(reading, true))

                p class="mt-4 text-center text-sm" {
                    @if state.stage == Stage::Egg {
                        "waiting for a first post"
                    } @else {
                        (state.mood.as_str()) " · " (state.level.label())
                        span class="text-secondary" { " · " (state.phase.label()) " hours" }
                    }
                }
            }

            (panel("vitals", html! {
                (fact("posts", &vitals.posts.to_string(), &format!("{} days old", vitals.age_days)))
                (fact("words", &thousands(vitals.words), &format!("{} per post", vitals.words_per_post)))
                (fact("streak", &streak(vitals), &format!("{} cadence", cadence(vitals.cadence_hours))))
            }))

            @if !reading.diet.is_empty() {
                (panel("diet", html! {
                    @for (topic, share) in reading.diet.ranked().into_iter().take(5) {
                        (meter(topic.label(), percent(share)))
                    }
                }))
            }

            @if reading.moods.len() > 1 {
                (panel("moods", html! {
                    @for (mood, share) in reading.moods.iter().take(4) {
                        (meter(mood.as_str(), percent(*share)))
                    }
                }))
            }

            (panel("character sheet", html! {
                @for (label, score) in reading.sheet.rows() {
                    (meter(label, score))
                }
            }))

            p class="text-sm text-secondary" {
                (state.stage.label()) " · " (state.posts) " posts"
                @if let (Some((_, toward)), Some(next)) =
                    (state.stage.progress(state.posts), state.stage.next())
                {
                    " · " (toward) "% toward " (next.label())
                }
                br;
                "form: " (state.form.label()) " (" (state.topic.label()) ")"
            }
        }
    }
}

/// The kaomoji itself.
///
/// `pre` because the lines are art, `text-center` because they are not the same
/// length and padding them by character count would be a lie — half these glyphs
/// are East Asian Ambiguous and render at whatever width the reader's font says.
///
/// On the page the picture is the subject, so it is an image with a description.
/// In the widget the three lines of text beside it already say everything it
/// says, so it is hidden rather than announced twice.
fn art(reading: &Reading, labelled: bool) -> Markup {
    let drawn = render::kaomoji(&reading.state);

    html! {
        @if labelled {
            pre role="img" aria-label=(render::describe(&reading.state))
                class="text-center font-mono text-lg leading-snug text-primary" {
                @for line in &drawn.lines { (line) "\n" }
            }
        } @else {
            pre aria-hidden="true"
                class="shrink-0 text-center font-mono leading-snug text-primary" {
                @for line in &drawn.lines { (line) "\n" }
            }
        }
    }
}

/// What the widget link announces: the picture described, plus the one number
/// that says how much pet there is.
fn summary(reading: &Reading) -> String {
    let described = render::describe(&reading.state);

    if reading.state.stage == Stage::Egg {
        return described;
    }
    format!("{described} {} posts.", reading.vitals.posts)
}

/// A titled card. The design's `├─ vitals ─────┤` rule, as a heading.
fn panel(title: &str, body: Markup) -> Markup {
    html! {
        section class="rounded-box border border-base-300 bg-base-200 p-4" {
            h2 class="mb-3 text-sm text-secondary" { (title) }
            div class="flex flex-col gap-1.5 text-sm" { (body) }
        }
    }
}

/// One `label  value  ◈ note` row from the vitals block.
fn fact(label: &str, value: &str, note: &str) -> Markup {
    html! {
        div class="flex items-baseline gap-3" {
            span class="w-16 shrink-0 text-secondary" { (label) }
            span class="w-20 shrink-0 tabular-nums" { (value) }
            span class="text-secondary" { "◈ " (note) }
        }
    }
}

/// One labelled bar.
///
/// The bar is two spans rather than one string so the filled part can be
/// coloured, and `aria-hidden` because a screen reader announcing fourteen block
/// characters is noise — the number beside it is the same information.
fn meter(label: &str, score: u8) -> Markup {
    let filled = score as usize * BAR / 100;

    html! {
        div class="flex items-baseline gap-3" {
            span class="w-20 shrink-0 truncate text-secondary" { (label) }
            span aria-hidden="true" class="font-mono" {
                span class="text-primary" { (repeat('█', filled)) }
                span class="text-base-300" { (repeat('░', BAR - filled)) }
            }
            span class="w-10 shrink-0 text-right tabular-nums" { (score) "%" }
        }
    }
}

fn repeat(glyph: char, count: usize) -> String {
    std::iter::repeat_n(glyph, count).collect()
}

/// A `0.0..=1.0` share as a percentage.
fn percent(share: f64) -> u8 {
    crate::familiar::stats::percent(share)
}

/// "5h", in minutes when someone is writing faster than that and in days when
/// they are writing slower.
///
/// The days case is not hypothetical: cadence is the gap between *sittings*, so
/// a writer who sits down every Sunday genuinely has one of 168 hours, and
/// printing that as `168h` would be a number nobody converts in their head.
fn cadence(hours: f64) -> String {
    if hours >= 48.0 {
        format!("{:.0}d", hours / 24.0)
    } else if hours >= 1.0 {
        format!("{hours:.0}h")
    } else {
        format!("{:.0}m", hours * 60.0)
    }
}

/// "3d 🔥" while it is still running, "3d" once it is over.
fn streak(vitals: &Vitals) -> String {
    match (vitals.streak_days, vitals.streak_alive) {
        (0, _) => "—".to_owned(),
        (days, true) => format!("{days}d 🔥"),
        (days, false) => format!("{days}d"),
    }
}

/// "4,320". `time` and `maud` have no number formatting between them, and this
/// is four lines.
fn thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);

    for (index, digit) in digits.char_indices() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_separates_every_three_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(4_320), "4,320");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn cadence_switches_units_at_both_ends() {
        assert_eq!(cadence(5.0), "5h");
        assert_eq!(cadence(1.0), "1h");
        assert_eq!(cadence(0.5), "30m");

        // A writer who sits down once a week, which is an ordinary rhythm and
        // not an edge case.
        assert_eq!(cadence(47.0), "47h");
        assert_eq!(cadence(48.0), "2d");
        assert_eq!(cadence(168.0), "7d");
    }

    #[test]
    fn a_bar_is_always_the_same_width() {
        for score in 0..=100u8 {
            let filled = score as usize * BAR / 100;
            assert!(filled <= BAR, "{score}% overflowed the bar");
            assert_eq!(repeat('█', filled).chars().count() + repeat('░', BAR - filled).chars().count(), BAR);
        }
    }
}
