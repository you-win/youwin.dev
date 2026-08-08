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
│  ├─ migrations/             # 0001_init.sql — embedded by sqlx::migrate!(), no CLI
│  ├─ tests/                  # pools.rs, posts.rs — one per statement in db/
│  └─ src/
│     ├─ lib.rs               # the crate proper — a lib so tests can reach the modules
│     ├─ main.rs              # thin binary: two listeners, graceful shutdown, `seed`
│     ├─ config.rs            # env → Config, fail fast on missing secrets
│     ├─ seed.rs              # `youwin-server seed` — dev rows via the real pipeline
│     ├─ db/{mod,posts}.rs    # pools + every statement  (+ sessions.rs at M2)
│     ├─ auth/{mod,password,session,middleware}.rs        (M2)
│     ├─ public/              # youwin.dev :8080
│     │  ├─ mod.rs            # Router — read pool only
│     │  ├─ assets.rs         # Vite manifest → hashed stylesheet URL, read at boot
│     │  ├─ routes.rs
│     │  └─ view/             # maud: layout, pages, post, atom, time_fmt
│     ├─ write/               # write.youwin.dev :8081
│     │  ├─ mod.rs            # Router — read + write pools
│     │  └─ routes/{auth,posts,drafts,preview}.rs         (M2/M3)
│     ├─ render/markdown.rs
│     └─ error.rs             # AppError → IntoResponse
├─ web/                       # pnpm; builds BOTH stylesheets and the SPA
│  ├─ vite.config.ts
│  └─ src/
│     ├─ theme.css            # mistwood tokens — single source of truth, imported by both
│     ├─ public.css           # tailwind + theme, no DaisyUI → the public site
│     ├─ app.css              # tailwind + DaisyUI + theme → the SPA
│     ├─ lib/api.ts
│     ├─ routes/{feed,permalink,login,drafts,settings}.tsx
│     └─ components/{Composer,PostCard,Thread,Feed}.tsx
└─ deploy/
   ├─ Caddyfile.youwin.dev    # both blocks
   ├─ youwin.service
   └─ deploy.sh
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

**Deferred to a later milestone**, but the schema is ready for both:
FTS5 over `body_text` with an external-content table plus insert/update/delete triggers,
and hashtag extraction into a `tags`/`post_tags` pair at render time.

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
GET  /about
GET  /feed.xml            Atom, public roots
```

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

**Edge caching.** Cookieless and JS-free means Cloudflare can cache the whole surface.
`Cache-Control: public, max-age=60, s-maxage=300` plus a Cloudflare cache rule (CF does not
cache HTML by default). The cost is that an edit or delete takes up to five minutes to
appear. If that grates, a purge-by-URL call on write is ~20 lines — but it needs a second
CF API token with cache-purge scope, since the DNS-01 token is scoped to DNS. Start with
the TTL.

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
POST   /api/posts         {body, parent_public_id?, visibility}
PATCH  /api/posts/:id     {body?, visibility?}
DELETE /api/posts/:id                 sets deleted_at

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
m=19456 KiB, t=2, p=1. A `youwin-server hash-password` subcommand generates the hash so
the plaintext never has to leave the terminal.

**Session.** 32 bytes from `OsRng`, base64url-encoded → the cookie value. `SHA-256` of
that value is the primary key in `sessions`. 90-day expiry, refreshed lazily: if
`last_seen_at` is more than a day old, bump both it and `expires_at`. So a phone that
opens the app weekly never gets logged out, while a truly idle session dies on schedule.

**Cookie.** `__Host-yw_session`, `HttpOnly; Secure; SameSite=Lax; Path=/`, set only by
`write.youwin.dev`. The `__Host-` prefix forbids a `Domain` attribute, so the cookie is
bound to that exact host and **cannot** be sent to `youwin.dev`. The public site is
therefore anonymous by construction: "did a draft leak because a cookie was present?" is
not a reachable bug, and it's also what makes the public surface edge-cacheable.

**CSRF** needs no token. Every state-changing route is `POST`/`PATCH`/`DELETE`, and
`SameSite=Lax` withholds the cookie from cross-site requests on those methods. As
defense in depth, a middleware rejects non-`GET` requests whose `Origin` isn't
`https://write.youwin.dev`.

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
`base-100`, 192/512 icons plus a maskable variant.

Because **every route on this origin is authenticated**, the usual rules collapse: there
is no public content to accidentally cache and no logout-purge dance to get right. What's
left:

- **`navigateFallbackDenylist: [/^\/api\//]`.** Without it the SW answers API calls with
  the HTML shell and every fetch fails at `JSON.parse`.
- `NetworkFirst` for `GET /api/feed` and `GET /api/posts/*` — the last-seen feed stays
  readable in a tunnel. `CacheFirst` for hashed assets. Non-`GET` never touches the SW.
- On logout, `caches.delete()` the API cache anyway. Cheap, and it means a shared or
  stolen device doesn't show the last feed.
- `sw.js` and `manifest.webmanifest` are served `no-cache` (see the Caddy block), or a
  stale SW pins an old build indefinitely.

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

**Caddy** — `deploy/Caddyfile.youwin.dev`, following the house style in
`grindshell/server-configs/static/Caddyfile` (Cloudflare DNS-01, loopback backends,
two-tier cache, baseline security headers).

```caddyfile
# The public archive. No JS, no cookies, no API — so the whole surface is
# edge-cacheable and the CSP can forbid script outright.
youwin.dev, www.youwin.dev {
        tls {
                dns cloudflare {env.CF_API_TOKEN}
        }

        encode zstd gzip

        # The one stylesheet (content-hashed) plus favicons and robots.txt.
        # Served off disk; the app never touches CSS bytes.
        handle /assets/* {
                root * /var/www/youwin/public
                header Cache-Control "public, max-age=31536000, immutable"
                file_server {
                        precompressed br gzip
                }
        }
        handle /favicon.ico /robots.txt {
                root * /var/www/youwin/public
                file_server
        }

        # Everything else is server-rendered HTML. max-age is for the browser,
        # s-maxage for Cloudflare — which also needs a cache rule, since CF does
        # not cache HTML by default.
        reverse_proxy 127.0.0.1:8080
        header Cache-Control "public, max-age=60, s-maxage=300"

        header {
                X-Content-Type-Options "nosniff"
                Referrer-Policy "strict-origin-when-cross-origin"
                X-Frame-Options "SAMEORIGIN"
                Strict-Transport-Security "max-age=31536000; includeSubDomains"
                Content-Security-Policy "default-src 'none'; style-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
                -Server
        }
}

# The authoring app. Session cookie lives here and ONLY here.
write.youwin.dev {
        tls {
                dns cloudflare {env.CF_API_TOKEN}
        }

        encode zstd gzip

        # JSON API and the authenticated /preview route. Listed first so it wins
        # over the SPA fallback below.
        handle /api/* {
                reverse_proxy 127.0.0.1:8081
        }
        handle /preview/* {
                reverse_proxy 127.0.0.1:8081
        }

        # Content-hashed SPA build output → immutable.
        handle /assets/* {
                root * /var/www/youwin/write
                header Cache-Control "public, max-age=31536000, immutable"
                file_server {
                        precompressed br gzip
                }
        }

        # PWA plumbing: on disk, but MUST revalidate. A cached sw.js pins an old
        # build on every installed device until it expires.
        handle /sw.js /manifest.webmanifest /icons/* {
                root * /var/www/youwin/write
                header Cache-Control "no-cache"
                file_server
        }

        # Client-routed SPA: serve the file if it exists, else the shell.
        handle {
                root * /var/www/youwin/write
                try_files {path} /index.html
                file_server
                header Cache-Control "no-cache"
        }

        # Nothing here is ever cached at the edge — it is all authenticated.
        header {
                X-Content-Type-Options "nosniff"
                Referrer-Policy "strict-origin-when-cross-origin"
                X-Frame-Options "DENY"
                Strict-Transport-Security "max-age=31536000; includeSubDomains"
                Content-Security-Policy "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"
                -Server
        }
}
```

Two things worth noting against the existing blocks. The public CSP is
`default-src 'none'` with only `style-src` and `img-src` opened up — a genuinely no-JS
site can assert that, and it's the strongest policy on the box. And `write.youwin.dev`
takes `X-Frame-Options: DENY` rather than `SAMEORIGIN`, because nothing should ever frame
the composer.

**systemd** — one unit, `Type=exec`, `Restart=on-failure`,
`EnvironmentFile=/etc/youwin/secrets.env` (mode 0600, holds `YOUWIN_PASSWORD_HASH`).
Hardened: `NoNewPrivileges`, `ProtectSystem=strict` with `ReadWritePaths=/var/lib/youwin`,
`ProtectHome`, `RestrictAddressFamilies=AF_INET AF_INET6`, `MemoryDenyWriteExecute`.
Graceful shutdown on SIGTERM — both listeners, so WAL checkpoints cleanly on restart.

**DNS.** One new record for `write.youwin.dev`. DNS-01 issuance works identically; Caddy
handles the per-host certificate automatically.

**Build.** You're on Windows and the target is Linux; `sqlx`'s `sqlite` feature bundles
SQLite through `libsqlite3-sys`, so cross-compiling means a cross C toolchain. Skip it.
`deploy.sh` builds both frontends locally with pnpm (`public.css` + the SPA), rsyncs the
tree, runs `cargo build --release` on the server, installs the binary, and restarts the
unit. Nothing beyond a Rust toolchain and a C compiler is needed there — no database at
build time, no `cargo sqlx`.

**Backups.** WAL means `cp` of the `.db` file is not a valid backup. A systemd timer runs
`sqlite3 youwin.db ".backup /var/backups/youwin/$(date +%F).db"` nightly. Separately,
`youwin-server export` dumps every post as JSON + markdown files — the actual insurance
policy, since it survives SQLite itself.

## Milestones

| | | |
|---|---|---|
| **M0** | Skeleton | ✅ **Done.** Zola tree deleted. Workspace, config, the two pools, `sqlx::migrate!()` on boot (schema landed here — `migrate!()` wants a real migration), two listeners with health checks, `tests/pools.rs`. `theme.css` split into `public.css` + `app.css` |
| **M1** | **The entire public site** | ✅ **Done.** `youwin-server seed`, maud templates, feed + cursor pagination, permalinks, threads, Atom, markdown pipeline, asset-manifest lookup, themed 404. 26 tests green, covering every statement in `db/`. *Still to do before it is actually live: put the Caddy block and the Cloudflare cache rule on the box, alongside M2.* |
| **M2** | Auth | `hash-password`, login, sessions, guard middleware, rate limiter, Origin check, the `write.youwin.dev` Caddy block |
| **M3** | Authoring app | Solid SPA, composer, create/edit/soft-delete, drafts, replies, `/preview/:id` |
| **M4** | PWA | Manifest, service worker, offline feed, install prompt, share sheet |
| **M5** | Polish | FTS5 search, hashtags, `export`, backup timer, optional cache purge on write |

The split changes the shape of the plan more than anything else: **M1 ships a complete,
finished artifact** — a public archive at `youwin.dev` that works and is done — rather
than half of a surface that needs auth before it means anything. Writing happens over SSH
and `INSERT` until M3, which is a perfectly good way to run a blog for a few weeks.

## Deliberately not in v1

Image uploads · link preview cards · multi-user or comments · federation
(ActivityPub) · full-text search (M5) · scheduled posts · analytics
(Umami is already on the box if it's ever wanted) · likes or counters of any kind.
