#!/usr/bin/env bash
#
# Sync the tree, build the binary on the server, restart, health-check.
#
# Written to be run from WSL2 Debian against a Windows checkout under /mnt/c,
# which is the setup here — but it does not require that, and behaves the same
# from a native Linux checkout.
#
# Cross-compiling is skipped deliberately: sqlx's sqlite feature bundles SQLite
# through libsqlite3-sys, so a cross build needs a cross C toolchain. The server
# already has cc; the first build there is slow and every one after is
# incremental.
#
# Usage:  ./deploy/deploy.sh [ssh-host]
#         YOUWIN_SKIP_BUILD=1 ./deploy/deploy.sh   # assets already built
#
# Needs:  rsync + ssh locally; rust + cc on the server.
#         A frontend build — see "Frontends" below for who does it.

set -euo pipefail

HOST="${1:-${YOUWIN_HOST:-youwin}}"
REMOTE_SRC="/srv/youwin/src"
REMOTE_BIN="/srv/youwin/bin/youwin-server"
WWW="/var/www/youwin"
MANIFEST="web/dist/public/.vite/manifest.json"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

die() { printf '\n%s\n' "$*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Frontends
#
# `pnpm` may be on PATH under WSL and still be unusable: interop appends the
# Windows PATH, so a Windows pnpm resolves to something under /mnt/. Running it
# would be worse than not finding it — web/node_modules holds native binaries
# for exactly one platform (@rollup/rollup-win32-x64-msvc, @tailwindcss/oxide,
# lightningcss), and the two installs cannot share a directory.
#
# So: build here only with a genuinely native pnpm. Otherwise use what is
# already in web/dist and refuse to ship it if it is stale.
# ---------------------------------------------------------------------------

native_pnpm() {
  local found
  found="$(command -v pnpm 2>/dev/null)" || return 1
  case "$found" in
    /mnt/*) return 1 ;;
  esac
  pnpm --version >/dev/null 2>&1
}

# How to produce a build, phrased for wherever this is running.
build_hint() {
  if [ -n "${WSL_DISTRO_NAME:-}" ] && command -v wslpath >/dev/null 2>&1; then
    printf 'Build it where pnpm'\''s node_modules actually works — on Windows, in PowerShell:\n\n    cd %s\\web ; pnpm run build' \
      "$(wslpath -w "$(pwd)")"
  else
    printf 'Build it with:\n\n    pnpm --dir web install --frozen-lockfile && pnpm --dir web run build'
  fi
}

require_fresh_dist() {
  [ -f "$MANIFEST" ] || die \
"No frontend build found ($MANIFEST).

$(build_hint)

then run this script again."

  # Tailwind scans the maud templates, so a .rs change under public/ can change
  # public.css. Leaving that out would let a stylesheet ship one deploy behind
  # its markup, which shows up as a page that is subtly unstyled rather than
  # anything that looks like a build problem.
  local stale
  stale="$(find web/src web/package.json web/vite.config.ts web/vite.public.config.ts \
                crates/server/src/public \
                -type f -newer "$MANIFEST" -print -quit 2>/dev/null || true)"

  [ -z "$stale" ] || die \
"web/dist is older than $stale

$(build_hint)

Or set YOUWIN_SKIP_BUILD=1 to ship the assets exactly as they are."
}

if [ "${YOUWIN_SKIP_BUILD:-}" = 1 ]; then
  echo "==> Using web/dist as-is (YOUWIN_SKIP_BUILD=1)"
  [ -f "$MANIFEST" ] || die "No frontend build found ($MANIFEST)."
elif native_pnpm; then
  echo "==> Building frontends"
  pnpm --dir web install --frozen-lockfile
  pnpm --dir web run build
  [ -f "$MANIFEST" ] || die "$MANIFEST missing after build"
else
  echo "==> No native pnpm here; using the existing build in web/dist"
  require_fresh_dist
fi

echo "==> Checking $HOST is reachable"
ssh -o BatchMode=yes -o ConnectTimeout=10 "$HOST" true 2>/dev/null || die \
"Cannot reach '$HOST' over ssh without a password.

WSL keeps its own ~/.ssh, separate from Windows — a key under /mnt/c cannot be
used in place, because everything there reports mode 0777 and ssh refuses a
private key that permissive.

Either pass the host explicitly:

    ./deploy/deploy.sh user@server.example

or add it to ~/.ssh/config in WSL:

    Host youwin
      HostName server.example
      User youwin"

# ---------------------------------------------------------------------------
# Sync
#
# -a is deliberately not used. It implies -p, and every file on a /mnt/ drive
# reports 0777, so -a would publish a world-writable source tree to the server.
# -p WITH --chmod is the combination that rewrites modes on files already there
# from an earlier run, rather than only on new ones.
# ---------------------------------------------------------------------------
RSYNC=(rsync -rlptz --chmod=D755,F644)

echo "==> Syncing sources to $HOST"
# Cargo.lock ships; build output, dependencies, and anything local do not.
"${RSYNC[@]}" --delete \
  --exclude '/target' \
  --exclude 'node_modules' \
  --exclude '/web/dist' \
  --exclude '/web/dev-dist' \
  --exclude '/.git' \
  --exclude '/.claude' \
  --exclude '/.vscode' \
  --exclude '.env' \
  --exclude '*.db' \
  --exclude '*.db-wal' \
  --exclude '*.db-shm' \
  ./ "$HOST:$REMOTE_SRC/"

echo "==> Syncing built assets"
ssh "$HOST" "mkdir -p $WWW/public $WWW/write"
"${RSYNC[@]}" --delete web/dist/public/ "$HOST:$WWW/public/"
"${RSYNC[@]}" --delete web/dist/write/  "$HOST:$WWW/write/"
# Favicons and robots.txt live alongside the hashed assets; Caddy serves them
# from the same root.
"${RSYNC[@]}" static/ "$HOST:$WWW/public/"

echo "==> Building on $HOST"
# No DATABASE_URL and no cargo-sqlx: queries are runtime-checked, so the build
# needs nothing but a toolchain.
ssh "$HOST" "cd $REMOTE_SRC && cargo build --release --package youwin-server"

echo "==> Installing and restarting"
# Install to a temp path then move: mv is atomic within a filesystem, so a
# half-copied binary is never what systemd execs.
ssh "$HOST" "
  set -euo pipefail
  install -D -m 0755 $REMOTE_SRC/target/release/youwin-server $REMOTE_BIN.new
  mv $REMOTE_BIN.new $REMOTE_BIN
  sudo systemctl restart youwin
"

echo "==> Health check"
# Give systemd a moment to bring both listeners up before judging it.
sleep 2
ssh "$HOST" "curl -fsS http://127.0.0.1:8080/health && echo ' <- public' \
          && curl -fsS http://127.0.0.1:8081/api/health && echo ' <- authoring'"

echo "==> Done"
