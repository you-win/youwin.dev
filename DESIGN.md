# youwin.dev — design

A single-user microblog, split across two surfaces:

- **`youwin.dev`** — the public archive. Server-rendered HTML, **no JavaScript at all**,
  no cookies, fully edge-cacheable.
- **`write.youwin.dev`** — the authoring app. A Solid SPA behind a password, installable
  as a PWA, calling a JSON API on its own origin.

One binary serves both, from one SQLite file.

Replaces the Zola static site entirely: `content/`, `sass/`, `templates/`, `themes/`,
and `config.toml` go away at the start of M0.

## Decisions

| | |
|---|---|
| Public site | `youwin.dev` — `maud` templates, zero JS, zero cookies |
| Authoring | `write.youwin.dev` — Solid SPA + PWA. API on the **same origin**, so no CORS |
| Database | SQLite via `sqlx` — async API, embedded migrations. Runtime-checked queries; no `cargo sqlx`, no build-time database |
| Auth | Password + `argon2id` → session cookie, scoped to the authoring subdomain only |
| Post content | Text with a markdown subset. Self-replies chain into threads |
| Search & tags | FTS5 over `body_text` (porter-stemmed) and `#hashtags`, on both surfaces |
| No images in v1 | No upload path, no attachment table. Sanitizer strips `<img>` |

The split is what makes the rest simple. A single origin serving both audiences would
need per-post OG tags spliced into an SPA shell, a `<noscript>` fallback for crawlers, a
shell that can never be cached because it carries per-post content, and a service worker
carefully taught which routes are authenticated. Separating them deletes all four
problems rather than solving them.

## Shape

```
        youwin.dev                                write.youwin.dev
   no JS · no cookies · cacheable            SPA + PWA · session cookie
              │                                        │
    ┌─────────▼─────────┐                    ┌─────────▼─────────┐
    │  Caddy :443       │                    │  Caddy :443       │
    └─────────┬─────────┘                    └────┬─────────┬────┘
              │                        /assets/*  │         │  everything else
              │                       + SPA shell │         │
              │                          ┌────────▼───┐  ┌──▼──────────────┐
              │                          │ file_server│  │                 │
              │                          │ /var/www   │  │                 │
              │                          └────────────┘  │                 │
    ┌─────────▼──────────┐                               ┌─────────────────┐
    │ :8080 public Router│                               │ :8081 write     │
    │  maud templates    │                               │  JSON API only  │
    │  read pool ONLY    │                               │  read + write   │
    └─────────┬──────────┘                               └────────┬────────┘
              └──────────────────┬─────────────────────────────────┘
                        one binary · shared pools
                        ┌────────▼───────┐
                        │ SQLite (WAL)   │
                        └────────────────┘
```

**Two listeners, not `Host`-header branching.** The public `Router` has no authoring
routes compiled into it, so a routing or middleware bug cannot expose the composer. The
boundary is enforced at the socket, not by middleware ordering that someone could later
get wrong. `main.rs` builds two routers and `tokio::join!`s two `axum::serve` calls.

The public listener is handed the **read pool only** — it has no handle to the write pool
at all, so "the public site wrote to the database" is not a reachable state.

**No CORS anywhere.** The SPA and its API share `write.youwin.dev`; the public site calls
nothing. In dev, Vite proxies `/api` to `127.0.0.1:8081` so cookies behave as in prod.

## Repo layout

```
youwin.dev/
├─ Cargo.toml
├─ crates/server/
│  ├─ migrations/             # 0001_init, 0002_search_and_tags, 0003_post_mood — no CLI
│  ├─ tests/                  # one per statement in db/, plus the router integration tests
│  └─ src/
│     ├─ lib.rs               # the crate proper — a lib so tests can reach the modules
│     ├─ main.rs              # thin binary: two listeners, graceful shutdown, subcommands
│     ├─ config.rs            # env → Config, fail fast on missing secrets
│     ├─ seed.rs              # `youwin-server seed` — dev rows via the real pipeline
│     ├─ export.rs            # `export` — posts.json + a markdown tree           (M5)
│     ├─ backup.rs            # `backup` — VACUUM INTO, dated, 30 kept            (M5)
│     ├─ cache.rs             # Cloudflare purge-on-write, off unless configured  (M5)
│     ├─ tag.rs               # canonical form + href — shared by render, db, view (M5)
│     ├─ mood.rs              # the seven moods — shared by db, API, export, pet  (M7)
│     ├─ url.rs               # percent-encoding for tag paths and ?q=            (M5)
│     ├─ db/{mod,posts,sessions,search,tags,familiar}.rs  # pools + every statement
│     ├─ auth/{mod,password,session,middleware,ratelimit}.rs
│     ├─ familiar/            # the pet — pure state machine, no schema         (M6)
│     │  ├─ mod.rs            # the five dimensions + compute()
│     │  ├─ topics.rs         # keyword taxonomy → topic blend
│     │  ├─ mood.rs           # mood hashtags + keyword inference
│     │  ├─ energy.rs         # decay, bursts, learned circadian phase
│     │  ├─ render.rs         # state → kaomoji, compositionally
│     │  ├─ stats.rs          # vitals + the character sheet
│     │  └─ cache.rs          # the five-minute snapshot, and its fast-forward
│     ├─ public/              # youwin.dev :8080
│     │  ├─ mod.rs            # Router — read pool only
│     │  ├─ assets.rs         # Vite manifest → hashed stylesheet URL, read at boot
│     │  ├─ routes.rs
│     │  └─ view/             # maud: layout, pages, post, atom, time_fmt, familiar
│     ├─ write/               # write.youwin.dev :8081
│     │  ├─ mod.rs            # Router — read + write pools
│     │  └─ routes/{auth,posts,preview}.rs
│     ├─ render/markdown.rs   # markdown → html + text + tags, in one pass
│     └─ error.rs             # AppError → IntoResponse
├─ web/                       # pnpm; builds BOTH stylesheets and the SPA
│  ├─ vite.config.ts
│  └─ src/
│     ├─ theme.css            # mistwood tokens — single source of truth, imported by both
│     ├─ public.css           # tailwind + theme, no DaisyUI → the public site
│     ├─ app.css              # tailwind + DaisyUI + theme → the SPA
│     ├─ lib/{api,session,pwa}.ts
│     ├─ routes/{Feed,Permalink,Login,Drafts,Search,Settings}.tsx
│     └─ components/{Composer,PostCard}.tsx
├─ .github/workflows/
│  └─ deploy.yml              # build, test, ship, activate — the normal path
└─ deploy/
   ├─ youwin.dev.caddy        # both blocks; installs to /etc/caddy/conf.d/
   ├─ youwin.service
   ├─ youwin-backup.{service,timer}
   └─ activate-youwin         # the one command CI may run as root
```

## Data model

SQLite, WAL. One file, `/var/lib/youwin/youwin.db`.

There is no `users` table. There is one user and the password hash lives in the
environment; a table modelling a user would be a table with one row and no purpose.

```sql
CREATE TABLE posts (
  id          INTEGER PRIMARY KEY,           -- rowid; monotonic, used for ordering
  public_id   TEXT    NOT NULL UNIQUE,       -- 12 random bytes, base64url (16 chars)
  parent_id   INTEGER REFERENCES posts(id) ON DELETE CASCADE,
  root_id     INTEGER NOT NULL,              -- thread head; equals id for roots
  body        TEXT    NOT NULL,              -- markdown source — the authority
  body_html   TEXT    NOT NULL,              -- rendered + sanitized, cached at write
  body_text   TEXT    NOT NULL,              -- plaintext, for OG descriptions and search
  visibility  TEXT    NOT NULL DEFAULT 'public'
                CHECK (visibility IN ('public','unlisted','draft')),
  mood        TEXT                           -- M7; NULL is "did not say", not neutral
                CHECK (mood IS NULL OR mood IN ('content','contemplative','tired',
                       'excited','melancholy','chaos','neutral')),
  created_at  INTEGER NOT NULL,              -- unix millis, UTC
  updated_at  INTEGER NOT NULL,
  edited_at   INTEGER,                       -- null until the body changes post-publish
  deleted_at  INTEGER                        -- soft delete; rows are never removed
);

CREATE INDEX idx_posts_feed   ON posts (created_at DESC, id DESC) WHERE deleted_at IS NULL;
CREATE INDEX idx_posts_root   ON posts (root_id, created_at)      WHERE deleted_at IS NULL;
CREATE INDEX idx_posts_parent ON posts (parent_id)                WHERE deleted_at IS NULL;

CREATE TABLE sessions (
  token_hash   BLOB    PRIMARY KEY,          -- SHA-256 of the cookie value, never the value
  created_at   INTEGER NOT NULL,
  expires_at   INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,
  user_agent   TEXT,
  ip           TEXT
);
CREATE INDEX idx_sessions_expiry ON sessions (expires_at);
```

Notes on the choices that aren't obvious:

- **`root_id` is denormalized.** Threads here are self-reply chains, so walking
  `parent_id` recursively would be N queries deep. `root_id` makes a whole thread one
  indexed range scan. Set it at insert: `root_id = parent.root_id`, or `id` for a root
  (needs a second `UPDATE` after insert, or precompute the rowid — do the update, it's
  one statement inside the same transaction).
- **Three body columns.** `body` is what you typed and what an edit form loads.
  `body_html` is rendered once at write, so the public site's read path is a bare `SELECT`
  into a template — it never runs a markdown parser. `body_text` feeds OG descriptions and
  FTS without re-stripping tags on every request. If the render pipeline changes, a
  `youwin-server rerender` subcommand rebuilds the two derived columns from `body`.
- **Soft delete.** `deleted_at` rather than `DELETE`, so an accidental swipe on a phone
  is recoverable. Every query carries `WHERE deleted_at IS NULL`; the partial indexes
  match that predicate so it costs nothing.
- **Session tokens are stored hashed.** A database leak (or a stray backup on a laptop)
  then doesn't hand over live sessions.

**Migrations** are `sqlx` migrations: numerically-prefixed files under
`crates/server/migrations/`, embedded by `sqlx::migrate!()` at compile time and applied on
boot against the write pool. sqlx tracks applied versions and their checksums in
`_sqlx_migrations`, so an edited migration that already ran is a startup error rather than
silent drift.

`sqlx::migrate!()` is a macro from the `migrate` feature — it needs no `cargo sqlx` and no
database at build time. The only thing the CLI would have done is create the file with a
timestamp prefix, so create it by hand: `0001_init.sql`, `0002_add_fts.sql`. The version is
whatever leading integer you write, and it only has to increase.

### Search and tags (`0002_search_and_tags.sql`, M5)

**`posts_fts` is an external-content FTS5 table** over `body_text`: the index stores terms,
`posts` stores the text, `content_rowid=id` ties them together. Without `content=` there
would be a second copy of every post that could drift from the first. Three triggers keep
it in step, and the update trigger is `AFTER UPDATE OF body_text` — narrowed to the one
column, because a soft delete and a visibility flip both `UPDATE posts` and would otherwise
tear down and rebuild an index entry that did not change.

Soft-deleted and draft posts stay *in* the index; every search joins `posts` and filters
there. Keeping the index a plain mirror of the text means changing a post's visibility is
one `UPDATE` rather than an index edit that could half-apply.

The tokenizer is `porter unicode61 remove_diacritics 2`. Stemming is what makes the most
common failed search — singular for plural — work at all. It is English-only, and other
languages degrade to exact-token matching, which is what an unstemmed index would have
given anyway; strictly better, never worse.

**Hashtags are extracted by the pass that links them.** `render::markdown` returns
`Rendered { html, text, tags }`, and `posts::insert`/`update` write the tag rows inside the
same transaction as the post. One pass, one source: a tag that renders as a link is always
a tag the post is indexed under. The alternative — extract in SQL, link in Rust — has two
notions of what a tag is, and they disagree the first time the rules change.

`tags.tag` holds the lowercased form and `tags.display` the casing first written, so a page
can title itself `#TypeScript`. Lowercasing happens in Rust, **not** via `COLLATE NOCASE`,
which folds ASCII only and would file `#Café` and `#café` as two tags.

Because the sanitizer denies relative hrefs, hashtag links needed an exception:
`UrlRelative::Custom` passes `/t/<slug>` through and removes every other relative URL. The
narrow rule is the point — it describes the one shape the renderer emits rather than
declaring internal links generally fine.

One consequence to note: ammonia's blanket `link_rel` puts `nofollow` on tag links too,
since the filter sees attributes one at a time and cannot condition `rel` on `href`. Tag
pages stay crawlable through `/tags`, which is in the nav on every page, so this costs
nothing worth adding a filter for.

## Concurrency

**Two pools, because SQLite has exactly one writer.** WAL lets readers run concurrently
with the writer, but two connections attempting to write at once produce `SQLITE_BUSY`.
Rather than retry-looping on that, make it unrepresentable:

```rust
// Write pool: exactly one connection, so writers queue in sqlx instead of
// colliding in SQLite. Opened first — it runs the migrations.
let write = SqlitePoolOptions::new().max_connections(1).connect_with(opts.clone()).await?;
sqlx::migrate!().run(&write).await?;

// Read pool: concurrency for the public site. query_only is a guard, not a
// permission — see below.
let read = SqlitePoolOptions::new()
    .max_connections(4)
    .after_connect(|c, _| Box::pin(async move {
        sqlx::query("PRAGMA query_only = ON").execute(c).await?;
        Ok(())
    }))
    .connect_with(opts)
    .await?;
```

The public router's state holds `read` alone. The authoring router's state holds both:
session validation reads, and bumping `last_seen_at` writes. A write accidentally routed
to the read pool fails loudly at `query_only` instead of becoming an intermittent
`SQLITE_BUSY` under load.

**Do not use `SqliteConnectOptions::read_only(true)` for the read pool.** A file-level
read-only connection to a WAL database can't create the `-shm` file, so it depends on a
read-write connection already being open — which works right up until pool startup order
changes and then fails in a way that's miserable to diagnose. `PRAGMA query_only`
enforces the same intent at the statement level with no ordering dependency.

**Connection options**, set once on `SqliteConnectOptions` and shared by both pools:

```rust
SqliteConnectOptions::from_str(&cfg.database_url)?
    .create_if_missing(true)
    .journal_mode(SqliteJournalMode::Wal)   // persisted in the file
    .synchronous(SqliteSynchronous::Normal) // safe under WAL: fsync per checkpoint
    .foreign_keys(true)                     // sqlx defaults this on; state it anyway
    .busy_timeout(Duration::from_secs(5))   // safety net for checkpoint stalls
    .pragma("temp_store", "MEMORY")
```

## Queries: runtime-checked, no build tooling

**No `cargo sqlx`, no `.sqlx/`, no `DATABASE_URL` at build time.** That rules out the
compile-time-checked `query!` / `query_as!` macros, which verify SQL by connecting to a
live database during compilation and otherwise need metadata that only `cargo sqlx prepare`
can generate. Every query is therefore the runtime-checked form:

```rust
#[derive(sqlx::FromRow)]
struct Post { id: i64, public_id: String, parent_id: Option<i64>, /* … */ }

sqlx::query_as::<_, Post>(
    "SELECT id, public_id, parent_id, body_html, created_at
       FROM posts
      WHERE deleted_at IS NULL AND parent_id IS NULL AND visibility = 'public'
        AND (created_at, id) < (?1, ?2)
      ORDER BY created_at DESC, id DESC
      LIMIT ?3",
)
.bind(cursor.created_at).bind(cursor.id).bind(limit)
.fetch_all(&state.read)
.await?
```

Keep the `macros` feature on regardless. Enabling it costs nothing — only *invoking*
`query!`/`query_as!` triggers the compile-time database check — and it carries
`#[sqlx::test]`, which provisions a fresh migrated database per test. That matters, because
tests are now the *only* thing standing between a renamed column and a 500 in production.

So the safety net moves from the compiler to the test suite, and that has to be a real
commitment rather than an intention. The mitigations:

- **All SQL lives in `db/posts.rs` and `db/sessions.rs`.** Roughly a dozen statements
  total. Nothing in a handler, ever — a query you can't find is a query no test covers.
- **One `#[sqlx::test]` per statement**, asserting shape rather than just success. A test
  that fetches at least one row of every `FromRow` struct catches column renames,
  `FromRow` field mismatches, and bind-order errors, which is precisely the set the macros
  would have caught at compile time. This is an M1 deliverable, not a follow-up.
- **Bind order is now load-bearing.** `?` placeholders bind positionally, so swapping two
  `.bind()` calls of the same type is a silent logic bug the compiler cannot see. Prefer
  numbered placeholders (`?1`, `?2`) where a statement takes more than two binds of like
  type.

**The upside of dropping the macros:** the SQLite nullability-inference problem goes with
them. `query_as!` routinely decides a `NOT NULL` column is nullable and forces `as "id!: i64"`
annotations at every call site; with `FromRow`, the struct field type simply is the
contract. `Option<i64>` for `parent_id`, `i64` for `id`, no annotations anywhere.

**What sqlx still buys:**

- No `spawn_blocking` discipline. sqlx-sqlite runs each connection on its own background
  thread and hands back a future, so nothing blocks the reactor and no handler can forget
  to offload.
- Pooling, and with it the read/write split above.
- Ergonomic transactions (`pool.begin()` / `tx.commit()`), which matter for the one place
  that needs them: `INSERT … RETURNING id` followed by the `root_id` fixup for a new thread
  root.
- `#[sqlx::test]` and `sqlx::migrate!()`, neither of which needs the CLI.

**What it doesn't buy:** the sqlx SQLite driver is not async I/O down to the syscall — it's
a worker thread per connection behind an async facade. The win is ergonomics and not having
to hand-roll the offload; it is not a different execution model. At this scale that's
irrelevant anyway — the database is a few megabytes and lives in page cache. The constraint
was never throughput, only not blocking the reactor.

## Public site — `youwin.dev`

```
GET  /                    feed, newest first
GET  /?before=<cursor>    older page
GET  /p/:public_id        permalink — the post plus its whole thread
GET  /search?q=           full-text search                                (M5)
GET  /t/:tag              everything carrying one hashtag                 (M5)
GET  /tags                every tag in use, most-used first               (M5)
GET  /about
GET  /familiar            the pet, at full size, with its character sheet      (M6)
                          (the authoring host has its own pair — see M8)
GET  /feed.xml            Atom, public roots
```

**Search is a GET form in the header**, so a search is a URL you can link, bookmark and go
back to — and the site still ships zero JavaScript. This is the one line of the public CSP
that had to loosen: `form-action` went from `'none'` to `'self'`, and it does **not** fall
back to `default-src`, so that line alone governs whether the form works at all.

**Every input becomes a valid query or none.** `search::fts_query` splits on non-alphanumeric
characters and quotes each token, which makes every token a literal phrase and takes FTS5's
operators — `AND`, `OR`, `NOT`, `NEAR`, `*`, `^`, `:` — off the table along with every way
to write a syntax error. Tokens are ANDed, so more words narrow. The trade is deliberate: no
boolean operators, in exchange for a public text box where no input is a 500. A stray
apostrophe is far likelier than a hand-written `NEAR(a b, 3)`.

**Results are ordered by recency, not `bm25()`.** Searching a personal archive is re-finding
something you know you wrote, where "when" is the strongest remaining clue; relevance
scoring across 300-character posts mostly ranks on length. It also means search paginates
with the same keyset cursor as the feed rather than needing a score-based one.

**Snippet markers are control characters.** FTS5 wraps matched terms in delimiters of our
choosing; choosing `<mark>` would mean interpolating database output into a page unescaped,
which is a privilege only `body_html` has earned. ASCII STX/ETX come back as inert text,
`search::segments` splits on them, and maud escapes each run.

**Search pages are always `noindex`**, and so is an empty tag page. A tag nothing has ever
used is a 404 rather than an empty page, which is what keeps `/t/<anything>` from being an
unbounded space of thin pages for a crawler to walk. A tag whose posts were all deleted
keeps its row and stays a 200 — the URL meant something once and may again.

`maud` rather than `askama`. The templates are ~6 functions returning `Markup`; they
compose as functions, auto-escape by construction, and — the actual argument — the
authoring app can serve an authenticated `/preview/:id` that calls *the exact same
template functions*. Draft preview is then pixel-identical to the published page for
free, instead of an approximation that drifts. Tailwind scans them via
`@source "../../crates/server/src/**/*.rs"`.

**Pagination is links, not scroll.** `?before=<cursor>` with the same keyset predicate as
the API. The cursor is base64url of `{created_at}:{id}`, opaque. Crawlable, linkable, and
back-button-correct — strictly better than infinite scroll for an archive.

**OG tags are just template variables.** `<title>`, `og:title`, `og:description` (first
~200 chars of `body_text`), `og:url`, `og:type=article`, `article:published_time`,
`twitter:card=summary`. This is the entire replacement for what would have been a
sentinel-splicing, escape-into-attributes, dev-reload machine on a shared origin.

**Visibility.** `public` renders. `unlisted` renders at its permalink but never appears in
the feed or the Atom document. `draft` is a 404 — indistinguishable from a bad id.

**No JavaScript.** Not "progressively enhanced" — none. That makes `script-src 'none'` an
honest CSP, and it means the page is complete when the HTML arrives.

**CSS is one external hashed file**, not inlined. Inlining would re-send the same ~9KB on
every navigation; an immutable `public.<hash>.css` is fetched once and reused across the
feed, permalinks, and about. Vite emits `manifest.json`; the server reads it **at startup**
to resolve the hashed URL, so `cargo build` never depends on the frontend build having run.
A missing manifest is a fatal startup error naming the pnpm command to run — the site
cannot render without its stylesheet, and saying so beats serving unstyled HTML.

The app also mounts a `ServeDir` at `/assets`. In production Caddy's `handle /assets/*`
block matches first, so the process still never touches a CSS byte; this exists so
`cargo run` on its own produces a styled site, and so a misconfigured Caddy degrades to
"correct but uncached" rather than "unstyled". Without it the request falls through to the
404 handler and the browser discards a stylesheet served as `text/html`.

**This bundle must also be reachable from the authoring host.** `/preview` renders through
these same templates, so it links the public stylesheet by its root-absolute path — which
resolves against `write.youwin.dev`, where the SPA's assets live instead. Caddy's write
block therefore serves `/assets/public-*` from the public root, ordered ahead of the SPA
assets, and Vite's dev proxy forwards `/assets` to the public listener for the same reason.
Without both, the preview renders unstyled, which is precisely the thing it exists to show.

**Edge caching.** Cookieless and JS-free means Cloudflare can cache the whole surface.
`Cache-Control: public, max-age=60, s-maxage=300` plus a Cloudflare cache rule (CF does not
cache HTML by default). The cost is that an edit or delete takes up to five minutes to
appear.

**Purge on write (M5) closes that gap, and is off unless configured.** Running on the TTL
alone stays a supported configuration; setting `YOUWIN_CF_ZONE_ID` and
`YOUWIN_CF_PURGE_TOKEN` turns it on. The token needs a second Cloudflare API token scoped
to `Cache Purge` — the DNS-01 token Caddy uses is scoped to DNS, and widening it to save
provisioning one more would trade a real boundary for a small convenience.

It purges **everything**, not a URL list. One write invalidates more than it looks like: a
reply changes its own permalink, every *other* permalink in the thread (each renders the
whole thread), the feed, the Atom document, each of its tag pages, the tag index, and any
cached search results that matched it. Enumerating that correctly is a bug waiting to
happen whose failure mode is a stale page nobody notices; purging everything cannot be
incomplete. The cost is a few origin renders and one re-fetch of a 10 kB stylesheet, on a
site written to a handful of times a day.

The call is spawned and never awaited: the write has already committed, so no outcome there
should turn a successful post into an error. A failure is logged and the TTL takes over.
The cost of all this is `reqwest` + `rustls` — about 28 crates on a binary that otherwise
makes no outbound connections, which is the honest reason this stayed optional for so long.

## The Familiar (M6)

A kaomoji that reads the archive's temperature. It sits above the feed on page one and has
its own page at `/familiar`. [`familiar-design.md`](familiar-design.md) is the specification;
this is what the implementation does differently and why.

**It has no schema and writes nothing.** Every field is a pure function of
`(posts, previous state, now)`, so it lives happily on the public listener behind the read
pool. There is no pet table, no migration, and no state to get out of sync with the posts —
delete a post and the pet forgets it on the next recompute.

**Five dimensions.** Form comes from the dominant topic over a 50-post window, stage from
total posts, mood from the most recent post that expressed one, energy from decay since the
last post plus bursts, and phase from this hour judged against the hours the archive is
actually written in. Rendering is compositional — eyes, mouth, crown and base are
independent lookups — so a new mood costs five table entries rather than a new drawing for
every combination it could appear in.

**The phase modifier is applied at render time, never stored.** This is the one substantive
correction to the prototype. Energy is recomputed on every cache miss, so a modifier folded
into the stored value compounds once per page load: a pet in its peak hours gains 0.10 per
visit and can be walked to hyper by holding down F5, and one visited through the night is
driven to the floor by traffic rather than by silence. `base_energy` carries decay and
bursts; `energy` is that plus the offset, computed fresh and discarded.

**The cadence factor runs the other way from the prototype's.** A burst of three posts means
more from someone who manages one a day than from someone who writes hourly. The prototype's
`6 / cadence` rewards the fast writer, against the stated intent in both its own docstring
and the design's energy section.

**Cadence is measured between sittings, not between posts (M8).** The original measured the
mean gap between posts over the last week, which cannot tell a weekly rhythm from abandonment:
somebody writing five notes every Sunday measured a cadence of five minutes, got a decay
half-life pinned to its two-hour floor, and had a pet lying flat from Monday until the next
weekend while doing exactly what they always did. Bursty writing is what a microblog looks
like, so this was wrong for a large share of the writers it had. Posts more than 45 minutes
apart now start a new sitting, and `familiar::baseline` reads the rhythm off two quantiles of
the most recent sixteen gaps between sittings — the median, and the 75th percentile the decay
half-life is set from. Quantiles rather than a mean and a spread because the distribution is
heavily right-skewed, and because the median absolute deviation collapses to zero for anyone
whose gaps are mostly identical, which is most people. The half-life is clamped to a fortnight
at the top: without it the formula concludes that a blog with two posts a year apart is on
schedule six months later, which is true and leaves a pet that never moves.

**Topic keywords match at word boundaries.** Bare substring matching fires `fn` on "often",
`xp` on "experience" and `ecs` on "specs" — collisions that are invisible until the pet is
inexplicably a biped. Matching the start of a word keeps plurals and simple inflections
without a stemmer.

**Mood is a field on the post, picked in the composer (M7).** It began as a hashtag —
`#tired` in the body — which worked and needed no new syntax, but was only discoverable if
you already knew the seven names. In practice that made it a feature you had to remember
rather than one you could see. It is now a `mood` column and a picker beside the visibility
select, and hashtags are back to being ordinary tags.

The column is **nullable, and NULL is not `neutral`**. NULL means "did not say" and the
familiar infers a mood from the text; a stored value means "did say" and nothing overrides
it — including a stored `neutral`, which means "nothing to report" and is the only way to
tell the pet that a post about a broken deploy was not a crisis. That distinction is why
`PATCH` takes a doubly-optional mood: the key absent leaves it alone, `null` clears it, and
one layer of `Option` could not express both.

**Mood never renders on youwin.dev.** It feeds the kaomoji and the aggregate on
`/familiar`, and appears per-post only in the composer. Changing it does not set
`edited_at` either — nothing on the public site shows it, so correcting one months later is
not an edit to what was published.

`0003_post_mood.sql` backfills from the hashtags that used to carry it, reading `post_tags`
rather than pattern-matching bodies: those rows were written by the pass that decided what
a hashtag *was*, so they are the extraction rules rather than an approximation. The tags
themselves are left in place — `/t/tired` still works, and rewriting published bodies to
delete a word is not something a migration should do quietly.

**It feeds on public posts only, replies included.** A reply is something that was sat down
and written. An unlisted post is not counted — the post count renders on a public page, and
including them would let a visitor infer that unlisted posts exist and roughly when, which
is the one thing `unlisted` protects.

**One five-minute snapshot, fast-forwarded.** Held in memory on `PublicState`, matched to the
`s-maxage=300` the pages are served with. The useful consequence is that the pet reflects the
gap between the *last visit* and the *last post*: through a quiet week nothing runs at all,
and the next visitor triggers one catch-up and sees a pet that has plainly been alone. Two
concurrent misses both recompute rather than queueing behind a lock held across a query;
the snapshot refuses to move backwards, so the later one wins.

**No ASCII box.** The design draws the stats in a 47-character frame. The kaomoji keeps its
`<pre>` — that is genuinely fixed-width art — but the frame does not reflow, and on a phone
it means either a horizontal scrollbar or a broken picture. The bars stay, because they are
the texture that makes it read as a terminal pet; the frame is the site's own card, which
already knows how to be narrow. The picture is centred with CSS rather than padded by
character count, which would be wrong anyway: half these glyphs are East Asian Ambiguous
and render at a width the server cannot know.

**Hours and days are UTC**, like every other timestamp here. Phase learning is relative to
the archive's own histogram, so it self-calibrates and the absolute hours stop mattering
once there is a fortnight of data — before that a default schedule is blended in, weighted
by how much history exists, so there is no threshold at which the pet's sense of time
lurches.

## Authoring API — `write.youwin.dev`

Caddy serves the SPA shell off disk (`try_files {path} /index.html`). axum serves JSON
only, plus the one HTML route:

```
POST   /api/auth/login    {password}  → Set-Cookie
POST   /api/auth/logout               deletes this session
POST   /api/auth/logout-all           deletes every session
GET    /api/auth/me                   401 when unauthenticated — the auth probe

GET    /api/feed?cursor=&limit=20     ALL visibilities, flagged
GET    /api/posts/:public_id          post + thread
GET    /api/drafts
GET    /api/search?q=&cursor=         ALL visibilities, drafts included    (M5)
POST   /api/posts         {body, parent_public_id?, visibility, mood?}
PATCH  /api/posts/:id     {body?, visibility?, mood?}   mood: absent leaves, null clears
DELETE /api/posts/:id                 sets deleted_at

GET    /api/familiar                  the pet, as JSON                     (M8)
POST   /api/familiar/draft {body, visibility?, mood?}                      (M8)
                                      the pet as that draft would leave it.
                                      POST only because a draft is too long for a
                                      query string — nothing here writes

GET    /preview/:public_id            HTML — the public templates, authenticated
```

Every route here is authenticated except `login`. There is no public surface on this
origin, which is what makes the service worker simple.

Errors are a single shape — `{"error": {"code": "…", "message": "…"}}` — produced by one
`AppError: IntoResponse` impl. `code` is a stable machine string; `message` is for me,
not for users.

## Auth

**Login.** `argon2id` verify against `YOUWIN_PASSWORD_HASH` (a PHC string in the
environment — never a plaintext password, never in the repo). OWASP baseline params:
m=19456 KiB, t=2, p=1. `youwin-server hash-password` generates it, reading the terminal
without echoing and never taking the password as an argument, which would put it in `ps`
output and shell history. Serving without a valid hash is a startup failure, not a login
that can never succeed.

When stdin is a pipe it reads lines instead, so provisioning can be scripted — and it
**strips a leading byte-order mark**, because PowerShell prepends one when piping to a
native command. Hashing `U+FEFF` + the password yields a credential that cannot be typed,
and the resulting failure is invisible in every tool you would reach for. A BOM at the
start of a password is never intentional.

**Session.** 32 bytes from `OsRng`, base64url-encoded → the cookie value. `SHA-256` of
that value is the primary key in `sessions`. 90-day expiry, refreshed lazily: if
`last_seen_at` is more than a day old, bump both it and `expires_at`. So a phone that
opens the app weekly never gets logged out, while a truly idle session dies on schedule.

**Cookie.** `__Host-yw_session`, `HttpOnly; Secure; SameSite=Lax; Path=/`, set only by
`write.youwin.dev`. The `__Host-` prefix forbids a `Domain` attribute, so the cookie is
bound to that exact host and **cannot** be sent to `youwin.dev`. The public site is
therefore anonymous by construction: "did a draft leak because a cookie was present?" is
not a reachable bug, and it's also what makes the public surface edge-cacheable.

**The guard is structural.** Every authenticated route lives in a sub-router carrying
`require_session` as a `route_layer`; login and health are the only two outside it.
Adding a route means adding it to that sub-router — there is no per-handler annotation
to forget, and forgetting one cannot leave a route open. `route_layer` rather than
`layer` so it runs only on *matched* routes: an unknown path 404s instead of 401ing and
confirming which routes exist.

**CSRF** needs no token. Every state-changing route is `POST`/`PATCH`/`DELETE`, and
`SameSite=Lax` withholds the cookie from cross-site requests on those methods. As
defense in depth, a middleware rejects non-`GET` requests whose `Origin` isn't
`https://write.youwin.dev`. A *missing* `Origin` is allowed: browsers always send it on
cross-origin writes, so its absence means a non-browser client carrying no ambient cookie
to abuse — rejecting it would break `curl` for no security gain.

**Brute force.** In-process per-IP limiter: 5 failures → 15-minute lockout, exponential
after. Single process, so a `Mutex<HashMap>` is the whole implementation. The login
handler also holds a ~250 ms floor regardless of outcome, so response time leaks
nothing. Client IP comes from `CF-Connecting-IP` (Cloudflare) falling back to
`X-Forwarded-For` — trustworthy only because the backend binds loopback and Caddy is the
sole possible peer. Worth a comment in the code so nobody later binds it to `0.0.0.0`.

Optionally, Cloudflare Access in front of `write.youwin.dev` as a second gate. The
password is the real control; this would just keep the login form off the open internet.

**Dev.** `Secure` and the `__Host-` prefix are config-driven (`YOUWIN_COOKIE_SECURE`);
in dev the cookie is a plain `yw_session` so `http://localhost:5173` works without
special-casing browsers.

## Markdown

`pulldown-cmark` → filter the event stream → `ammonia` sanitize → store.

The one non-default behavior that matters: **`SoftBreak` is rewritten to `HardBreak`.**
CommonMark collapses a single newline into a space, which is wrong for a microblog —
people expect the line breaks they typed. Mapping the event is cleaner than
preprocessing the source text.

Allowed: emphasis, strong, strikethrough, inline code, fenced code, links, blockquote,
lists, `hr`. Rejected: raw HTML (dropped at the parser, then again by the sanitizer),
headings (a 300-character post has no sections), images (no upload path in v1 — the
sanitizer strips `<img>`, and allowing it later is a one-line allowlist change).

Links get `rel="nofollow noopener noreferrer"` and a scheme allowlist of
`http`/`https`/`mailto`. Bare URLs autolink.

Limits: soft 500 characters with a meter in the composer, hard 4000 characters rejected
server-side, plus a `RequestBodyLimitLayer` well below that as a blunt outer guard.

`body_html` is inserted into maud with `PreEscaped` — it is the one place escaping is
bypassed, which is exactly why sanitization happens at write time and is tested.

## Authoring frontend

Vite + `vite-plugin-solid` + `@solidjs/router`. Tailwind v4 via `@tailwindcss/vite`
(CSS-first config — no `tailwind.config.js`), DaisyUI v5 as a CSS plugin. A pure SPA:
axum never serves its shell, so there is no SSR path, no shell-reload branch, and no
per-request HTML on this origin.

```
/            feed — all visibilities, composer at the top
/p/:id       permalink + thread + reply composer
/login
/drafts
/settings    change password, list sessions, log out everywhere
```

- Auth state is one `createResource` on `/api/auth/me`. The typed fetch wrapper in
  `lib/api.ts` intercepts every 401, clears that resource, and routes to `/login` — so
  no component ever handles expired sessions.
- Feed is `createResource` with an `IntersectionObserver` sentinel for infinite scroll.
  Infinite scroll is fine *here* — this surface isn't crawled and isn't linked into.
- Composer: auto-growing textarea, Ctrl/⌘+Enter to post, character meter that turns
  `warning` past the soft limit. Posting inserts optimistically and rolls back on error.
- A "preview" affordance opens `/preview/:id` — the real public rendering, not a mimic.

## PWA

`vite-plugin-pwa` (`generateSW`). `display: standalone`, `theme_color` matching
`base-100`, 192/512 icons plus a maskable variant, all generated from the palette by
[`web/scripts/generate-icons.mjs`](web/scripts/generate-icons.mjs) — a committed script
rather than a folder of binaries nobody can regenerate when the theme moves.

Because **every route on this origin is authenticated**, the usual rules collapse: there
is no public content to accidentally cache. What's left:

- **`navigateFallbackDenylist: [/^\/api\//, /^\/preview\//]`.** Without the first, the SW
  answers API calls with the HTML shell and every fetch dies at `JSON.parse`. The second
  matters for the opposite reason: `/preview` is *server-rendered HTML on this origin*, not
  an SPA route, so the shell must not stand in for it.
- `NetworkFirst` for `GET /api/posts*` only, in a `youwin-api` cache. `/api/auth/*` is
  deliberately absent — caching it would let a signed-out device keep answering as though
  it were signed in. `method: "GET"` means a POST/PATCH/DELETE never reaches the worker.
- On sign-out, `caches.delete("youwin-api")`. The cache name is written in two places
  (`vite.config.ts` and `lib/pwa.ts`) and they must stay in step.
- `sw.js` and `manifest.webmanifest` are served `no-cache` (see the Caddy block), or a
  stale SW pins an old build indefinitely.

**Updates are offered, not applied.** `registerType: "prompt"` with `skipWaiting: false`
and `clientsClaim: false`: a new build sits in *waiting* until the shell's Reload button
posts `SKIP_WAITING`. An automatic reload can land mid-sentence and take an unposted draft
with it — the one loss this app must not risk. The cost is that a freshly installed worker
does not control the page until the next navigation, which is the correct trade.

Four things that cost real time to find, all verified against `pnpm run preview`:

- **`registerSW({ immediate: true })` is load-bearing.** By default it defers registration
  to the window `load` event, but `initPwa()` runs from Solid's `onMount`, which in a
  production build can fire *after* load has passed — so the listener never runs and the
  worker never registers. Dev hides this completely, because the plugin injects its own
  registration there. The symptom is a PWA that installs but has no offline support and
  never offers an update.
- **The dev service worker omits `runtimeCaching`.** `devOptions` generates a stripped
  worker that honours `navigateFallback` and nothing else, so the API cache does not exist
  in dev. Concluding "caching is broken" from a dev session would be wrong; verify against
  the preview build.
- **`vite preview` needs a *different* proxy from the dev server.** In dev, Vite serves
  modules from `/src` and never touches `/assets`, so that prefix can be forwarded whole.
  In preview the SPA's own bundle lives under `/assets`, and forwarding it sends the app's
  JavaScript to the public listener — nothing mounts at all. Preview forwards only
  `/assets/public-`, which is exactly what the Caddy write block does.
- **Offline must distinguish "unreachable" from "401".** Because `/api/auth/me` is never
  cached, it fails offline; treating that failure as "signed out" bounces you to a login
  form that cannot be submitted, while a perfectly readable cached feed sits behind it.
  `api.ts` throws a distinct `NetworkError`, and `loadSession` falls back to the last
  server-confirmed session in `localStorage`. That fallback is safe because the only way to
  have one is to have signed in on this device, and signing out clears it alongside the
  cache.

## Theme — "mistwood"

Deep, cold, low-chroma green; the palette of standing water and lichen under canopy.
All colors are OKLCH so lightness steps are perceptually even.

| Token | Value | Role |
|---|---|---|
| `base-100` | `oklch(17% 0.016 162)` | forest floor — page |
| `base-200` | `oklch(21% 0.020 162)` | post cards |
| `base-300` | `oklch(26% 0.024 162)` | borders, hover |
| `base-content` | `oklch(91% 0.014 152)` | mist — body text |
| `primary` | `oklch(74% 0.105 158)` | lichen glow — actions, links |
| `secondary` | `oklch(63% 0.055 196)` | cold water — timestamps, meta |
| `accent` | `oklch(82% 0.085 128)` | new growth — highlights |

`--depth: 0` and `--noise: 0`: mist has no hard edges, so cards are separated by a
1px `base-300` border rather than a drop shadow. The one flourish is a fixed radial
vignette on `body` — a faint green lift at the top, falling to near-black at the
bottom — so the page reads as depth rather than as a flat dark rectangle.

Dark only. `color-scheme: dark` is set; a *deep forest* doesn't have a day mode.

**Two bundles, one source of truth.** [`web/src/theme.css`](web/src/theme.css) holds the
OKLCH literals plus the base layer and the `.post-body` rules that style server-rendered
markup. Then:

- [`web/src/public.css`](web/src/public.css) — Tailwind + theme, **no DaisyUI**. A
  read-only site needs typography and layout, not components. Lands around 5KB.
- [`web/src/app.css`](web/src/app.css) — Tailwind + DaisyUI + theme, for the SPA.

**The tokens must live in `@theme static`, not `:root`.** This bit me at M1 and the
failure is quiet, so it is worth stating plainly:

- DaisyUI reads `--color-*` at *runtime*, so `:root` satisfies it — verified on 5.7.16,
  no `@plugin "daisyui/theme"` block needed, `.btn-primary` resolves correctly.
- **Tailwind generates utility classes only from `@theme`.** With the tokens in `:root`,
  `bg-base-200`, `border-base-300`, `text-secondary`, and `rounded-box` are never emitted
  at all. Nothing errors; the page just renders with browser defaults, and because
  hand-written `.post-body` rules using `var(--color-*)` *do* work, it looks like a
  partial theme rather than a build problem.

`static` is required too: it emits every variable regardless of whether Tailwind can see
it used. Usage-based pruning would strip out the values DaisyUI reads at runtime.

`color-scheme: dark` stays in a separate `:root` rule — it is a property, not a custom
property, so `@theme` cannot hold it.

## Deployment

**Caddy** — `deploy/youwin.dev.caddy`, installed to `/etc/caddy/conf.d/` where the box's
`import /etc/caddy/conf.d/*.caddy` picks it up alongside the static sites. Follows the house
style in `grindshell/server-configs/static/Caddyfile` (Cloudflare DNS-01, loopback backends,
two-tier cache, baseline security headers).

DNS-01 costs a non-stock Caddy: `dns.providers.cloudflare` is not in the Cloudsmith package,
so the binary has to be replaced via `caddy add-package` and then held back from `apt`, which
would otherwise restore the stock build and leave Caddy unable to load its own config. That
is a real maintenance cost, taken because DNS-01 is the only issuance path that keeps working
regardless of whether the Cloudflare proxy is on — TLS-ALPN cannot work behind it at all, and
HTTP-01 makes renewal depend on Cloudflare forwarding ACME challenges to the origin. The
alternative that avoids the custom build entirely is a Cloudflare Origin CA certificate,
rejected because it is trusted only by Cloudflare: turning the proxy off would break the site
for browsers, which is exactly the wrong failure mode for the escape hatch.

The config itself is not reproduced here. It runs to a hundred and forty lines that have to
be byte-correct to be worth reading; the real one is checked by `caddy validate` against a
real Caddy carrying the DNS-01 module during setup ([`deploy/README.md`](deploy/README.md)
step 5), and a copy in this file is checked by nothing. The copy that used to sit here had
drifted to the old `/var/www/youwin` roots, a stale CSP, and — worse — a directive form that
Caddy rejects outright. Read [`deploy/youwin.dev.caddy`](deploy/youwin.dev.caddy); what
follows is why it looks the way it does.

**Two cache tiers, and the second one is only half in this file.** Content-hashed assets get
`max-age=31536000, immutable`; server-rendered HTML gets `max-age=60` for the browser and
`s-maxage=300` for Cloudflare. The favicons and `robots.txt` sit between them at a day. The
`s-maxage` is inert on its own: Cloudflare does not cache HTML by default, so it also needs a
cache rule in the dashboard, and without that every request reaches the origin while this
file looks entirely correct. What the five-minute edge TTL costs, and the purge-on-write that
buys it back, are under **Purge on write** in *Public site* above.

**A list of paths needs a named matcher.** Every Caddy directive takes at most **one**
matcher token, so `handle /favicon.ico /robots.txt { … }` is not a shorter spelling of
anything — it fails `caddy validate` with "wrong argument count", and because the box loads
`conf.d/*.caddy` into one config, a rejected file takes down *every* site on the server, not
just this one. Both places that serve a set of paths (`@root_files` on the public host,
`@pwa` on the authoring one) bind the list to a matcher first.

**`handle` blocks are sorted by specificity, not by the order they appear in.** This is what
makes the preview work: `/preview` renders through the *public* templates, so it links the
public stylesheet by a root-absolute path that resolves against `write.youwin.dev`. A
`handle /assets/public-*` block on that host serves that one file out of the public root, and
it beats the sibling `/assets/*` block wherever either is written. Verified with
`caddy adapt`. Vite's hashed names cannot collide across the two builds, so it is safe even
if the ordering were ever to change.

**The public CSP is `default-src 'none'`, and it is honest.** A site that ships zero
JavaScript can assert that at no cost, which makes it the strongest policy on the box. Only
three things are opened up: `style-src 'self'` for the one stylesheet, `img-src 'self' data:`,
and — as of M5 — `form-action 'self'`, because the header carries a GET search form. That
last one is easy to get wrong: `form-action` does **not** fall back to `default-src`, so it is
the only line governing the submission, and leaving it at `'none'` blocks the search box while
every other directive still reads as correct.

**`write.youwin.dev` takes `X-Frame-Options: DENY`** rather than the public site's
`SAMEORIGIN`, because nothing should ever frame the composer. Nothing on that host is cached
at the edge either — it is all authenticated — and the PWA plumbing (`sw.js`, the manifest,
the icons) is served `no-cache` rather than off the immutable tier, since a cached service
worker pins an old build on every installed device until it expires.

**systemd** — one unit, `Type=exec`, `Restart=on-failure`,
`EnvironmentFile=/etc/youwin/secrets.env` (mode 0600, holds `YOUWIN_PASSWORD_HASH`).
Hardened: `NoNewPrivileges`, `ProtectSystem=strict` with `ReadWritePaths=/var/lib/youwin`,
`ProtectHome`, `RestrictAddressFamilies=AF_INET AF_INET6`, `MemoryDenyWriteExecute`.
Graceful shutdown on SIGTERM — both listeners, so WAL checkpoints cleanly on restart.

**DNS.** One new record for `write.youwin.dev`. DNS-01 issuance works identically; Caddy
handles the per-host certificate automatically.

**Deploys run from GitHub Actions**, on push to `master`. CI builds the frontends, runs the
test suite, builds the binary, ships a release directory, and asks the server to activate
it. The server has no build toolchain at all — no Rust, no C compiler, no source tree.

**Releases are directories and `current` is a symlink**, in the same layout the static sites
on the box already use — one directory per site under `/srv/sites`, named for the domain:

```
/srv/sites/youwin.dev/releases/<utc-timestamp>-<sha>/{bin,public,write,deploy}
/srv/sites/youwin.dev/current  -> releases/…   what systemd and the Caddy roots point at
/srv/sites/youwin.dev/previous -> releases/…   what --rollback goes back to
```

The binary and the assets it serves therefore change together or not at all. systemd
resolves `ExecStart` at start time, so the restart is the cutover. The database is not under
`/srv` at all — `/var/lib` is where the FHS puts application state — and no deploy goes near
it.

**Ownership deviates from the static sites deliberately.** There the site directory belongs
to `deploy`, which is why `activate-release` needs no `sudo` at all. Here it belongs to
`root` and only `releases/` is `deploy`-writable, because `current` selects the **binary**
systemd executes — and the nightly backup timer runs that binary too. A `deploy`-writable
`current` would mean anything uploaded to `releases/` could be executed as the `youwin` user
without passing the smoke test, the health check, or the rollback. Uploading a release and
*asking* for it to be activated is the intended power; swapping the running binary directly
is not.

**The privilege boundary is one sudoers line.** CI authenticates as an unprivileged `deploy`
user that can write only to `releases/`, and may run exactly one command as root:

```
deploy ALL=(root) NOPASSWD: /usr/local/bin/activate-youwin
```

`activate-youwin` validates the release name against a single permitted shape, refuses a
release with no asset manifest, **smoke-tests the new binary before stopping anything**,
flips the symlink with `mv -T` (atomic, unlike `ln -sfn` onto an existing link), restarts,
health-checks both listeners, and rolls back on its own if the site does not come up.

The smoke test is `youwin-server version`, which deliberately touches no configuration and
no database — so the only thing it can fail on is the dynamic loader. That is the failure
worth catching here, because it is the one that would otherwise take the site down *after*
the old process had already been stopped.

**glibc is the one real coupling, and it is asserted rather than assumed.** `sqlx`'s
`sqlite` feature bundles SQLite through `libsqlite3-sys`, and `reqwest` uses rustls, so the
binary links nothing but `libc`, `libm` and `libgcc_s` — measured, it needs at most
`GLIBC_2.38`. The server is **Debian 13, glibc 2.41**; the runner is pinned to
`ubuntu-24.04` (2.39) rather than `ubuntu-latest`, so a runner image bump cannot quietly
raise the floor. A workflow step compares the binary's highest required symbol version
against `SERVER_GLIBC` and fails the build before anything is uploaded.

The margin is not large in the direction that matters: Debian **12** (2.36) would not run
this binary, so "which Debian" was load-bearing rather than trivia. The escape hatch is one
line — build in a matching container (`container: rust:1-trixie`) — rather than a static
musl build, which `aws-lc-sys` in the rustls tree makes considerably less pleasant than it
sounds.

A subtlety worth recording, because it is easy to reason about backwards: the requirement
comes from which symbol versions the *used functions* last changed ABI at, not from the
builder's glibc. Building on a newer system does not by itself raise the runtime floor —
this source measures 2.38 whether built on Ubuntu 24.04 or Debian 13.

**What is deliberately not automated:** first-time provisioning (root, one-time), the
password hash (the site's only credential — it must not exist in a CI secret), DNS and the
Cloudflare cache rule (dashboard), the purge token (server-side, CI never sees it), and
`rerender`, because *when* to rebuild derived columns is a judgement about data rather than
a build step.

**There is no local deploy path, and that is the point.** A `deploy.sh` existed briefly and
was deleted: it had to guess at whether the pnpm on `PATH` was the Linux one or the Windows
one leaking in through WSL interop, whether `web/dist` was stale, and whether `/mnt/c`'s
0777 modes were about to be rsynced onto the server. All of that was scaffolding around a
problem CI does not have — it installs Linux `node_modules` from scratch on a clean runner —
and it broke the first time it was used in a checkout it had not been written for. One route
that runs on every push beats two where the rarely-used one quietly rots.

That also decides the bootstrap. Rather than a script to place the first release, the
runbook is **ordered** so CI can: install the unit but do not enable it, hand CI its key,
push, and let the first run go red at activation. `current` is populated by then, so
`hash-password` can run against the binary CI just delivered, and re-running the workflow
turns it green. The one intentionally-failing run is cheaper than a second deployment
mechanism.

A `.gitattributes` pins `eol=lf` for shell scripts, systemd units and the Caddyfile, so a
clone on a machine with `core.autocrlf=true` cannot produce an `activate-youwin` that dies
with `bad interpreter: /bin/bash^M`.

**Backups.** WAL means `cp` of the `.db` file is not a valid backup — the `.db` alone can be
missing every commit still sitting in the `-wal`. `youwin-backup.timer` runs two things
nightly, insuring against different failures:

- `youwin-server backup` — `VACUUM INTO`, which reads through one consistent snapshot of a
  live database and writes a compact self-contained file, with the site still serving. Done
  in-process rather than by shelling out to `sqlite3`, which keeps the server's only
  requirement a Rust toolchain. Writes to a `.part` and renames, so an interrupted run
  leaves a stray file rather than replacing yesterday's good backup with a truncated one.
  Keeps 30 dated files, and only ever deletes names matching exactly `youwin-YYYY-MM-DD.db`.
- `youwin-server export` — `posts.json` (every column, deletions included, enough to rebuild
  the database) plus a markdown tree with front matter. The real insurance policy: readable
  in ten years with no SQLite, no Rust, and no memory of how any of this worked.

**`youwin-server rerender`** rebuilds `body_html`, `body_text` and the tag rows from `body`,
which is the authority. Needed whenever the renderer changes — M5 is exactly that case, and
hashtags in existing posts are neither linked nor indexed until it runs. It deliberately
does not touch `updated_at` or `edited_at`: a re-render is not an edit, and marking an
archive as edited because a sanitizer rule changed would be a false claim about its history.
One transaction per post, so an interrupted run leaves a consistent database a second run
simply finishes.

## Milestones

| | | |
|---|---|---|
| **M0** | Skeleton | ✅ **Done.** Zola tree deleted. Workspace, config, the two pools, `sqlx::migrate!()` on boot (schema landed here — `migrate!()` wants a real migration), two listeners with health checks, `tests/pools.rs`. `theme.css` split into `public.css` + `app.css` |
| **M1** | **The entire public site** | ✅ **Done.** `youwin-server seed`, maud templates, feed + cursor pagination, permalinks, threads, Atom, markdown pipeline, asset-manifest lookup, themed 404. 26 tests green, covering every statement in `db/`. *Still to do before it is actually live: put the Caddy block and the Cloudflare cache rule on the box, alongside M2.* |
| **M2** | Auth | ✅ **Code done.** `hash-password`, login, sessions, structural guard, throttling, Origin check. 61 tests green. Deploy artifacts written to [`deploy/`](deploy/README.md) — **installing them on the server is still outstanding**, including the Cloudflare cache rule without which M1's caching headers are inert |
| **M3** | Authoring app | ✅ **Done.** Solid SPA — composer with optimistic insert, inline edit, create/edit/soft-delete, drafts, replies, `/preview/:id` through the public templates. 81 tests green |
| **M4** | PWA | ✅ **Done.** Generated icons, manifest, service worker, offline feed, update prompt, install prompt, share sheet. Offline verified against `pnpm run preview` with every server stopped |
| **M5** | Polish | ✅ **Done.** FTS5 search on both surfaces, hashtags with `/t/:tag` and `/tags`, `export`, `backup` + nightly timer, `rerender`, Cloudflare purge-on-write (off unless configured). 114 tests green |
| **M6** | The Familiar | ✅ **Done.** The whole state machine — topics, mood, energy decay and bursts, learned circadian phase, growth stages, pose triggers — plus compositional kaomoji rendering, the character sheet, and the five-minute snapshot. On the feed and at `/familiar`. 178 tests green |
| **M7** | Mood as a field | ✅ **Done.** `posts.mood`, a picker in the composer, and `0003` backfilling the hashtags that used to carry it. Hashtags are ordinary tags again; keyword inference stays as the fallback for a post with nothing picked. 190 tests green |
| **M8** | A familiar worth coming back to | 🚧 **In progress.** [`familiar-design.md`](familiar-design.md) is the spec, rewritten against the code and no longer a dangling reference. `familiar::baseline` landed first: sittings instead of posts, quantiles instead of means, and a decay half-life derived from the writer's own gap distribution — which fixes a pet that read every bursty writer as an absent one. Then the composer: `GET /api/familiar`, `POST /api/familiar/draft`, and a pet above the box that changes as you type. Then speech — one line, picked as the least likely true thing about the archive, in the pet's own voice on all three surfaces, and silence on an ordinary day. Then sparks — the pet's first transient: milestones that last a window instead of a single post, and a visible welcome back from a real absence. Still to come, in order: traits, a diet-shift line, the chronicle, anticipation. 261 tests green |

The split changes the shape of the plan more than anything else: **M1 ships a complete,
finished artifact** — a public archive at `youwin.dev` that works and is done — rather
than half of a surface that needs auth before it means anything. Writing happens over SSH
and `INSERT` until M3, which is a perfectly good way to run a blog for a few weeks.

**Still outstanding, and not a code task:** none of this is on the server yet. Routine
deploys are automated end to end, but the one-time setup is not and cannot be — users and
directories, `activate-youwin` and its sudoers rule, the systemd units, the Caddy blocks,
the password hash, the CI deploy key, and the one that is easy to skip and silently inert,
the **Cloudflare cache rule**, without which M1's `s-maxage` header does nothing at all.
See [`deploy/README.md`](deploy/README.md). After the first M5 deploy, run
`youwin-server rerender` once so existing posts pick up hashtag links.

## Deliberately not in v1

Image uploads · link preview cards · multi-user or comments · federation
(ActivityPub) · scheduled posts · analytics
(Umami is already on the box if it's ever wanted) · likes or counters of any kind.
