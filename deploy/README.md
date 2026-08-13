# Deploying youwin.dev

Deploys run entirely from GitHub Actions: push to `master`, and CI builds, tests,
ships, activates, health-checks, and rolls back if the site does not come up.
Nothing is built on your machine and nothing is built on the server.

That includes the *first* deploy — there is no bootstrap script, and the steps
below are ordered so CI can do it. What is left over is provisioning that needs
root and one password that must never touch CI, which is most of this document.

## What CI can and cannot do

**Entirely CI:** frontend build, `cargo test`, release build, upload, symlink
flip, restart, health check, automatic rollback, pruning old releases — the first
deploy included. There is no build step on the server and no Rust toolchain on
it; the binary arrives prebuilt. Nor is there a local deploy path to keep in
sync, which is deliberate: one route that runs on every push beats two where the
rarely-used one quietly rots.

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

One directory per site under `/srv/sites`, named for the domain — the same shape
`timothyyuen.io` uses, so the box has one answer to "where does a site live".

```
/srv/sites/
├─ timothyyuen.io/           static, served straight off the symlink
└─ youwin.dev/
   ├─ releases/20260808-143000-a1b2c3d/
   │  ├─ bin/youwin-server   built in CI; no toolchain on the server
   │  ├─ public/             hashed CSS, favicons, robots.txt
   │  ├─ write/              the SPA, sw.js, manifest
   │  └─ deploy/             unit files and this document, travelling with the release
   ├─ current  -> releases/20260808-143000-a1b2c3d
   └─ previous -> releases/20260807-101500-9f8e7d6

/var/lib/youwin/youwin.db    never touched by a deploy
/var/backups/youwin/         nightly backup + export
/etc/youwin/secrets.env      password hash, purge token, off-site URL — if used
```

`current` is what the systemd unit and the Caddy roots point at, so the binary
and the assets it serves change together or not at all. The database is not in
`/srv` at all — `/var/lib` is where the FHS puts application state, and no
deploy goes near it.

**One deliberate difference from the static sites.** There, the site directory
is owned by `deploy`, which is why `activate-release` needs no `sudo`. Here it is
owned by `root` and only `releases/` is writable by `deploy`, because `current`
decides which **binary** systemd executes — and the nightly backup timer runs
that binary too. A `deploy`-writable `current` would let anything uploaded to
`releases/` be executed as the `youwin` user without passing the smoke test, the
health check, or the rollback. Uploading a release and *asking* for it to be
activated is the intended power; swapping the running binary directly is not.

## First-time setup

Steps are marked **[server]** (as your admin user, over SSH), **[local]**, or
**[dashboard]**.

**1. [server] Users and directories**

Assumes `/srv/sites` and the `deploy` user already exist from the
`timothyyuen.io` setup; if not, create them the same way — `deploy` gets no
password, no sudo, and one SSH key.

```bash
sudo useradd --system --home /srv/sites/youwin.dev --shell /usr/sbin/nologin youwin
sudo install -d -m 755 -o root   -g root   /srv/sites/youwin.dev /etc/youwin
sudo install -d -m 755 -o deploy -g deploy /srv/sites/youwin.dev/releases
sudo install -d -m 750 -o youwin -g youwin /var/lib/youwin /var/backups/youwin
```

Note the ownership split, which differs from the static sites on purpose: only
`releases/` belongs to `deploy`, so that is the only thing CI can write to.
`/srv/sites/youwin.dev` itself is root-owned, so `current` can be moved only by
the activation script — see "Layout on the server" for why that matters here and
not there.

**2. [server] The activation script and its sudoers rule**

Get the repo onto the box however you like (`git clone`, or scp the `deploy/`
directory); after the first deploy it also lives at `/srv/sites/youwin.dev/current/deploy`.

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

Install them, but do **not** enable `youwin` yet — there is no binary and no
password hash. Step 8 brings it up.

The unit does need to exist before step 7, though: `activate-youwin` restarts it,
and "unit not found" is a less informative failure than "cannot start without
`YOUWIN_PASSWORD_HASH`".

**4. [dashboard] DNS, on Cloudflare**

The zone has to be on Cloudflare before step 5 can work: TLS here is DNS-01,
which proves control by writing a `_acme-challenge` TXT record through the
Cloudflare API, so Cloudflare must be authoritative for `youwin.dev`. Add the
zone, change the nameservers at the registrar, wait for **Active**.

Then `A`/`AAAA` records for the apex, `www`, and `write` — all pointing at this
server. Leave them **DNS-only (grey cloud)** for now. The proxy goes on in step
9a, once TLS is confirmed working, so that a certificate problem and a proxy
problem cannot arrive at the same time.

**5. [server] Caddy**

Three parts, in order: the plugin, the token, then the site file. Installing the
site file before the other two gets you a Caddy that will not load its config at
all — including for the sites already on the box.

> **The stock Caddy package cannot do this.** `youwin.dev.caddy` asks for TLS via
> Cloudflare DNS-01, which lives in `github.com/caddy-dns/cloudflare` — a module
> the Cloudsmith package does not ship. Drop the site file in first and
> `caddy validate` fails with
> `module not registered: dns.providers.cloudflare`. Loud, at least, rather than
> a site that half works.
>
> If you would rather not replace the binary, see *Alternatives to DNS-01* at
> the end of this step.

**5a. Give Caddy the Cloudflare DNS module.**

```bash
sudo caddy add-package github.com/caddy-dns/cloudflare
caddy list-modules | grep dns.providers.cloudflare   # expect one line
sudo systemctl restart caddy
```

`add-package` downloads a replacement binary from Caddy's build service and
writes it over `/usr/bin/caddy` — the same path the `.deb` owns. So:

```bash
sudo apt-mark hold caddy
```

Without the hold, the next `apt upgrade` silently restores the stock binary and
Caddy then refuses to start, because its config references a module that is no
longer there — a broken site at whatever hour you happened to run an upgrade.
Debian's unattended-upgrades only covers Debian's own origins, not Cloudsmith,
so this is a manual-upgrade hazard rather than an overnight one. Update Caddy
with its own updater instead, which preserves the module set:

```bash
sudo caddy upgrade && sudo systemctl restart caddy
```

**5b. The API token.** Create one at Cloudflare → My Profile → API Tokens, with
the **Edit zone DNS** template scoped to `youwin.dev` (and `timothyyuen.io` too,
if you are also moving that site onto Cloudflare — see the section after the
setup steps). This is a *third* token, separate from the cache-purge one in
step 11 — different jobs, different blast radius.

The Caddyfile reads it as `{env.CF_API_TOKEN}`, and Caddy's unit does not load an
environment file by default:

```bash
printf 'CF_API_TOKEN=%s\n' '<token>' | sudo tee /etc/caddy/caddy.env
sudo chown root:caddy /etc/caddy/caddy.env
sudo chmod 640 /etc/caddy/caddy.env

sudo systemctl edit caddy      # creates a drop-in; do not edit the unit itself
```

```ini
[Service]
EnvironmentFile=/etc/caddy/caddy.env
```

```bash
sudo systemctl daemon-reload && sudo systemctl restart caddy
```

`0640 root:caddy` rather than `0600 root:root`: the unit runs as `caddy`, and a
file it cannot read produces an empty token and an authentication error from the
Cloudflare API that reads nothing like a permissions problem.

**5c. The site file.**

```bash
sudo install -m 644 -o root -g root deploy/youwin.dev.caddy /etc/caddy/conf.d/youwin.dev.caddy

# Validate with the token in scope, then hand the log files back to caddy.
# Both halves matter — see below.
sudo bash -c 'set -a; . /etc/caddy/caddy.env; caddy validate --config /etc/caddy/Caddyfile'
sudo chown -R caddy:caddy /var/log/caddy

sudo systemctl reload caddy
journalctl -u caddy -f          # watch both certificates get issued
```

Two things about that validate command, both of which bite as a plain
`sudo caddy validate --config /etc/caddy/Caddyfile`:

- **The token has to be in the environment.** It lives in the systemd
  `EnvironmentFile`, which a root shell knows nothing about, so validation
  reaches TLS provisioning and dies with
  `API token '' appears invalid`. Nothing is wrong with the config; it just
  cannot see the token. Sourcing `caddy.env` first fixes it — and as a side
  benefit, the Cloudflare module sanity-checks the token's *shape*, so a
  truncated paste is caught here rather than at first renewal.
- **Validation creates the log files.** It provisions the whole config, log
  writers included, so it creates `/var/log/caddy/*.log` **mode 0600 owned by
  whoever ran it** — root. Caddy then runs as `caddy`, cannot open its own log,
  and fails to start. The `chown` puts that right. (Worth running once over the
  whole directory anyway if you have ever validated as root before.)

Issuance takes a few seconds per hostname. Requests arriving in that window get a
TLS handshake failure, so do this promptly rather than leaving it half-done.

Confirm before moving on:

```bash
curl -fsSI https://youwin.dev | head -1
curl -fsSI https://write.youwin.dev | head -1
```

> `Strict-Transport-Security` with `includeSubDomains` is set on both blocks,
> which commits every browser that sees it to HTTPS-only across `youwin.dev` for
> a year. That is intended, but it means `write.youwin.dev` has to have working
> TLS from the moment the apex is first served — which is why both go live
> together rather than one at a time.

**Alternatives to DNS-01**, if replacing the Caddy binary is unappealing:

| | |
|---|---|
| **HTTP-01 / TLS-ALPN** — delete the `tls` blocks entirely | Stock Caddy, zero setup. Works today. But TLS-ALPN cannot work once the orange cloud is on (Cloudflare terminates TLS), leaving HTTP-01 as the only path, which depends on Cloudflare forwarding `/.well-known/acme-challenge` to the origin. It does, but renewal now has a dependency you did not choose. |
| **Cloudflare Origin CA** — `tls /path/cert.pem /path/key.pem` | A free 15-year certificate, no ACME at all. But it is trusted *only* by Cloudflare, so the site breaks for browsers the moment the proxy is turned off — and requires SSL mode "Full (strict)". |

DNS-01 is the one that keeps working whatever the proxy is doing, which is why
it is the default here.

**6. [local + GitHub] Hand CI the keys**

Generate a deploy key *for CI only* — not your admin key. It goes on the
`deploy` user, which has no password and cannot log in interactively.

```bash
ssh-keygen -t ed25519 -N '' -C 'github-actions youwin.dev' -f ~/.ssh/youwindev_deploy
ssh-keyscan -t ed25519 server.example       # for DEPLOY_KNOWN_HOSTS
```

Install the public half on the server:

```bash
sudo tee -a /home/deploy/.ssh/authorized_keys < ~/.ssh/youwindev_deploy.pub
```

In the repo's **Settings → Secrets and variables → Actions**:

| Secret | Value |
|---|---|
| `DEPLOY_SSH_KEY` | the whole of `~/.ssh/youwindev_deploy` (private half) |
| `DEPLOY_KNOWN_HOSTS` | the `ssh-keyscan` output |
| `DEPLOY_HOST` | `server.example` |

**7. [GitHub] First deploy — it is supposed to go red**

Push to `master`, or run the workflow by hand from the Actions tab.

It will build, test, upload the release, point `current` at it — and then fail,
because `youwin.service` cannot start without a password hash that does not
exist yet. That is the expected outcome and the reason this ordering exists: the
binary has to be on the box before you can generate a hash with it, and the hash
has to exist before the service can start.

What matters is that the release landed. Check:

```bash
readlink -f /srv/sites/youwin.dev/current
/srv/sites/youwin.dev/current/bin/youwin-server version
```

**8. [server] The password, then bring it up**

```bash
sudo -u youwin /srv/sites/youwin.dev/current/bin/youwin-server hash-password \
  | sudo tee /etc/youwin/secrets.env
sudo chmod 0600 /etc/youwin/secrets.env

# Step 7 left the unit in a failed state, and `Restart=on-failure` will have
# retried until systemd's start-rate limit tripped. Without this, `start` is
# refused with "Start request repeated too quickly".
sudo systemctl reset-failed youwin

sudo systemctl enable --now youwin
systemctl status youwin
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

Then **re-run the failed workflow** from the Actions tab. This time activation
finds a service it can restart, both health checks pass, and it goes green —
which is also the proof that the whole pipeline works end to end.

Now actually use it: open `https://write.youwin.dev`, sign in with the password
you just set, and post something. Then check it appears at `https://youwin.dev`.

That is the acceptance test for everything above — TLS on both hosts, the session
cookie, the write pool, the render pipeline, and the public read path — and it
has to happen before step 11 can be verified at all, since there is nothing to
purge until something has been written.

**9a. [dashboard] Turn on the proxy**

Set **SSL/TLS → Overview → Full (strict)** first. Cloudflare's default on a new
zone can be Flexible, which terminates TLS at the edge and speaks *plain HTTP* to
the origin — with HSTS being sent by both blocks, that is a redirect loop waiting
to happen. Full (strict) is correct here because the origin has a real
publicly-trusted certificate from step 5.

Now flip the apex, `www` and `write` records from grey to **orange**. Confirm the
site still answers, and that it is now coming through Cloudflare:

```bash
curl -fsSI https://youwin.dev | grep -iE 'server|cf-ray'
```

**9b. [dashboard] The cache rule — do not skip this**

The public site sends `Cache-Control: public, max-age=60, s-maxage=300`, but
**Cloudflare does not cache HTML by default**, so that header is inert until a
rule exists. Every request reaches the origin, and the main benefit of a
cookieless, JS-free site is left on the table. The symptom is
`cf-cache-status: DYNAMIC` — Cloudflare's way of saying "not eligible", as
opposed to `BYPASS`, which means eligible but deliberately skipped.

> **`write.youwin.dev` is in this same zone.** It is a subdomain of `youwin.dev`,
> so a rule built on the default *All incoming requests* would apply to the
> authoring app as well, and start caching authenticated API responses at the
> edge. The filter below is not a nicety — it is the whole safety of this step.

**Caching → Cache Rules → Create rule.**

1. **Name:** `Cache public HTML`.
2. **When incoming requests match:** choose **Custom filter expression**, *not*
   "All incoming requests", and paste this into the expression editor:

   ```
   (http.host in {"youwin.dev" "www.youwin.dev"})
   ```

   That is the same boundary the application draws: two hostnames, one of which
   is a cookieless read-only surface and the other of which is not.
3. **Then → Cache eligibility:** **Eligible for cache**.
4. **Edge TTL:** the option that defers to the origin — currently labelled
   *"Use cache-control header if present, use default otherwise"*. That is what
   picks up `s-maxage=300`; Cloudflare prefers `s-maxage` over `max-age` for its
   own cache when both are present.
5. **Browser TTL:** likewise *"Respect origin TTL"*, which leaves visitors on the
   `max-age=60` the origin already sends.
6. **Deploy.**

Labels in this UI move around between redesigns; the intent to hold on to is
*eligible for cache, and take both TTLs from the origin's header*. Nothing here
should override a TTL — the origin's numbers are chosen deliberately, and
step 11 exists precisely so they can stay long.

Now verify, because this is the easiest step in the document to believe without
checking:

```bash
# The public site: MISS on the first request, HIT on the second.
curl -fsSI https://youwin.dev | grep -i cf-cache-status
curl -fsSI https://youwin.dev | grep -i cf-cache-status

# The authoring host must NOT be caching. Expect DYNAMIC, never HIT.
curl -fsSI https://write.youwin.dev/api/health | grep -i cf-cache-status
```

If the second call still says `DYNAMIC`, the rule is not matching — check the
expression before anything else. If it says `BYPASS`, the rule matched but
something in the response opted out; on this site that would be surprising, since
it sets no cookies.

To re-test after a change, **Caching → Configuration → Purge Everything**.

Two things worth knowing rather than discovering:

- `/assets/*` was *already* cached before this rule, by Cloudflare's default
  handling of static extensions. The rule is for the HTML, the Atom feed, and the
  404s.
- Cache rules are per-zone, so `timothyyuen.io` is untouched by this and needs no
  rule of its own — see the section below for why its headers already do the
  right thing.

> Once the proxy is on, the origin sees Cloudflare's addresses rather than
> visitors'. The app reads `CF-Connecting-IP` for its login throttle, which is
> sound because the backend binds loopback only and Caddy is its sole possible
> peer. It does mean someone who finds the origin IP could reach port 443
> directly and spoof that header to sidestep the throttle. If that bothers you,
> restrict 80/443 to [Cloudflare's ranges](https://www.cloudflare.com/ips/) with
> `ufw` — worth doing eventually, not a blocker.

**10. [server] Backups**

```bash
sudo systemctl enable --now youwin-backup.timer
sudo systemctl start youwin-backup.service   # take one now rather than waiting
sudo systemctl status youwin-backup.service  # oneshot: confirm it exited 0
```

This writes to `/var/backups/youwin` — the same disk as the database. See
[Off-site copies](#off-site-copies) for the half that makes it a backup rather
than a second copy; it is optional and configured in the same `secrets.env`.

**11. [optional] Cache purging on write**

Skip this and the site runs on the `s-maxage` TTL alone, which is correct — an
edit just takes up to five minutes to appear. This closes that gap. It needs two
values.

**The zone ID** identifies which zone to purge, and goes straight into the API
path the server calls (`/zones/<id>/purge_cache`). It is a 32-character hex
string, *not* the domain name — the API will not accept `youwin.dev` there.

Cloudflare dashboard → select the **youwin.dev** zone → **Overview** → the
right-hand column, under **API** → **Zone ID**, with a copy button. It is not a
secret; it appears in URLs and support threads routinely, which is why it sits
beside the token rather than being treated like one.

From the command line instead, with any token that can read zones:

```bash
curl -s -H "Authorization: Bearer $CF_TOKEN" \
  'https://api.cloudflare.com/client/v4/zones?name=youwin.dev' \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"][0]["id"])'
```

**The purge token** is a **second** API token, separate from the DNS-01 one Caddy
uses — that one is scoped to DNS edits and should stay that way. My Profile →
API Tokens → **Create Token** → Create Custom Token:

- **Permissions:** `Zone` → `Cache Purge` → `Purge`
- **Zone Resources:** Include → Specific zone → `youwin.dev`

Both go in `secrets.env`, which the unit already loads — nothing edits the unit
file itself, because a deploy can reinstall it and take a hand-edited value with
it:

```bash
sudo tee -a /etc/youwin/secrets.env >/dev/null <<'EOF'
YOUWIN_CF_ZONE_ID=<32 hex characters>
YOUWIN_CF_PURGE_TOKEN=<the purge token>
EOF
sudo systemctl restart youwin

journalctl -u youwin -n 20 | grep 'edge cache'   # expect: cache_purging="on"
```

That log line only says the server *found* both values. To prove the credentials
actually work, watch the log while you write something:

```bash
journalctl -u youwin -f
```

Then create or edit a post in the app. Expect `purged the edge cache`. A failure
logs `cache purge refused` together with the HTTP status **and Cloudflare's
response body**, which names the reason — a 404 means the zone ID is wrong, a 403
means the token lacks Cache Purge or is not scoped to this zone. That body is
logged for exactly this moment; a bare status code would leave those two
indistinguishable.

## Verifying the setup

Worth running once at the end, and again any time something behaves oddly. Each
line checks a specific thing an earlier step was supposed to establish; the
comment says what "right" looks like.

```bash
# 1 — ownership. The split is the point: only releases/ belongs to deploy.
ls -ld /srv/sites/youwin.dev /srv/sites/youwin.dev/releases \
       /var/lib/youwin /var/backups/youwin /etc/youwin
#   /srv/sites/youwin.dev            root:root     755
#   /srv/sites/youwin.dev/releases   deploy:deploy 755
#   /var/lib/youwin                  youwin:youwin 750

# 2 — the privilege boundary, as sudo itself sees it.
ls -l /usr/local/bin/activate-youwin        # root:root 755
sudo -l -U deploy                           # exactly one NOPASSWD entry

# 3 — units installed; youwin enabled, backup timer enabled.
systemctl is-enabled youwin youwin-backup.timer

# 4 — all three names resolve.
dig +short youwin.dev www.youwin.dev write.youwin.dev

# 5 — Caddy has the DNS module, is held back from apt, can read its token,
#     and owns its own log files.
caddy list-modules | grep dns.providers.cloudflare
apt-mark showhold | grep -x caddy           # must print: caddy
ls -l /etc/caddy/caddy.env                  # root:caddy 640
systemctl cat caddy | grep EnvironmentFile  # the drop-in is in effect
ls -l /var/log/caddy                        # every file caddy:caddy, none root

# 6-8 — a release is live, the service is up, the secret is locked down.
readlink -f /srv/sites/youwin.dev/current
/srv/sites/youwin.dev/current/bin/youwin-server version
systemctl is-active youwin
ls -l /etc/youwin/secrets.env               # root:root 600

# 9 — the public site caches, the authoring host does not. See below.
curl -fsSI https://youwin.dev | grep -i cf-cache-status
curl -fsSI https://youwin.dev | grep -i cf-cache-status
curl -fsSI https://write.youwin.dev/api/health | grep -i cf-cache-status

# 10 — the timer is scheduled and the last run exited clean.
systemctl list-timers youwin-backup.timer
systemctl show -p ExecMainStatus youwin-backup.service   # 0
ls -l /var/backups/youwin

# 11 — nothing in deploy/ has shipped without being installed. A deploy cannot
#      install these, so a change to one of them sits on disk doing nothing
#      until you act. Silence is correct. See "After deploying a change to
#      anything in deploy/".
cd /srv/sites/youwin.dev/current/deploy
diff -q youwin.dev.caddy /etc/caddy/conf.d/youwin.dev.caddy
diff -q activate-youwin  /usr/local/bin/activate-youwin
for u in youwin.service youwin-backup.service youwin-backup.timer; do
  diff -q "$u" "/etc/systemd/system/$u"
done
```

**The one to look at hardest is `write.youwin.dev`.** If it reports anything
other than `DYNAMIC` — a `HIT`, a `MISS`, an `EXPIRED` — the cache rule from step
9b is matching the authoring host, which means Cloudflare is caching
authenticated responses at the edge. That happens when the rule was built on the
dashboard's default *All incoming requests* rather than a filter expression.
Fix it by editing the rule to use:

```
(http.host in {"youwin.dev" "www.youwin.dev"})
```

then **Caching → Configuration → Purge Everything** to discard whatever it
already stored.

## Moving timothyyuen.io onto Cloudflare at the same time

Optional, and genuinely convenient to do here: step 5b replaced the Caddy binary
with one that can do Cloudflare DNS-01, and that binary serves every site on the
box. The static site can use it too, and gets a CDN in front of it.

Do this **after** `youwin.dev` is fully working. If something is wrong with the
new Caddy binary or the token, you want to find out on the site that is not yet
carrying traffic.

**1. [dashboard] Add the zone.** Add `timothyyuen.io` to Cloudflare and change the
nameservers at the registrar. Cloudflare imports the existing records; check the
apex and `www` came across pointing at this server's IP before the zone goes
Active.

Because the record *values* do not change — same server, same address — the
nameserver migration itself is invisible to visitors. Nothing moves until you
turn the orange cloud on, which is step 4.

**2. [dashboard] Widen the DNS token.** The token from step 5b needs
`timothyyuen.io` in its zone list as well. Edit it in place rather than making a
second one; Caddy reads a single `CF_API_TOKEN` for every site it manages.

**3. [server] Switch its TLS to DNS-01.** Its certificate currently comes from
HTTP-01 or TLS-ALPN, and TLS-ALPN stops working the moment the proxy is on —
Cloudflare terminates TLS at the edge, so the challenge never reaches the origin.
Rather than depend on Cloudflare forwarding ACME challenges over port 80, move it
to the same mechanism `youwin.dev` uses:

```bash
sudo tee /etc/caddy/conf.d/timothyyuen.io.caddy <<'EOF'
timothyyuen.io {
	tls {
		dns cloudflare {env.CF_API_TOKEN}
	}
	import static_site timothyyuen.io
}

www.timothyyuen.io {
	tls {
		dns cloudflare {env.CF_API_TOKEN}
	}
	redir https://timothyyuen.io{uri} permanent
}
EOF
sudo bash -c 'set -a; . /etc/caddy/caddy.env; caddy validate --config /etc/caddy/Caddyfile'
sudo chown -R caddy:caddy /var/log/caddy
sudo systemctl reload caddy
```

The existing certificate stays valid until it expires, so this changes nothing
visible today — it changes what happens at the *next renewal*, which is exactly
the failure you do not want to discover sixty days from now. Force the issue and
confirm it works now rather than trusting it:

```bash
# Ask for a fresh certificate through the new path.
sudo systemctl stop caddy
sudo rm -rf /var/lib/caddy/.local/share/caddy/certificates/*/timothyyuen.io
sudo systemctl start caddy
journalctl -u caddy -f     # expect: obtaining certificate, using DNS challenge
curl -fsSI https://timothyyuen.io | head -1
```

**4. [dashboard] Turn on the proxy.** SSL/TLS → **Full (strict)** first, for the
same reason as step 9a — Flexible plus the site's HSTS header is a redirect loop.
Then flip the apex and `www` to orange.

No cache rule is needed. The `(static_site)` snippet already sends
`max-age=31536000, immutable` for `/_build/*` and `max-age=0, must-revalidate`
for everything else, and Cloudflare respects both: the hashed bundles cache at
the edge, HTML always revalidates, and a deploy is visible immediately. That is a
nicer arrangement than `youwin.dev` has, and it comes for free from the headers
already being right.

**5. Check what the logs are now recording.** With the proxy on, Caddy's access
log for that site records Cloudflare's addresses rather than visitors'. If the
log is only ever read for debugging, ignore this. If you want real client
addresses, add to the global block in `/etc/caddy/Caddyfile`:

```
servers {
	trusted_proxies static <cloudflare ranges…>
	client_ip_headers CF-Connecting-IP
}
```

Keeping that list current is a maintenance job, which is why it is a footnote
rather than a step.

**Rolling it back** is one dashboard toggle: set the records back to DNS-only.
The origin certificate is a real publicly-trusted one, so the site keeps working
with the proxy off — which is precisely the property DNS-01 buys and an Origin CA
certificate would not.

## Routine deploys

Push to `master`. That is the whole procedure — for code, which is almost every
change. The exception is anything in `deploy/` itself: those need root to
install, so CI ships them and cannot apply them. See
[After deploying a change to anything in `deploy/`](#after-deploying-a-change-to-anything-in-deploy).

The workflow is [`.github/workflows/deploy.yml`](../.github/workflows/deploy.yml):
`pnpm install` → typecheck → build → check the expected build outputs exist →
`cargo test` → `cargo build --release` → assemble a release directory → rsync it
→ `activate-youwin`.

Things in there that are load-bearing:

- **`runs-on: ubuntu-24.04`, pinned, and the glibc floor is asserted.** The
  server is Debian 13 (glibc 2.41); the binary links only `libc`, `libm` and
  `libgcc_s` — SQLite is bundled, rustls needs no OpenSSL — and measures out at
  **GLIBC_2.38**, so it loads with room to spare. `SERVER_GLIBC` in the workflow
  states 2.41 and a step fails the build if the binary ever needs more than
  that, before anything is uploaded. Update it only when the server itself is
  upgraded. If a dependency ever pushes the requirement past the server, build
  in a matching container instead (`container: rust:1-trixie`).

  Worth knowing: the requirement comes from which symbol versions the functions
  in use last changed at, **not** from the builder's glibc. Building on 2.39
  does not by itself demand 2.39 at runtime — this same source measures 2.38
  whether it is built on Ubuntu 24.04 or Debian 13.
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
ls /srv/sites/youwin.dev/releases                            # or pick one
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
sudo -u youwin /srv/sites/youwin.dev/current/bin/youwin-server rerender
```

Idempotent, safe at any time, and it does not mark anything as edited.

### After deploying a change to anything in `deploy/`

**A deploy does not install these.** CI copies the whole directory into the
release, so `current/deploy/` always holds the versions matching the running
binary — but `activate-youwin` only flips a symlink and restarts a service, and
that is the entire privilege boundary. Installing a systemd unit or a Caddy
block needs root, which the `deploy` user does not have and should not get.

So a change to one of these files ships, sits on disk, and does nothing until
you install it. Every one of them fails silently when you forget: the site stays
up, the timer keeps firing, and the thing you changed simply is not in effect.

Install from `current/`, **after** the deploy carrying the change has landed —
that is what puts the new copy on the box in the first place:

| File | Installs to | Silent failure if skipped |
|---|---|---|
| `youwin.dev.caddy` | `/etc/caddy/conf.d/` | A block you added does nothing. Anything relying on it behaves as though it were absent — a header not set, a path not special-cased, an origin response overwritten by the catch-all |
| `youwin.service` | `/etc/systemd/system/` | The process keeps its old environment. A new `Environment=` line is invisible, and so is a hardening change |
| `youwin-backup.service`<br>`youwin-backup.timer` | `/etc/systemd/system/` | The nightly run keeps working and keeps doing the old thing. A new `EnvironmentFile` means a variable you set in `secrets.env` is never seen — which looks exactly like not having configured it |
| `activate-youwin` | `/usr/local/bin/` | The **next** deploy still runs the old script — see the note below |

`deploy/offsite/` is **not** in that table and installs nowhere on this box. CI
copies the whole directory into the release, so those files ride along and sit
there inert — which is deliberate, so the receiver's runbook is on the machine
you are already logged into when you go looking for it. Installing them belongs
to a different box entirely; see [`offsite/README.md`](offsite/README.md).

**Caddy** — validate with the token in scope and hand the log files back, both
for the reasons spelled out in step 5c above: a plain `sudo caddy validate`
fails on a token it cannot see, and creates root-owned log files that Caddy then
cannot open.

```bash
sudo install -m 644 -o root -g root /srv/sites/youwin.dev/current/deploy/youwin.dev.caddy /etc/caddy/conf.d/youwin.dev.caddy
sudo bash -c 'set -a; . /etc/caddy/caddy.env; caddy validate --config /etc/caddy/Caddyfile'
sudo chown -R caddy:caddy /var/log/caddy
sudo systemctl reload caddy
```

**systemd** — `daemon-reload` makes the unit current; the service also needs a
restart, and `youwin-backup` does not, since the timer starts a fresh oneshot
each night.

```bash
cd /srv/sites/youwin.dev/current/deploy
sudo install -m 644 youwin.service youwin-backup.service youwin-backup.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl restart youwin          # only if youwin.service itself changed
```

**`activate-youwin`** is the awkward one, because it is the thing CI runs. A
change to it arrives *with* a release that was activated by the previous
version, so it takes effect one deploy later:

```bash
sudo install -m 755 -o root -g root /srv/sites/youwin.dev/current/deploy/activate-youwin /usr/local/bin/activate-youwin
```

If the release that shipped the change actually needed the new behaviour,
install it and re-run the activation by hand — it is idempotent, and activating
the release that is already live is a no-op beyond a restart:

```bash
sudo /usr/local/bin/activate-youwin "$(basename "$(readlink -f /srv/sites/youwin.dev/current)")"
```

**Is anything outstanding?** This is the question worth being able to answer at
a glance, and it is why the release carries `deploy/` at all. Silence means
everything installed matches what is running:

```bash
cd /srv/sites/youwin.dev/current/deploy
diff -q youwin.dev.caddy /etc/caddy/conf.d/youwin.dev.caddy
diff -q activate-youwin  /usr/local/bin/activate-youwin
for u in youwin.service youwin-backup.service youwin-backup.timer; do
  diff -q "$u" "/etc/systemd/system/$u"
done
```

Worth running after any deploy that touched this directory, and worth running
when something behaves as though a change you made never happened — because
that is exactly what has occurred.

### Shipping without pushing

There is no local deploy script, on purpose. One path that is exercised on every
push beats two paths where the rarely-used one quietly rots — which is exactly
what happened to the script that used to live here.

If you need to ship something that is not on `master`, run the workflow by hand
against a branch: **Actions → deploy → Run workflow → pick the branch.** It takes
the same path as any other deploy, including the tests.

If GitHub itself is down, the shape of a manual deploy is the "Assemble the
release" and "Upload and activate" steps of
[`deploy.yml`](../.github/workflows/deploy.yml) run by hand: build the frontends
and the binary on a Linux host, arrange `bin/`, `public/`, `write/` and `deploy/`
in a directory, `rsync` it to `deploy@host:/srv/sites/youwin.dev/releases/<name>/`,
then `ssh deploy@host sudo /usr/local/bin/activate-youwin <name>`. Reading it out
of the workflow means it cannot describe a layout the real deploy stopped using.

## Checking on it — [server]

```bash
systemctl status youwin
journalctl -u youwin -f
readlink -f /srv/sites/youwin.dev/current                 # which release is live
/srv/sites/youwin.dev/current/bin/youwin-server version   # which commit it was built from
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

### Off-site copies

A backup that only exists on the same disk as the thing it backs up is not one.
Set `YOUWIN_OFFSITE_URL` and each nightly run also uploads two files:

| | |
|---|---|
| `youwin-YYYY-MM-DD.db` | the `VACUUM INTO` snapshot — what you restore from |
| `youwin-YYYY-MM-DD.json` | the same `posts.json`, dated — readable with no SQLite and no toolchain |

The markdown tree stays local: every line of it is derivable from that JSON, and
sending a directory would mean either a tar dependency or one request per post.

It is a plain `PUT` to `{YOUWIN_OFFSITE_URL}/{filename}` with an optional
`Authorization` header, so the target can be a Storage Box, rsync.net,
Nextcloud, an S3 gateway, a WebDAV server, or an nginx with `dav_methods` — no
provider SDK and no signing algorithm. Both values go in the same file the other
secrets do, because a deploy reinstalls the unit and would take a hand-edited
value with it:

```bash
sudo tee -a /etc/youwin/secrets.env >/dev/null <<'EOF'
YOUWIN_OFFSITE_URL=https://u123456.your-storagebox.de/youwin
YOUWIN_OFFSITE_AUTH=Basic <base64 of user:password>
EOF

sudo systemctl start youwin-backup.service
journalctl -u youwin-backup.service -n 20    # expect two "Uploaded …" lines
```

`YOUWIN_OFFSITE_AUTH` is a **complete header value** — `Bearer …`, `Basic …`,
whatever the target wants — rather than a token plus a scheme setting, which
would be a second thing to configure that can only ever be wrong. Omit it
entirely for a target that authenticates through the URL itself.

**If the target is a box you own**, `youwin-offsite` is the other half of this,
and the reason to prefer it over any of the above: it opens each arriving
snapshot and runs `PRAGMA integrity_check` before accepting it, so a backup that
went bad turns *this* unit red on the night it happened rather than on the day
you need it. It lives on a different machine and is provisioned entirely
separately — see [`deploy/offsite/README.md`](offsite/README.md). Nothing on
this box changes except the two lines above:

```
YOUWIN_OFFSITE_URL=https://backup.youwin.dev
```

with no path on the end — the receiver answers at the root, and a trailing path
segment gets a 400 whose body says so, on a machine you are not looking at.

**A failed upload fails the unit**, unlike the cache purge, which is fire and
forget. `systemctl status youwin-backup.service` shows non-zero and the journal
carries the status and the remote's own response body, which is what tells a
wrong path apart from a full quota or an expired credential. This is the one
place in the deploy where silence would be the dangerous outcome: a timer that
exits zero having uploaded nothing looks exactly like one that worked.

Retention off-site is the remote's job. This never deletes anything it did not
just write, and every target worth using has lifecycle rules of its own.

Leave `YOUWIN_OFFSITE_URL` unset and none of this happens — local dated
snapshots are still taken, and pulling them down by hand stays perfectly good:

```bash
rsync -avz youwin-admin@server.example:/var/backups/youwin/ /mnt/c/Users/theaz/backups/youwin/
```

## Rotating the password — [server]

```bash
sudo -u youwin /srv/sites/youwin.dev/current/bin/youwin-server hash-password \
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
