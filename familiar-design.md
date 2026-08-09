# The Familiar — design

A kaomoji that reads the archive's temperature. It sits above the feed on page one and has its
own page at `/familiar`. This is the specification; [`DESIGN.md`](DESIGN.md#the-familiar-m6)
records where the implementation departs from it and why.

Two documents existed before this one — a prototype spec that was never committed, and the M6
section of `DESIGN.md` written against it. Both referred to a `familiar-design.md` that was not
in the repository. This file replaces the missing one and is written against what is actually
in `crates/server/src/familiar/`.

## What it is for

**The pet is a character, not a dashboard.** Everything it shows is a reading of one writer's
habits. Dressed as engagement metrics the same numbers would mean something they do not, and
the site has no audience metrics at all by design — no likes, no view counts, nothing that
turns writing into performance.

**Appetite and surprise, never obligation.** The pull to come back should be *it has something
new to show me*, not *it will be sad if I don't*. This is the line every future feature gets
held to. Loss aversion is the cheap lever and it works on strangers; on a single-user blog,
guilt mechanics teach you to stop visiting your own site. A pet that is interesting to check on
survives that, and a pet that nags does not.

**Nothing here is a goal.** No target streak, no daily quota, no bar that is supposed to reach
the end. The bars measure; they do not ask.

## Invariants

These are load-bearing. Anything proposed below that cannot hold them does not get built.

**No schema, no writes.** Every field is a pure function of `(posts, previous state, now)`.
There is no pet table, no migration, and no state that can drift out of agreement with the
posts — delete a post and the pet forgets it on the next recompute. This is what lets the whole
thing live on the public listener, which is handed the read pool and nothing else.

**The archive is the save file.** Anything that looks like it needs stored memory can almost
always be recovered by finding the post that caused it. A milestone is not a flag, it is the
timestamp of the fiftieth post. A form change is not an event log, it is the moment a replay of
the state machine crosses over. Reach for this before reaching for a column.

**One snapshot per process, five minutes.** Matched to the `s-maxage=300` the public pages are
served with. The useful consequence is that the pet reflects the gap between the *last visit*
and the *last post*: through a quiet week nothing runs at all, and the next visitor triggers one
catch-up and sees a pet that has plainly been alone.

**Recomputation must be idempotent.** Energy is recomputed on every cache miss, so anything
folded into the carried value compounds once per page load. `base_energy` carries decay and
bursts; the circadian offset is added to the copy being rendered and thrown away. A pet that
could be walked to hyper by holding down F5 is the bug this prevents, and it is easy to
reintroduce.

**Public posts only, replies included.** A reply is something that was sat down and written.
Unlisted posts and drafts are excluded — the post count renders on a public page, and counting
them would let a visitor infer that unlisted posts exist and roughly when.

**UTC everywhere**, like every other timestamp on the site. Phase learning is relative to the
archive's own histogram, so the absolute hours stop mattering once there is a fortnight of data.

## The state machine

Five dimensions, in order of how fast they move. Implemented in
[`familiar/mod.rs`](crates/server/src/familiar/mod.rs) and the modules beside it.

| Dimension | Reads | Shows as |
|---|---|---|
| `Form` | dominant topic over a 50-post window | the silhouette |
| `Stage` | total posts | how much silhouette there is |
| `Mood` | most recent post that expressed one, over 10 | eyes and mouth |
| `Level` | energy: decay since the last post, plus bursts | frame size, motion, sleep |
| `Phase` | this hour against the learned posting rhythm | an energy offset |

Beneath the five is a slower channel, [traits](#traits--the-slow-channel), read off the whole
archive rather than off the present. It draws nothing of its own; it changes what two of the
five mean.

**Rendering is compositional.** Eyes, mouth, crown and base are independent lookups assembled at
draw time, so a new mood costs five table entries rather than the seven hundred hand-drawn
kaomoji every combination would otherwise need.

**Mood is a field on the post**, picked in the composer, nullable — NULL means "did not say" and
infers from the text, a stored value means "did say" and nothing overrides it, including a
stored `neutral`.

## Rhythm — the baseline

*Implemented in [`familiar/baseline.rs`](crates/server/src/familiar/baseline.rs).*

Every question the pet asks that involves waiting — how fast energy decays, how much one new
post is worth — only means something relative to how often this particular person writes. Eight
hours is an ordinary afternoon for one archive and a disappearance for another. The first
version measured that badly enough to make the pet actively wrong for a large class of writer.

### Sittings, not posts

The first version took the **mean gap between posts over the last seven days**. People do not
write on a schedule; they sit down. Somebody who wrote five notes every Sunday measured a
cadence of five minutes, which set a decay half-life pinned to its two-hour floor, which left
their pet flat on the floor from Monday morning until the following weekend — while they were
doing exactly what they had always done. The pet could not tell a weekly rhythm from
abandonment, and bursty writing is what a microblog actually looks like.

A post starts a new **sitting** when more than 45 minutes separates it from the one before, and
the rhythm is the gap between sitting *starts*. Forty-five minutes is deliberately wider than
the 30-minute window over which posts stack into an energy burst: two posts close enough to
count as one burst must never be counted as two separate visits to the composer.

### Order statistics, not moments

Gaps between sittings span orders of magnitude and lean hard to the right — a fortnight away is
one number among a hundred ordinary evenings, and a mean is hostage to it. The decay curve is
set by two quantiles of the most recent sixteen gaps:

- **typical gap** — the median. The rhythm the writer is working to.
- **long gap** — the 75th percentile. The gap they exceed about a quarter of the time.

The module holds **three** such distributions, all built the same way and all sixteen long: gaps
between sittings, posts per sitting, and words per post. Only the first was built at first,
because only the first had a caller; [speech](#speech) is the caller the other two were waiting
for, and it reads all three as tails rather than as quantiles.

A **count** of gaps rather than a time window is what makes it self-scaling. Sixteen gaps is
most of a day for someone who writes hourly, and four months of habit for someone who writes
weekly. No window measured in days does both: seven days holds a single gap for the weekly
writer, which is not a distribution at all.

This started as a median and spread in log space, on the theory that log-gaps are roughly
normal. Quantiles are invariant under monotone transforms, so the logarithm changed neither
number; and the median absolute deviation collapses to exactly zero once more than half the
gaps are identical — the common case of somebody who writes most days and vanishes for a week
now and then. That writer would have been handed no tolerance at all, which is backwards. The
empirical quantiles have no such breakdown and assume nothing about the shape.

### The half-life falls out of it

```
τ = clamp(2 × long_gap, 2 hours, 14 days)
```

The long gap already contains both halves of the question. A metronome's quartiles sit on top
of each other, so its long gap *is* its typical one and anything further is immediately out of
character. Somebody whose weeks vary has them far apart, so a stretch that would alarm the
metronome is, for them, a Tuesday. One number covers three orders of magnitude of posting habit,
which is why it replaced a hand-picked multiple of the mean.

The bounds are not decoration. Without a floor a pet could be built that visibly sagged inside a
single sitting. Without a ceiling the formula happily concludes that a blog with two posts a
year apart is on schedule six months later — true, and useless, because nothing about that pet
would ever move. **A fortnight is the pet's memory.** Past that it stops pretending, and the
writers this bites are the monthly ones, who were never the audience for a creature that
changes daily.

Roughly what changed, energy at half of the writer's own typical gap:

| Writer | Typical sitting gap | τ before | τ after | Mid-rhythm reads as |
|---|---|---|---|---|
| Hourly | 1h | 3h | 2h | active → active |
| Daily, a few notes a sitting | 24h | ~22h | ~60h | normal → active |
| **Weekly, five notes a sitting** | 168h | **2h** | 336h | **bored → active** |
| Monthly | 720h | 18h | 336h (capped) | bored → lethargic |

The weekly row is the bug. The monthly row is the deliberate limit.

## The composer

*Implemented in [`write/routes/familiar.rs`](crates/server/src/write/routes/familiar.rs) and
[`Familiar.tsx`](web/src/components/Familiar.tsx).*

The pet rendered on the public site — where the archive is read — and not in the authoring app,
where the posting happens. As a reason to keep posting that is backwards: the loop closed on a
surface the author visits occasionally. It now sits above the composer and answers the question
the public site cannot ask, which is what the post you have not made yet would do to it.

`GET /api/familiar` is the pet as it is. `POST /api/familiar/draft` takes what is in the box and
returns `compute(posts + [draft], previous, now)` — the draft through the real markdown
pipeline, so keyword matching sees the same plaintext a stored post would produce. The client
holds both readings and reports the difference, because the difference is the only interesting
thing on screen. Four hundred milliseconds of quiet before asking.

**Drafts and unlisted notes preview as no change**, because that is what they are: only public
posts feed the pet. It is a true and slightly surprising rule, and the composer is the one place
it can be discovered beforehand rather than by watching nothing happen afterwards.

**The authoring host holds its own snapshot.** Two instances of the same derived state, kept
apart because they are invalidated by opposite things. The public one lives on the five-minute
TTL matched to the edge cache in front of it. This one is dropped on every write — a five-minute
wait to see what you just posted is the TTL doing exactly the wrong thing — and it answers draft
previews, which must never be able to write a hypothetical into what the public site is serving.

**Invalidating is not the same as discarding**, and this was a real bug before it was a rule.
Dropping the held `Reading` outright hands the next read a `previous` of `None`, which is the
cold-start path — and cold start estimates from the last post's age and applies *no burst*. So a
draft previewed as the jolt it genuinely is, the post landed, and the pet settled at the flat
cold-start value instead. The composer was wrong about every post, in the one direction that
makes the feature pointless. Staleness and the carried energy are therefore separate: a write
marks the snapshot for recomputation and leaves the state it carries alone.

## Speech

*Implemented in [`familiar/speech.rs`](crates/server/src/familiar/speech.rs).*

One line, and the rule that picks it: **the pet says the most surprising true thing about the
archive.** A shuffled table of stock phrases is dynamic for a week and wallpaper after that,
because the interesting sentence is never the one that is always available. "47 posts" is true
every day and worth saying on none of them.

**Every candidate reports a tail, not a score.** Each one answers the same question — what is
the probability the archive would look *at least this extreme*, under this writer's own history
— and the least likely observation wins. That shared meaning is the only thing making two
candidates comparable; a hand-assigned weight anywhere would quietly become the real ranking.

**The floor is load-bearing.** Below two bits — one in four — the pet says nothing at all. A pet
that always has a line spends most of its life saying something dull, and one dull line teaches
you to stop reading the interesting ones. Silence on an ordinary day is the feature.

**The ceiling is an admission.** The samples behind these tails are sixteen observations long,
so the rarest thing they can honestly report is a shade under one in seventeen. Surprise is
capped there rather than running away. `Sample::at_least` is Laplace-smoothed for the same
reason: an unsmoothed count of zero says "never, and never could", and `−log2(0)` is infinite —
one unprecedented post would out-shout everything the pet could ever say again. A useful side
effect is that no dimension can monopolise the line; once two candidates are both pinned at the
ceiling they take turns, by the day, chosen by a hash so the rotation does not march predictably
as the days do.

**Creature voice, everywhere.** Nothing addresses a reader. The public widget is read by
strangers, so a line in the second person would be telling *them* about someone else's habits,
and one register means one table to keep true instead of two. The pet is "it", the archive is
what it eats, the writer is never spoken to. A test pins this, because it is exactly the kind of
rule that erodes one convenient phrasing at a time.

Five candidates, one per dimension the pet already reads: **silence** against the gap
distribution, **abundance** against sitting sizes, **length** against words per post, **odd
hour** against the circadian profile, and **rare mood** against the archive's mood split.
Everything except silence describes the last thing written, so it is gated on that post still
being recent by the writer's own definition — one typical gap.

**Odd hour needs a gate the others do not**, and the difference is worth stating because it will
come up again. Every other tail is smoothed against a sample that is simply *empty* early on,
which returns a probability of one and falls under the floor by itself. Odd hour is judged
against the circadian profile, which is a **blend of evidence and an assumed human schedule** —
a prior, not an observation. Ungated it announced that a single midnight post came at an odd
hour, having never seen the writer at any hour at all. It now needs a fortnight (when the guess
has finished being displaced) and a full sample of posts (below which a histogram of three
spikes makes the other twenty-one hours unusual by default).

Speech **displaces** the stats line on the feed widget and in the composer rather than adding a
line — it only appears when something is genuinely unusual, the numbers it covers are all on
`/familiar` anyway, and a widget that grew a line on interesting days would reflow the feed
under it. On `/familiar` it sits under the picture, where the pet is the subject.

It lives on the `Reading`, not on each surface, so the widget, the sheet and the composer cannot
say three different things about one moment — and so the day-rotation lands on one answer per
snapshot.

## Sparks — the pet's first transient

*Implemented in [`familiar/spark.rs`](crates/server/src/familiar/spark.rs).*

Everything else in `PetState` is a steady-state reading of the present: what the archive looks
like now, drawn now. A **spark** is not — it is something that happened at a moment and fades
afterwards. The shape is worth getting right once, because anticipation and appetite will both
want it.

It holds the invariants. The event's timestamp is in the archive, how long ago it was comes from
`now`, and the window it survives comes from the writer's own rhythm — still a pure function of
`(posts, now)`, still nothing stored. Delete the post that caused it and it never happened.

**Milestones already existed and were unwatchable.** The pose was drawn while the post count was
*exactly* ten, fifty or a hundred — a window of zero. Post your fiftieth and fifty-first an hour
apart and the only celebration the pet offers existed for that hour, seen or not. A reward you
can miss without knowing it was there is worse than none. A milestone is now the *timestamp of
the Nth post*, and it lasts a window.

**Rekindling is what the whole premise was missing.** Coming back after a real absence showed a
floored pet climbing slowly out of it: the absence was punished and the return was not rewarded,
which is backwards for a creature meant to be a reason to write again. The first sitting after a
genuine silence is now an event in its own right — `✧` in the corners where a milestone puts
`*`, and a line in speech.

**An absence is both unusual and long, and it needs both tests.** The 90th-percentile gap alone
fails on exactly the degeneracy that ruled out a median absolute deviation in the baseline: for
a writer whose gaps are all alike, the ninetieth percentile *is* the ordinary gap, so anything
beyond it clears the bar and a daily writer skipping one day gets welcomed back from nowhere. A
silence must also run to three times the typical gap. It also needs half a sample of gaps behind
it — but not the full sample `speech`'s odd-hour claim demands, because that one is judged
against a prior and this against observations, and a fortnight away is precisely when somebody
needs welcoming back rather than being told there is insufficient data.

**The window is two of the writer's own gaps**, floored at a day and capped at a fortnight. Two
rather than one because at one a daily writer's milestone expires at the exact moment they next
sit down — a spark has to last *past* the next sitting, not up to it.

The speech line for a return is the mirror of the silence line and inherits its arithmetic
exactly: the tail of the gap just closed is the number `silence` would have reported an hour
before the returning post landed. The two can never both fire, so the pet reports an absence
right up until it ends and then reports the ending. It is gated on the spark rather than
re-deriving the condition, so what the pet says about coming back and what it draws cannot
disagree about whether it happened.

**Detection reports every event; only the renderer chooses between them.** Both can be true at
once — a return that happens to be the fiftieth post — and the first cut resolved that by
returning a single winner, milestone over rekindling. That put a fact about *corners* (there is
one pair, and only one thing fits) inside the code that decides what is *true*, and because
speech reads the same value, a fiftieth post after three weeks away drew a `*` and had nothing
whatever to say about the three weeks. Every other candidate is quiet in that moment, so
silencing the return silenced the pet entirely, on what was easily the most surprising day in
the archive. Scarcity belongs where the scarce resource is.

## Traits — the slow channel

*Implemented in [`familiar/traits.rs`](crates/server/src/familiar/traits.rs).*

Every other channel reads the present: ten posts for mood, fifty for form, sixteen gaps for
rhythm. A **trait** is a characteristic of the whole archive, and it changes how the pet
*reacts* rather than what it shows. This is the part that is meant to make the creature
yours — two people's pets behaving differently because of how they write, not merely looking
different.

It holds the invariants unchanged: still a pure function of `(posts, now)`, still nothing
stored, still no threshold at which anything lurches.

### Two of the three were already built

This section originally asked for nocturnal, irregular, and laconic/prolix. Two of those had
already been implemented by machinery that landed *after* they were written down. Recording
that is worth more than quietly dropping them, because it is an easy mistake to repeat: a
characteristic that sounds like a trait is very often a constant somewhere that is already
relative to the writer.

**Nocturnal** — "the pet stops sleeping at 3am and inverts its phase offsets" — is what the
learned circadian profile does by itself. Phases are cut from the archive's own histogram, so
somebody who only ever posts at three in the morning has 03:00 as their `Peak` within a
fortnight, and the pet only ever sleeps in `Deep`. `energy`'s
`the_learned_rhythm_displaces_the_assumed_one` is that assertion, already passing.

Nor can it be *detected* honestly. The site is UTC and deliberately does not guess at zones,
so nocturnal measured against the clock calls a writer eleven zones east a night owl for
posting after breakfast. Measured against the learned profile instead, it is self-defeating:
the more of an archive that lands at night, the less the night reads as unusual. There is no
third option, and the invariant that says so is the one about UTC at the top of this file.

**Irregular** — "it forgives longer" — is the decay half-life, which is already twice the
writer's own 75th-percentile gap. `baseline`'s `regularity_decides_how_long_a_long_gap_is` is
literally a test that an irregular writer is forgiven longer than a metronome with the same
median. A trait multiplying that again would count one piece of evidence twice, which is the
failure the tails-not-scores rule in [speech](#speech) exists to prevent.

What was left is the two places the pet was still the same creature for everybody.

### Length — energy counted posts and never words

The burst is worth a fixed amount per post, so a weekly two-thousand-word essay moves the pet
exactly as far as "brb" does. That is the same shape as the bug the [baseline](#rhythm--the-baseline)
rewrite was for — an archive measured in a unit that is wrong for a whole class of writer —
and it leaves the essayist with a pet on the floor no matter how much they write.

**Length** is the archive's median words per post, against thirty. An absolute count, and the
contrast with nocturnal is the point: an hour needs a timezone before it means anything and
the site does not have one, while thirty words is a couple of sentences for everybody. Effort
is sublinear in length — four times the words is not four times the sitting — so the ratio is
taken through a square root, and it is bounded to between half and twice a note.

It is a property of the *archive*, never of the post in hand. Per-post it would make one
pasted quotation a spike, and the whole point of the slow channel is that slow things move
slowly.

**Two amplifiers, one bound.** Cadence already asks how often this writer sits down; length
asks how much lands when they do. Alone, either can double a post's worth; together they are
capped at three rather than four, because the two overlap — somebody who sits down once a
week is usually also writing more each time — and a fourfold post would take a pet from the
floor to hyper on one essay. They are not folded into cadence's own clamp, which saturates
past a twelve-hour rhythm and would have left the trait inert for exactly the writers it was
built for.

### Focus — the phase cut had no measure of its own confidence

`energy::phases` names a densest four-hour block whether or not there is a habit to find. For
a flat histogram it names one anyway, and the writer gets the full ±0.10 and ±0.15 swings, and
a pet that sleeps, off a peak that is arithmetic rather than evidence.

**Focus** is how much of the archive falls in that block, measured against how much falls in
the *assumed* schedule's own. Below it is the sixth of the archive that any four hours holds
by arithmetic, which is the zero. It scales the circadian offset, and below half it also stops
the pet drawing `zZ` — because `Deep` is only the leftovers of the cut, and for a writer who
works at every hour equally the leftovers are not a night.

The reference is what makes it safe to deploy. An archive too young to have displaced the
prior *is* the prior, so it measures as exactly as concentrated as one and its pet keeps every
bit of the offset it had the day before. The number then comes off the ceiling only as the
histogram earns it.

### Neutral is whatever the pet already did

Both traits are ratios against a reference picked so that an ordinary archive lands on exactly
1.0 and the arithmetic downstream is the arithmetic that was there before. A trait can bend the
pet for a writer the machine currently mis-serves; it cannot move one that was already right.
No archive changes behaviour on the day this ships unless it was being read wrongly the day
before.

Neither is a flag, for the same reason the circadian profile is a blend rather than a switch:
the effect is continuous, so nothing lurches as an archive drifts across a line. The names on
`/familiar` — *terse*, *prolix*, *scattered* — are a rounding of the number for the page, and
only departures are named. An archive of note-sized posts at consistent hours is not "ordinary,
punctual"; it goes unremarked on, which is the same rule speech follows and keeps the page
clear of anything that could be read as a pole to aim for.

## What comes next

None of this is built. Ordered by how much dynamism they buy per unit of code, and every one of
them holds the invariants above.

**A diet-shift candidate.** The sixth line speech should have, held back from the first cut
because it is the one needing new maths: compare the recent window's topic mix against the
long-run diet and report the divergence — "it has eaten nothing but tech lately". The tail is a
binomial one, `P(X ≥ k)` for `n` recent posts at the long-run share, which is a small exact loop
and fits the existing contract without stretching it.

**The chronicle.** The pet has no memory of itself: everything is computed from *now* looking
backwards. Replaying `compute` across the archive recovers its whole biography — when it
hatched, every time it changed form, its lowest ebb, its longest streak, the first post about
each topic. Pure replay, no schema, and it gives `/familiar` a timeline that grows. The cost is
real and needs handling: `topics::classify` over a rolling 50-post window per step is O(n·50·k),
so this wants either an incremental classify or a daily-granularity replay cached on a longer
TTL than the five-minute snapshot. It only changes when you post.

**More traits.** [Two](#traits--the-slow-channel) are built and the slot is open. The bar a
third has to clear is the one the first two set: it must name a place the pet is still the same
creature for everybody, rather than a second reading of something already relative to the
writer. Sitting shape is the most promising unclaimed one — somebody who always writes in runs
and somebody who never does are different creatures, and nothing in the pet currently knows
which it is looking at.

**Anticipation and appetite.** The learned circadian profile already knows when you write.
During a predicted peak with nothing posted yet, the pet can be *expectant* — a nudge that only
fires at the moment you would plausibly write anyway. Appetite is the same idea on the topic
axis: a diet that has been all tech for a fortnight is a deficit the pet can visibly want
something else for. Both are pull, not obligation, which is the line.

**A character sheet that breathes.** Four of the five stats barely move — WIS is `posts/100`,
CUR and MAG are whole-archive shares, STR is posts against a fixed prolific baseline. Only VIT
changes week to week, and a bar that never moves is not worth looking at twice. Each should be
measured against the writer's own trailing baseline instead, so 50% means "a normal week for
you" and the bars move because your writing did.

## Deliberately not

**Notifications.** The obvious tamagotchi move, and the one most likely to make somebody
uninstall their own blog. Ruled out by the appetite-not-obligation line above rather than by
anything technical.

**Any pet state in the database.** See the invariants. If a feature genuinely cannot be derived
from the archive, that is strong evidence it is the wrong feature.

**Anything that reads as a score to beat.** Ranks, levels-as-achievement, completion
percentages, comparisons to anyone else. There is no anyone else.
