# The off-site backup receiver

`youwin-offsite` is the far end of the nightly `PUT`. It runs on a **different
box** from youwin.dev — one that already has an unrelated service on it — and its
whole job is to receive two files a night, prove they are usable, and keep the
last ninety of each.

The sending half is documented in [the main deploy README](../README.md#off-site-copies)
and needs no changes to talk to this: it already `PUT`s to a URL with an
`Authorization` header, and a Storage Box or an nginx with `dav_methods` would
answer it just as well. What none of those can do is the reason this exists.

**It opens the database before it accepts it.** A file server stores 40MB and
returns 201. It cannot tell a `VACUUM INTO` snapshot from 40MB of zeroes, and
neither can the sender — a successful `PUT` is the only signal it has. Every
arriving `.db` goes through `PRAGMA integrity_check` and every `.json` is parsed
*before* it is renamed into place. A file that fails is deleted, the request is
refused with 422, and the sending box's `youwin-backup.service` goes red. The
night a backup goes bad is the night you find out.

## What is deliberately not automated

**There is no deploy pipeline for this box.** The site ships on every push
because it changes constantly; this changes maybe twice a year, and a pipeline
that runs twice a year is one that has quietly rotted by the time you need it —
which is the same reasoning that deleted the local deploy script from the other
box. The trade is real and it is stated here so it stays a decision: **installing
an update is a thing you must remember to do.**

It also keeps the blast radius honest. A deploy user with a sudoers rule on a box
running somebody else's service is a bigger thing to own than the twenty minutes
a year it would save.

CI still *builds* it — **Actions → offsite → Run workflow** — with the same glibc
assertion the site's binary gets. What you download is the binary plus this
directory, so the unit file and the Caddy block always match it.

**There is no health endpoint.** The Caddy block aborts every method but `PUT`,
so there is nothing to curl. That is not an oversight to work around: the honest
status check is `ls -lt` on the backup directory, and the real alarm lives on the
*other* box — see [Knowing it still works](#knowing-it-still-works).

## Layout on the box

```
/usr/local/bin/youwin-offsite                    the binary, installed by hand
/etc/youwin-offsite/secrets.env                  YOUWIN_OFFSITE_AUTH — 0600
/etc/systemd/system/youwin-offsite.service
/etc/caddy/conf.d/backup.youwin.dev.caddy
/var/backups/youwin.dev/                         0750 youwin-offsite:youwin-offsite
├─ youwin-2026-08-13.db                          the snapshot — what you restore
└─ youwin-2026-08-13.json                        the export — readable with no SQLite
/var/log/caddy/backup.access.log
```

One flat directory of dated files, no per-day nesting, because `ls -lt` answering
"did last night work?" at a glance is the only status interface this service has.

## First-time setup

Steps are marked **[server]** (the *receiving* box, over SSH), **[dashboard]**,
**[local]**, or **[sending box]** (the youwin.dev server).

**1. [local] Get the binary**

**Actions → offsite → Run workflow**, then download the artifact. It contains
`youwin-offsite` and a `deploy/` copy of this directory.

Building it yourself works too, on any Linux host with a Rust toolchain — but
mind the glibc floor the workflow asserts for you:

```bash
cargo build --release --locked --package youwin-offsite
```

**2. [dashboard] DNS — and leave the cloud grey**

An `A`/`AAAA` record for `backup.youwin.dev` pointing at the **receiving** box,
**DNS-only (grey cloud)**.

> **Do not turn on the orange cloud for this hostname.** Two independent reasons,
> and the first is the one that bites silently:
>
> - **Cloudflare caps request bodies at 100MB on the free plan.** Proxy this name
>   and every upload over 100MB is refused at the edge with a 413 that never
>   reaches this box. The archive is smaller than that today. The failure arrives
>   on the night it is not, which is the night you least want a novel error.
> - **TLS-ALPN cannot work behind the proxy**, because Cloudflare terminates TLS
>   at the edge. The Caddy block here uses HTTP-01/TLS-ALPN with the *stock*
>   Caddy binary — unlike youwin.dev, which needs the `caddy-dns/cloudflare`
>   module. Keeping this grey is what lets this box stay on stock Caddy.
>
> If you ever do want it proxied, the fix is to add a `tls` block using DNS-01
> like `youwin.dev.caddy` does, and to accept the 100MB ceiling — or pay for a
> plan without it.

Note that `youwin.dev` sends `Strict-Transport-Security` with
`includeSubDomains`, which commits every browser that has seen the apex to
HTTPS-only on `backup.youwin.dev` too. Nothing here is a browser, so this is
harmless — but it does mean a half-working certificate on this name would be
invisible to `curl` and hard-failed in Chrome, so confirm TLS properly in step 7.

**3. [server] User and directories**

```bash
sudo useradd --system --home /nonexistent --shell /usr/sbin/nologin youwin-offsite
sudo install -d -m 755 -o root -g root /etc/youwin-offsite
sudo install -d -m 750 -o youwin-offsite -g youwin-offsite /var/backups/youwin.dev
```

A user of its own, not the one the box's other service runs as. It owns exactly
one directory and needs nothing else.

**4. [server] Install the binary**

```bash
sudo install -m 755 -o root -g root youwin-offsite /usr/local/bin/youwin-offsite
/usr/local/bin/youwin-offsite version
```

`version` touches no config, no environment, and no filesystem — so the only
thing it can fail on is the dynamic loader, which is the failure worth catching
before the unit exists to hide it.

**5. [server] The credential**

One string, held identically by both boxes. Generate it here:

```bash
printf 'YOUWIN_OFFSITE_AUTH=Bearer %s\n' "$(openssl rand -base64 32)" \
  | sudo tee /etc/youwin-offsite/secrets.env
sudo chmod 0600 /etc/youwin-offsite/secrets.env
```

Keep that value — step 9 puts the same line, verbatim, on the sending box. It is
a **complete `Authorization` header value**, compared whole, so `Bearer`,
`Basic`, and anything else differ only in a prefix and there is no scheme setting
for the two ends to disagree about.

The service refuses to start without it. That is deliberate: this listens on a
name the world can resolve, and an unauthenticated one is a public drop box.

**6. [server] systemd**

```bash
sudo install -m 644 deploy/youwin-offsite.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now youwin-offsite
systemctl status youwin-offsite
```

Expect a `listening` line naming the directory, the retention, and `newest=none`.
That last field is the one to look at from here on; see
[Knowing it still works](#knowing-it-still-works).

**7. [server] Caddy**

First check whether the box's other site already defines a `(hardening)` snippet
— if it does, **delete the snippet from the top of the file before installing
it**. Caddy rejects a duplicate definition, and it rejects the *whole* config,
which takes the unrelated service down with it:

```bash
grep -rn '(hardening)' /etc/caddy/
```

Also confirm the main `Caddyfile` ends with `import /etc/caddy/conf.d/*.caddy`;
if this box arranges its sites differently, paste the block wherever they live.

```bash
sudo install -m 644 -o root -g root deploy/backup.youwin.dev.caddy /etc/caddy/conf.d/
sudo caddy validate --config /etc/caddy/Caddyfile
sudo chown -R caddy:caddy /var/log/caddy
sudo systemctl reload caddy
journalctl -u caddy -f          # watch the certificate get issued
```

`caddy validate` provisions the whole config, log writers included, so it creates
`/var/log/caddy/*.log` owned by whoever ran it — root. Caddy then cannot open its
own log and fails to start. The `chown` puts that right, and is worth running
over the whole directory once regardless.

**8. [server] Prove it, including the refusal**

The upload half is easy to believe without checking. The *refusal* half is the
reason this program exists, so test that one too.

```bash
AUTH="$(sudo grep -oP '(?<=^YOUWIN_OFFSITE_AUTH=).*' /etc/youwin-offsite/secrets.env)"
TODAY="$(date -u +%F)"
URL="https://backup.youwin.dev/youwin-$TODAY.db"

# 1 — no credential. Expect 401 and nothing on disk.
curl -sS -o /dev/null -w '%{http_code}\n' -X PUT --data-binary 'x' "$URL"

# 2 — the right credential, the wrong bytes. Expect 422 and a sentence saying so.
curl -sS -X PUT -H "Authorization: $AUTH" --data-binary 'not a database' "$URL"

# 3 — a name it will not write. Expect 400.
curl -sS -X PUT -H "Authorization: $AUTH" --data-binary 'x' \
  https://backup.youwin.dev/../../etc/passwd

# 4 — nothing should have landed from any of the above.
sudo ls -la /var/backups/youwin.dev
```

Step 2 is the acceptance test for the whole idea. If it returns 201, this is a
file server with extra steps and something is wrong.

**9. [sending box] Point the nightly run at it**

On the **youwin.dev server**, add the URL and the same credential from step 5 to
the file the backup unit already loads:

```bash
sudo tee -a /etc/youwin/secrets.env >/dev/null <<'EOF'
YOUWIN_OFFSITE_URL=https://backup.youwin.dev
YOUWIN_OFFSITE_AUTH=Bearer <the same string from step 5>
EOF

sudo systemctl start youwin-backup.service
journalctl -u youwin-backup.service -n 20    # expect two "Uploaded …" lines
```

> **No path on the end of that URL.** `https://backup.youwin.dev`, not
> `https://backup.youwin.dev/youwin`. The receiver answers at the root, and a
> trailing path segment gets a 400 whose body says exactly this — but on a
> machine you are not looking at, at an hour you did not choose.

Then confirm on the receiving box:

```bash
sudo ls -l /var/backups/youwin.dev
journalctl -u youwin-offsite -n 20
```

Expect two files and two `stored an off-site backup` lines carrying `bytes`,
`posts`, and `pruned`.

## Verifying the setup

Worth running once at the end, and again any time something behaves oddly.

```bash
# 1 — ownership. The service owns one directory and nothing else.
ls -ld /var/backups/youwin.dev /etc/youwin-offsite
ls -l /etc/youwin-offsite/secrets.env       # root:root 600
#   /var/backups/youwin.dev   youwin-offsite:youwin-offsite 750

# 2 — up, enabled, and listening on loopback only.
systemctl is-enabled youwin-offsite
systemctl is-active youwin-offsite
sudo ss -lntp | grep 8080                   # 127.0.0.1:8080, nothing else

# 3 — the hostname is grey. An answer with cf-ray means it is proxied, which
#     means a 100MB ceiling nobody chose. See step 2.
curl -sSI https://backup.youwin.dev -X PUT | grep -i 'cf-ray' && echo "PROXIED — fix this"

# 4 — everything but PUT is closed. `abort` drops the connection, so curl
#     reports a failure rather than a status code. That is the pass.
curl -sS https://backup.youwin.dev/ ; echo "exit=$?"

# 5 — what has actually arrived, newest last.
sudo ls -l --time-style=long-iso /var/backups/youwin.dev | tail -5

# 6 — nothing in deploy/ has drifted from what is installed. Silence is correct.
diff -q deploy/youwin-offsite.service /etc/systemd/system/youwin-offsite.service
diff -q deploy/backup.youwin.dev.caddy /etc/caddy/conf.d/backup.youwin.dev.caddy
/usr/local/bin/youwin-offsite version
```

## Knowing it still works

**Nothing on this box raises an alarm, and that is on purpose.** The alarm
already exists, on the other one.

The sending box's `youwin-backup.service` treats a failed upload as a failed
backup: it exits non-zero and the unit goes to `failed`. So every way this
receiver can let you down — down, out of disk, refusing a corrupt snapshot,
holding a credential you rotated on one side only — surfaces over there as a red
oneshot with the status code and this service's own response body in the journal.
Duplicating that into a second alarm on a second box would mean two things to
maintain and two places to be wrong about which one is authoritative.

What this box gives you instead is the evidence, in three places:

```bash
# The newest date it holds, printed at every start. Not yesterday's date after
# a restart means uploads stopped before the restart, not because of it.
systemctl status youwin-offsite | head -20

# Every arrival, with its size and post count. A snapshot that suddenly holds a
# tenth of the posts is the failure no status code shows.
journalctl -u youwin-offsite -g 'stored an off-site backup' -n 10

# Every refusal, with the reason.
journalctl -u youwin-offsite -p warning -n 20
```

The one thing genuinely worth doing by hand, once a quarter: take the newest
`.db` off this box and open it. A backup nobody has ever restored is a belief,
not a backup.

```bash
scp backup-box:/var/backups/youwin.dev/youwin-2026-08-13.db .
sqlite3 youwin-2026-08-13.db 'PRAGMA integrity_check; SELECT count(*) FROM posts;'
```

## Restoring from it

The receiver holds exactly what the sending box's `/var/backups/youwin` holds,
so [the restore steps in the main README](../README.md#backups) apply unchanged
once the file is back on the server:

```bash
scp backup-box:/var/backups/youwin.dev/youwin-2026-08-13.db /tmp/
sudo systemctl stop youwin
sudo -u youwin cp /tmp/youwin-2026-08-13.db /var/lib/youwin/youwin.db
sudo -u youwin rm -f /var/lib/youwin/youwin.db-wal /var/lib/youwin/youwin.db-shm
sudo systemctl start youwin
```

Removing the sidecars matters: they belong to the database you replaced, and
leaving them beside a different file is how you corrupt the one you just
restored.

If the youwin.dev box is gone entirely, the `.json` is the one that does not need
it. It is complete enough to rebuild the database, deletions included, and
readable with no SQLite and no Rust toolchain.

## Updating it

There is no pipeline, so this is the whole procedure:

```bash
# [local] download the artifact from Actions → offsite
sudo install -m 755 -o root -g root youwin-offsite /usr/local/bin/youwin-offsite
sudo systemctl restart youwin-offsite
/usr/local/bin/youwin-offsite version
```

If the update also changed the unit file or the Caddy block — both travel in the
artifact — install those too, then `daemon-reload` or `caddy validate` and reload
as in steps 6 and 7. The `diff -q` pair in
[Verifying the setup](#verifying-the-setup) is how you find out you forgot;
silence means everything installed matches what is running.

Restarting mid-upload is safe: the service handles `SIGTERM`, and an upload cut
off part way leaves a `.part` that the sender retries the next night and that
retention is structurally incapable of mistaking for a backup.

## Two limits that must move together

| | |
|---|---|
| `request_body max_size 512MB` | in `backup.youwin.dev.caddy` — rejects before a byte reaches the service |
| `YOUWIN_OFFSITE_MAX_BYTES` | default `536870912`, enforced as the body streams to disk |

They are independent caps on the same thing, set to the same number, and Caddy's
is the one that normally fires. The service's exists because a Caddyfile can be
edited by somebody who does not know this is behind it. Raising one alone does
nothing except move which layer says no — and if the archive ever genuinely
outgrows 512MB, raise both **and** check the Cloudflare cloud is still grey,
because 100MB would have been the real ceiling all along.
