# Deploying youwin.dev

One binary, two listeners on loopback, Caddy in front. Nothing here is
automated on first run — these are the steps for standing it up once.

## First-time setup on the server

**1. User, directories, ownership**

```bash
sudo useradd --system --home /srv/youwin --shell /usr/sbin/nologin youwin
sudo mkdir -p /srv/youwin/{src,bin} /var/lib/youwin /var/www/youwin/{public,write} /var/backups/youwin
sudo chown -R youwin:youwin /srv/youwin /var/lib/youwin
```

**2. The password hash**

`hash-password` reads the terminal without echoing and never takes the password
as an argument, so it stays out of `ps` and shell history.

```bash
sudo -u youwin /srv/youwin/bin/youwin-server hash-password | sudo tee /etc/youwin/secrets.env
sudo chmod 0600 /etc/youwin/secrets.env
```

The server refuses to start without `YOUWIN_PASSWORD_HASH`, and refuses again if
it is not an argon2id PHC string. A misconfigured deploy fails loudly rather
than serving a site whose login can never succeed.

**3. DNS**

Add an `A`/`AAAA` record for `write.youwin.dev` alongside the apex. Caddy issues
its certificate through the same Cloudflare DNS-01 flow; no extra config.

**4. systemd**

```bash
sudo cp deploy/youwin.service /etc/systemd/system/youwin.service
sudo systemctl daemon-reload
sudo systemctl enable --now youwin
```

**5. Caddy**

Append `deploy/Caddyfile.youwin.dev` to the server Caddyfile (or `import` it),
then:

```bash
sudo caddy validate --config /etc/caddy/Caddyfile && sudo systemctl reload caddy
```

**6. The Cloudflare cache rule — do not skip this**

The public site sends `Cache-Control: public, max-age=60, s-maxage=300`, but
**Cloudflare does not cache HTML by default**, so that header is inert until a
cache rule exists. Without it every request reaches the origin and the main
benefit of a cookieless, JS-free site is left on the table.

In the dashboard: Caching → Cache Rules → for `youwin.dev`, "Eligible for cache"
with "Respect origin TTL". Do **not** apply it to `write.youwin.dev`, which is
entirely authenticated.

**7. Backups**

```bash
sudo cp deploy/youwin-backup.service deploy/youwin-backup.timer /etc/systemd/system/
sudo chown youwin:youwin /var/backups/youwin
sudo systemctl daemon-reload
sudo systemctl enable --now youwin-backup.timer
sudo systemctl start youwin-backup.service   # take one now rather than waiting
```

**8. Cache purging on write — optional**

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

```bash
./deploy/deploy.sh
```

Builds both frontends locally, rsyncs, builds the binary on the server, installs
it atomically, restarts, and health-checks both listeners.

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

## Checking on it

```bash
systemctl status youwin
journalctl -u youwin -f
curl -fsS http://127.0.0.1:8080/health      # public listener + read pool
curl -fsS http://127.0.0.1:8081/api/health  # authoring listener + write pool
```

Both health checks touch the database, so a green response means the pool is
live and not merely that axum is listening.

## Rotating the password

```bash
sudo -u youwin /srv/youwin/bin/youwin-server hash-password | sudo tee /etc/youwin/secrets.env
sudo systemctl restart youwin
```

Existing sessions survive a password change — they are rows in `sessions`, not
derivations of the password. To end them too, use "log out everywhere" in the
app (M3), or:

```bash
sqlite3 /var/lib/youwin/youwin.db 'DELETE FROM sessions;'
```
