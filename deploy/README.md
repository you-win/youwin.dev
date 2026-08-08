# Deploying youwin.dev

One binary, two listeners on loopback, Caddy in front. Nothing here is
automated on first run — these are the steps for standing it up once.

## Where you run this from

The working copy lives on Windows (`C:\Users\theaz\dev\youwin.dev`); deploys go
out from **WSL2 Debian**, which sees the same tree at
`/mnt/c/Users/theaz/dev/youwin.dev`. Everything below assumes that split. Three
things about it are worth knowing before the first deploy, because each fails in
a way that does not look like its cause.

**The frontend build cannot run in WSL.** `web/node_modules` holds native
binaries for exactly one platform — `@rollup/rollup-win32-x64-msvc`,
`@tailwindcss/oxide-win32-x64-msvc`, `lightningcss-win32-x64-msvc` — and Windows
and Linux cannot share one directory of them. `pnpm` *does* appear on `PATH` in
WSL, because interop appends the Windows `PATH`, which makes this look like it
should work right up until a module resolution error. So the split is:

```powershell
# PowerShell, in the repo
cd web ; pnpm run build
```

```bash
# WSL2 Debian, in the repo
./deploy/deploy.sh youwin
```

`deploy.sh` detects that no *native* pnpm is present, uses what is already in
`web/dist`, and refuses to ship it if any source is newer — including the maud
templates under `crates/server/src/public/`, which Tailwind scans, so a `.rs`
change can change `public.css`. `YOUWIN_SKIP_BUILD=1` overrides that check.

If you would rather have one command, clone into the Linux filesystem
(`~/dev/youwin.dev`, not `/mnt/c`) and install pnpm in WSL. `deploy.sh` then
finds a native pnpm and builds the frontends itself.

**Every file under `/mnt/c` reports mode 0777.** `rsync -a` preserves that, so
the plain form would publish a world-writable source tree to the server.
`deploy.sh` uses `-rlptz --chmod=D755,F644` instead — `-p` *with* `--chmod`, so
modes are corrected on files already there from an earlier run and not only on
new ones.

The same fact breaks SSH keys: `ssh` refuses a private key that permissive, so a
key under `/mnt/c/Users/theaz/.ssh` cannot be used in place. WSL keeps its own
`~/.ssh` — put the key there at `0600`, or point `~/.ssh/config` at it.

**There is no `~/.ssh/config` in WSL yet**, so the default host alias `youwin`
will not resolve. Either pass `user@host` to the script or add:

```
Host youwin
  HostName server.example
  User youwin
```

WSL's own `systemd` is irrelevant here — every `systemctl` below runs on the
remote server over SSH.

## First-time setup

Steps are marked **[server]** (over SSH) or **[wsl]** (from the repo in WSL2).

The order matters in one place: the binary has to exist before you can generate
a password hash with it, and the unit has to exist before `deploy.sh` can
restart it. So the first deploy happens in the middle, and fails at its last
step — that is expected, and step 6 finishes the job.

**1. [server] User, directories, ownership**

```bash
sudo useradd --system --home /srv/youwin --shell /usr/sbin/nologin youwin
sudo mkdir -p /srv/youwin/{src,bin} /var/lib/youwin /var/www/youwin/{public,write} /var/backups/youwin /etc/youwin
sudo chown -R youwin:youwin /srv/youwin /var/lib/youwin /var/backups/youwin
```

**2. [wsl] First deploy — builds the binary, then fails at the restart**

```powershell
cd web ; pnpm run build          # PowerShell — see "Where you run this from"
```

```bash
./deploy/deploy.sh youwin        # WSL2
```

It will end with `Failed to restart youwin.service: Unit youwin.service not
found`. The binary and the assets are in place; the unit is not, yet.

**3. [server] The password hash**

`hash-password` reads the terminal without echoing and never takes the password
as an argument, so it stays out of `ps` and shell history.

```bash
sudo -u youwin /srv/youwin/bin/youwin-server hash-password | sudo tee /etc/youwin/secrets.env
sudo chmod 0600 /etc/youwin/secrets.env
```

The server refuses to start without `YOUWIN_PASSWORD_HASH`, and refuses again if
it is not an argon2id PHC string. A misconfigured deploy fails loudly rather
than serving a site whose login can never succeed.

> Piping into `hash-password` also works, which is how it gets scripted — but
> **not from PowerShell**, which prepends a UTF-8 BOM to piped input. The BOM is
> stripped on the way in for exactly that reason; the point is that the failure
> it used to cause was invisible. From `bash` there is nothing to think about.

**4. [server] DNS**

Add an `A`/`AAAA` record for `write.youwin.dev` alongside the apex. Caddy issues
its certificate through the same Cloudflare DNS-01 flow; no extra config.

**5. [server] systemd**

The tree synced in step 2 is at `/srv/youwin/src`, so the unit files are already
on the box:

```bash
sudo cp /srv/youwin/src/deploy/youwin.service /etc/systemd/system/youwin.service
sudo systemctl daemon-reload
sudo systemctl enable --now youwin
```

**6. [server] Caddy**

Append `/srv/youwin/src/deploy/Caddyfile.youwin.dev` to the server Caddyfile (or
`import` it), then:

```bash
sudo caddy validate --config /etc/caddy/Caddyfile && sudo systemctl reload caddy
```

**7. [dashboard] The Cloudflare cache rule — do not skip this**

The public site sends `Cache-Control: public, max-age=60, s-maxage=300`, but
**Cloudflare does not cache HTML by default**, so that header is inert until a
cache rule exists. Without it every request reaches the origin and the main
benefit of a cookieless, JS-free site is left on the table.

In the dashboard: Caching → Cache Rules → for `youwin.dev`, "Eligible for cache"
with "Respect origin TTL". Do **not** apply it to `write.youwin.dev`, which is
entirely authenticated.

**8. [server] Backups**

```bash
sudo cp /srv/youwin/src/deploy/youwin-backup.{service,timer} /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now youwin-backup.timer
sudo systemctl start youwin-backup.service   # take one now rather than waiting
sudo systemctl status youwin-backup.service  # oneshot: confirm it exited 0
```

**9. [server] Cache purging on write — optional**

Skip this and the site runs on the `s-maxage` TTL alone, which is correct: an
edit takes up to five minutes to appear. To close that gap, create a **second**
Cloudflare API token with the `Cache Purge` permission — not the DNS-01 token
Caddy uses, which is scoped to DNS and should stay that way — then:

```bash
echo 'YOUWIN_CF_PURGE_TOKEN=...' | sudo tee -a /etc/youwin/secrets.env
sudo sed -i 's|^#Environment=YOUWIN_CF_ZONE_ID=.*|Environment=YOUWIN_CF_ZONE_ID=<zone id>|' /etc/systemd/system/youwin.service
sudo systemctl daemon-reload && sudo systemctl restart youwin
journalctl -u youwin -n 20 | grep 'edge cache'   # expect: cache_purging="on"
```

Every write then purges the whole zone. A failed purge is logged and never fails
the write.

## Routine deploys

```powershell
cd web ; pnpm run build          # PowerShell
```

```bash
./deploy/deploy.sh               # WSL2
```

Rsyncs the tree, builds the binary on the server, installs it atomically,
restarts, and health-checks both listeners. It refuses to run if `web/dist` is
older than any source it was built from, so a forgotten `pnpm run build` is an
error rather than a deploy that silently ships last week's stylesheet.

Only touched the Rust side? The build is still checked, and still has to be
current — but nothing will have changed it, so the first command is a no-op.

### After deploying a change to the renderer

`body_html` is a cache of what the renderer made of `body`, so posts written
before a renderer change keep their old HTML. **M5 is exactly this case**:
hashtags in existing posts are neither linked nor indexed until you run

```bash
sudo -u youwin /srv/youwin/bin/youwin-server rerender
```

It is idempotent, safe to run at any time, and does not mark anything as edited.

## Backups

WAL means `cp` of the `.db` file is **not** a valid backup — it can capture a
torn state with the committed data still sitting in the `-wal` sidecar. The
nightly timer installed above runs two things instead:

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
leaving them next to a different file is how you corrupt the one you just
restored.

### Pulling a copy down

A backup that only exists on the same disk as the thing it backs up is not one.
From WSL:

```bash
rsync -avz youwin:/var/backups/youwin/ /mnt/c/Users/theaz/backups/youwin/
```

`-a` is fine in this direction: the destination is DrvFs, which ignores modes
anyway, and nothing here is going to be executed.

## Checking on it — [server]

```bash
systemctl status youwin
journalctl -u youwin -f
curl -fsS http://127.0.0.1:8080/health      # public listener + read pool
curl -fsS http://127.0.0.1:8081/api/health  # authoring listener + write pool
```

Both health checks touch the database, so a green response means the pool is
live and not merely that axum is listening.

## Rotating the password — [server]

```bash
sudo -u youwin /srv/youwin/bin/youwin-server hash-password | sudo tee /etc/youwin/secrets.env
sudo systemctl restart youwin
```

Existing sessions survive a password change — they are rows in `sessions`, not
derivations of the password. To end them too, use "log out everywhere" in the
app, which needs nothing installed. Failing that:

```bash
sqlite3 /var/lib/youwin/youwin.db 'DELETE FROM sessions;'
```

That is the only step in this document that wants the `sqlite3` CLI, which is
otherwise deliberately not a dependency — the server needs a Rust toolchain and
a C compiler, and nothing else.
