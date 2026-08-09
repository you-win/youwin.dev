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
one number among a hundred ordinary evenings, and a mean is hostage to it. The baseline is two
quantiles of the most recent sixteen gaps:

- **typical gap** — the median. The rhythm the writer is working to.
- **long gap** — the 75th percentile. The gap they exceed about a quarter of the time.

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

## What comes next

None of this is built. Ordered by how much dynamism they buy per unit of code, and every one of
them holds the invariants above.

**Speech, ranked by surprise.** One line under the pet, and the rule is that *it says the most
surprising true thing about you*. Score each candidate observation by its information content
under the baseline — `−log P(observation)` — and say the highest. "47 posts" is always true and
scores nothing. "Quietest week since April" is rare, and therefore worth saying. This is the
largest perceived-dynamism win per line of code in the list, and the baseline is what makes the
selection principled rather than a shuffled table of stock phrases.

**The chronicle.** The pet has no memory of itself: everything is computed from *now* looking
backwards. Replaying `compute` across the archive recovers its whole biography — when it
hatched, every time it changed form, its lowest ebb, its longest streak, the first post about
each topic. Pure replay, no schema, and it gives `/familiar` a timeline that grows. The cost is
real and needs handling: `topics::classify` over a rolling 50-post window per step is O(n·50·k),
so this wants either an incremental classify or a daily-granularity replay cached on a longer
TTL than the five-minute snapshot. It only changes when you post.

**Traits that change behaviour.** Slow-moving characteristics read off the whole archive, which
modify the state machine rather than decorate it. Nocturnal — a third of sittings in the learned
deep hours — and the pet stops sleeping at 3am and inverts its phase offsets. Irregular, from a
wide gap between the quartiles, and it forgives longer. Laconic or prolix, from word-count
percentiles. This is what makes the creature *yours*: two people's pets should behave
differently because of how they write, not merely look different.

**Anticipation and appetite.** The learned circadian profile already knows when you write.
During a predicted peak with nothing posted yet, the pet can be *expectant* — a nudge that only
fires at the moment you would plausibly write anyway. Appetite is the same idea on the topic
axis: a diet that has been all tech for a fortnight is a deficit the pet can visibly want
something else for. Both are pull, not obligation, which is the line.

**Rekindling, and milestones that last.** Coming back after three weeks currently shows a
floored pet that recovers slowly — the absence is punished and the return is not rewarded. The
first post after a gap past the writer's own 90th percentile should be a distinct, visible
event, derivable entirely from the length of the last gap and the hours since that post. The
same trick fixes the milestone stars, which today appear only while the post count is *exactly*
10, 50 or 100 and can be missed entirely: find the fiftieth post's timestamp and celebrate for a
day after it.

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
