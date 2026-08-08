# Deploying youwin.dev

Routine deploys run entirely from GitHub Actions: push to `master`, and CI
builds, tests, ships, activates, health-checks, and rolls back if the site does
not come up. Nothing needs to happen on your machine.

Getting to that point does not. This document is mostly the one-time setup.

## What CI can and cannot do

**Entirely CI, after setup:** frontend build, `cargo test`, release build,
upload, symlink flip, restart, health check, automatic rollback, pruning old
releases. There is no build step on the server and no Rust toolchain on it —
the binary arrives prebuilt.

**Never CI, by design:**

| | |
|---|---|
| First-time provisioning | Needs root. One-time, and worth doing by hand where you can see it. |
| `hash-password` | The site's only credential. It must not exist in a CI secret, a log, or a workflow file. |
| DNS and the Cloudflare cache rule | Dashboard, one-time. |
| The Cloudflare purge token | Lives in `/etc/youwin/secrets.env` on the server. CI never sees it. |
| `rerender` after a renderer change | Idempotent and safe, but *when* to run it is a judgement call about your data, not a build step. One SSH command — see below. |

The privilege boundary is one line of sudoers: the `deploy` user may run
`/usr/local/bin/activate-youwin` as root and nothing else. CI can ship a release
and ask for it to be activated. It cannot do anything else on the box.

## Layout on the server

```
/srv/youwin/
├─ releases/20260808-143000-a1b2c3d/
│  ├─ bin/youwin-server      built in CI; no toolchain on the server
│  ├─ public/                hashed CSS, favicons, robots.txt
│  ├─ write/                 the SPA, sw.js, manifest
│  └─ deploy/                unit files and this document, travelling with the release
├─ current  -> releases/20260808-143000-a1b2c3d
└─ previous -> releases/20260807-101500-9f8e7d6

/var/lib/youwin/youwin.db    never touched by a deploy
/var/backups/youwin/         nightly backup + export
/etc/youwin/secrets.env      password hash, and the purge token if used
```

`current` is what the systemd unit and the Caddy roots point at, so the binary
and the assets it serves change together or not at all. The database lives
outside all of it and no deploy goes near it.

## First-time setup

Steps are marked **[server]** (as your admin user, over SSH), **[local]**, or
**[dashboard]**.

**1. [server] Users and directories**

Assumes the `deploy` user already exists from the `timothyyuen.io` setup; if
not, create it the same way — no password, no sudo, one SSH key.

```bash
sudo useradd --system --home /srv/youwin --shell /usr/sbin/nologin youwin
sudo install -d -m 755 -o root   -g root   /srv/youwin /etc/youwin
sudo install -d -m 755 -o deploy -g deploy /srv/youwin/releases
sudo install -d -m 750 -o youwin -g youwin /var/lib/youwin /var/backups/youwin
```

`releases/` is owned by `deploy` — that is the only thing CI can write to.
`/srv/youwin` itself is root-owned, so the `current` symlink can only be moved
by the activation script.

**2. [server] The activation script and its sudoers rule**

Get the repo onto the box however you like (`git clone`, or scp the `deploy/`
directory); after the first deploy it also lives at `/srv/youwin/current/deploy`.

```bash
sudo install -m 755 -o root -g root deploy/activate-youwin /usr/local/bin/activate-youwin
```

Validate the sudoers fragment **before** installing it — a syntax error anywhere
under `/etc/sudoers.d` breaks `sudo` entirely, and by then the bad file is live:

```bash
echo 'deploy ALL=(root) NOPASSWD: /usr/local/bin/activate-youwin' > /tmp/youwin-sudoers
sudo visudo -cf /tmp/youwin-sudoers \
  && sudo install -m 440 -o root -g root /tmp/youwin-sudoers /etc/sudoers.d/youwin
rm /tmp/youwin-sudoers
```

**3. [server] systemd**

```bash
sudo install -m 644 deploy/youwin.service /etc/systemd/system/youwin.service
sudo install -m 644 deploy/youwin-backup.service /etc/systemd/system/
sudo install -m 644 deploy/youwin-backup.timer /etc/systemd/system/
sudo systemctl daemon-reload
```

Do **not** enable `youwin` yet — there is no binary and no password hash. Step 6.

**4. [dashboard] DNS**

Add an `A`/`AAAA` record for `write.youwin.dev` alongside the apex. Caddy issues
its certificate through the same Cloudflare DNS-01 flow; no extra config.

**5. [server] Caddy**

Append `deploy/Caddyfile.youwin.dev` to the server Caddyfile (or `import` it):

```bash
sudo caddy validate --config /etc/caddy/Caddyfile && sudo systemctl reload caddy
```

**6. [local, then server] First release and the password**

CI cannot do this one, because the password hash has to exist before the service
can start and the binary has to exist before you can generate a hash with it.
From WSL2 Debian, which has cargo and can build the Linux binary directly:

```bash
DEPLOY_HOST=server.example ./deploy/deploy.sh
```

It will upload a release and then fail at activation, because `youwin.service`
cannot start without `YOUWIN_PASSWORD_HASH`. That is expected. Now, on the
server:

```bash
sudo -u youwin /srv/youwin/current/bin/youwin-server hash-password \
  | sudo tee /etc/youwin/secrets.env
sudo chmod 0600 /etc/youwin/secrets.env
sudo systemctl enable --now youwin
```

`hash-password` reads the terminal without echoing and never takes the password
as an argument, so it stays out of `ps` and shell history. The server refuses to
start without the hash, and refuses again if it is not an argon2id PHC string —
a misconfigured deploy fails loudly rather than serving a site whose login can
never succeed.

> Piping into `hash-password` works too, and is how it gets scripted — but not
> from PowerShell, which prepends a UTF-8 BOM to piped input. That BOM is
> stripped on the way in for exactly this reason; the point is that the failure
> it used to cause was invisible.

**7. [dashboard] The Cloudflare cache rule — do not skip this**

The public site sends `Cache-Control: public, max-age=60, s-maxage=300`, but
**Cloudflare does not cache HTML by default**, so that header is inert until a
cache rule exists. Without it every request reaches the origin and the main
benefit of a cookieless, JS-free site is left on the table.

Caching → Cache Rules → for `youwin.dev`, "Eligible for cache" with "Respect
origin TTL". Do **not** apply it to `write.youwin.dev`, which is entirely
authenticated.

**8. [server] Backups**

```bash
sudo systemctl enable --now youwin-backup.timer
sudo systemctl start youwin-backup.service   # take one now rather than waiting
sudo systemctl status youwin-backup.service  # oneshot: confirm it exited 0
```

**9. [local + GitHub] Hand over to CI**

Generate a deploy key *for CI only* — it is not your admin key, and it goes on
the `deploy` user, which cannot log in interactively.

```bash
ssh-keygen -t ed25519 -N '' -C 'github-actions youwin.dev' -f ~/.ssh/youwin-ci
ssh-keyscan -t ed25519 server.example    # for DEPLOY_KNOWN_HOSTS
```

On the server:

```bash
sudo tee -a /home/deploy/.ssh/authorized_keys < ~/.ssh/youwin-ci.pub
```

In the repo's **Settings → Secrets and variables → Actions**:

| Secret | Value |
|---|---|
| `DEPLOY_SSH_KEY` | the whole of `~/.ssh/youwin-ci` (private half) |
| `DEPLOY_KNOWN_HOSTS` | the `ssh-keyscan` output |
| `DEPLOY_HOST` | `server.example` |

Then push, or run the workflow by hand from the Actions tab.

**10. [optional] Cache purging on write**

Skip this and the site runs on the `s-maxage` TTL alone, which is correct: an
edit takes up to five minutes to appear. To close that gap, create a **second**
Cloudflare API token with the `Cache Purge` permission — not the DNS-01 token
Caddy uses, which is scoped to DNS and should stay that way:

```bash
echo 'YOUWIN_CF_PURGE_TOKEN=...' | sudo tee -a /etc/youwin/secrets.env
sudo sed -i 's|^#Environment=YOUWIN_CF_ZONE_ID=.*|Environment=YOUWIN_CF_ZONE_ID=<zone id>|' /etc/systemd/system/youwin.service
sudo systemctl daemon-reload && sudo systemctl restart youwin
journalctl -u youwin -n 20 | grep 'edge cache'   # expect: cache_purging="on"
```

## Routine deploys

Push to `master`. That is the whole procedure.

The workflow is [`.github/workflows/deploy.yml`](../.github/workflows/deploy.yml):
`pnpm install` → typecheck → build → check the expected build outputs exist →
`cargo test` → `cargo build --release` → assemble a release directory → rsync it
→ `activate-youwin`.

Things in there that are load-bearing:

- **`runs-on: ubuntu-24.04`, pinned.** The glibc CI builds against becomes the
  floor for the server. 24.04 is glibc 2.39 and Debian 13 has 2.41, so the
  binary loads. A server *older* than the runner could not run it — which is
  what the smoke test below exists to catch. If that ever happens, build in a
  matching container instead (`container: rust:1-trixie`).
- **`--locked` on both cargo commands.** The deploy builds exactly what
  `Cargo.lock` says or it fails; quietly resolving a different dependency set
  during a deploy is something you find out about much later.
- **`activate-youwin` smoke-tests before it stops anything.**
  `youwin-server version` touches no config and no database, so the only thing
  it can fail on is the dynamic loader — the one failure that would otherwise
  take the site down *after* the old process had already been stopped.
- **It health-checks both listeners and rolls back on failure.** Both checks
  touch the database, so passing means more than "the socket is open".

### Rolling back

```bash
sudo /usr/local/bin/activate-youwin --rollback     # to the previous release
ls /srv/youwin/releases                            # or pick one
sudo /usr/local/bin/activate-youwin 20260807-101500-9f8e7d6
```

Five releases are kept. Whatever `current` and `previous` point at is never
pruned regardless of that limit.

### After deploying a change to the renderer

`body_html` is a cache of what the renderer made of `body`, which is the
authority — so posts written before a renderer change keep their old HTML.
**M5 is exactly this case**: hashtags in existing posts are neither linked nor
indexed until you run

```bash
sudo -u youwin /srv/youwin/current/bin/youwin-server rerender
```

Idempotent, safe at any time, and it does not mark anything as edited.

### Manual deploy

For the first release, or when you need to ship without pushing. Requires a
Linux host with `cargo` — WSL2 Debian qualifies.

```powershell
cd web ; pnpm run build          # PowerShell: see "The Windows/WSL split"
```

```bash
DEPLOY_HOST=server.example ./deploy/deploy.sh
```

It assembles byte-for-byte the same release layout as CI and calls the same
`activate-youwin`.

## The Windows/WSL split

The working copy is on Windows; anything Linux-shaped runs in WSL2 Debian, which
sees the same tree at `/mnt/c/Users/theaz/dev/youwin.dev`. Only the manual path
cares, but when it does, these bite:

**The frontend build cannot run in WSL.** `web/node_modules` holds native
binaries for exactly one platform — `@rollup/rollup-win32-x64-msvc`,
`@tailwindcss/oxide-win32-x64-msvc`, `lightningcss-win32-x64-msvc` — and Windows
and Linux cannot share one directory of them. `pnpm` *does* appear on `PATH` in
WSL, because interop appends the Windows `PATH`, which makes this look like it
should work right up until a module resolution error. `deploy.sh` detects that
the pnpm it found lives under `/mnt/` and uses `web/dist` as built on Windows,
refusing to ship it if any source is newer — including
`crates/server/src/public/**/*.rs`, which Tailwind scans.

CI has none of this problem: it installs Linux `node_modules` from scratch.

**Every file under `/mnt/c` reports mode 0777.** `deploy.sh` builds its release
in a temp directory with explicit `chmod`s rather than rsyncing modes across.
The same fact means `ssh` refuses a private key stored under `/mnt/c` — WSL
keeps its own `~/.ssh`.

**`CARGO_TARGET_DIR` is redirected out of the tree.** A Windows `cargo build`
and a WSL one cannot share `./target`; each invalidates the other's artifacts,
turning every platform switch into a full rebuild.

## Checking on it — [server]

```bash
systemctl status youwin
journalctl -u youwin -f
readlink -f /srv/youwin/current                 # which release is live
/srv/youwin/current/bin/youwin-server version   # which commit it was built from
curl -fsS http://127.0.0.1:8080/health          # public listener + read pool
curl -fsS http://127.0.0.1:8081/api/health      # authoring listener + write pool
```

Both health checks touch the database, so a green response means the pool is
live and not merely that axum is listening.

## Backups

WAL means `cp` of the `.db` file is **not** a valid backup — it can capture a
torn state with the committed data still sitting in the `-wal` sidecar. The
nightly timer runs two things instead:

```bash
youwin-server backup /var/backups/youwin        # VACUUM INTO — a consistent .db
youwin-server export /var/backups/youwin/export # posts.json + a markdown tree
```

`backup` uses SQLite's `VACUUM INTO`, which reads through one consistent
snapshot of a live WAL database, so nothing has to be stopped. It keeps the last
30 dated files and removes nothing else — only names matching exactly
`youwin-YYYY-MM-DD.db` are ever candidates for deletion.

`export` is the one that outlives everything: a directory of markdown with front
matter is readable with no SQLite, no Rust toolchain, and no memory of how any of
this worked. `posts.json` alongside it is complete enough to rebuild the
database, deletions included.

To restore, stop the service and put a backup in place:

```bash
sudo systemctl stop youwin
sudo -u youwin cp /var/backups/youwin/youwin-2026-08-08.db /var/lib/youwin/youwin.db
sudo -u youwin rm -f /var/lib/youwin/youwin.db-wal /var/lib/youwin/youwin.db-shm
sudo systemctl start youwin
```

Removing the sidecars matters: they belong to the database you replaced, and
leaving them beside a different file is how you corrupt the one you just
restored.

### Pulling a copy down

A backup that only exists on the same disk as the thing it backs up is not one.
From WSL:

```bash
rsync -avz youwin-admin@server.example:/var/backups/youwin/ /mnt/c/Users/theaz/backups/youwin/
```

## Rotating the password — [server]

```bash
sudo -u youwin /srv/youwin/current/bin/youwin-server hash-password \
  | sudo tee /etc/youwin/secrets.env
sudo systemctl restart youwin
```

Existing sessions survive a password change — they are rows in `sessions`, not
derivations of the password. To end them too, use "log out everywhere" in the
app, which needs nothing installed. Failing that:

```bash
sqlite3 /var/lib/youwin/youwin.db 'DELETE FROM sessions;'
```

That is the only step here that wants the `sqlite3` CLI, which is otherwise
deliberately not a dependency — with the binary built in CI, the server needs no
build toolchain at all.
