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

## Routine deploys

```bash
./deploy/deploy.sh
```

Builds both frontends locally, rsyncs, builds the binary on the server, installs
it atomically, restarts, and health-checks both listeners.

## Backups

WAL means `cp` of the `.db` file is **not** a valid backup — it can capture a
torn state with the committed data still sitting in the `-wal` sidecar.

```bash
sqlite3 /var/lib/youwin/youwin.db ".backup /var/backups/youwin/$(date +%F).db"
```

Put that on a nightly systemd timer. `youwin-server export` (M5) is the real
insurance policy, since a directory of markdown outlives SQLite itself.

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
