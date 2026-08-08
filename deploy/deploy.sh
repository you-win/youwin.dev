#!/usr/bin/env bash
#
# Build the frontends locally, then build the binary on the server and restart.
#
# Cross-compiling from Windows is skipped deliberately: sqlx's sqlite feature
# bundles SQLite through libsqlite3-sys, so a cross build needs a cross C
# toolchain. The server already has cc; the first build there is slow and every
# one after is incremental.
#
# Usage:  ./deploy/deploy.sh [ssh-host]
# Needs:  pnpm locally; rust + cc on the server; ssh/rsync on both.

set -euo pipefail

HOST="${1:-${YOUWIN_HOST:-youwin}}"
REMOTE_SRC="/srv/youwin/src"
REMOTE_BIN="/srv/youwin/bin/youwin-server"
WWW="/var/www/youwin"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

echo "==> Building frontends"
pnpm --dir web install --frozen-lockfile
pnpm --dir web run build

# Fail early rather than shipping a tree the server cannot start from. The
# binary reads this manifest at boot to resolve the hashed stylesheet URL.
test -f web/dist/public/.vite/manifest.json \
  || { echo "web/dist/public/.vite/manifest.json missing after build" >&2; exit 1; }

echo "==> Syncing sources to $HOST"
# Cargo.lock ships; target/ and node_modules/ do not.
rsync -az --delete \
  --exclude '/target' \
  --exclude 'node_modules' \
  --exclude '/web/dist' \
  --exclude '/.git' \
  ./ "$HOST:$REMOTE_SRC/"

echo "==> Syncing built assets"
ssh "$HOST" "mkdir -p $WWW/public $WWW/write"
rsync -az --delete web/dist/public/ "$HOST:$WWW/public/"
rsync -az --delete web/dist/write/  "$HOST:$WWW/write/"
# Favicons and robots.txt live alongside the hashed assets; Caddy serves them
# from the same root.
rsync -az static/ "$HOST:$WWW/public/"

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
