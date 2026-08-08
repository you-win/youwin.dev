#!/usr/bin/env bash
#
# Manual deploy — the same release, assembled the same way, activated by the
# same script CI uses. This exists for the first deploy (before CI has a key on
# the box) and for the times you need to ship without pushing.
#
# The normal path is .github/workflows/deploy.yml. If you find yourself running
# this often, something about CI is broken and worth fixing instead.
#
# Usage:  ./deploy/deploy.sh [ssh-target]        default: deploy@$DEPLOY_HOST
# Needs:  a Linux host with cargo + cc, rsync, ssh. WSL2 Debian qualifies.

set -euo pipefail

TARGET="${1:-deploy@${DEPLOY_HOST:?set DEPLOY_HOST or pass user@host}}"
MANIFEST="web/dist/public/.vite/manifest.json"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

die() { printf '\n%s\n' "$*" >&2; exit 1; }

# A Windows checkout and a Linux build cannot share ./target — each would
# invalidate the other's artifacts on every switch. Somewhere outside the tree
# also means the build does not crawl over /mnt/c, which is the slow part.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${TMPDIR:-/tmp}/youwin-target}"

# ---------------------------------------------------------------------------
# Frontends
#
# web/node_modules holds native binaries for exactly one platform, so on a
# Windows checkout these are built in PowerShell and this script only checks
# the result is current. See deploy/README.md.
# ---------------------------------------------------------------------------

build_hint() {
  if [ -n "${WSL_DISTRO_NAME:-}" ] && command -v wslpath >/dev/null 2>&1; then
    printf 'Build it on Windows, in PowerShell:\n\n    cd %s\\web ; pnpm run build' \
      "$(wslpath -w "$(pwd)")"
  else
    printf 'Build it with:\n\n    pnpm --dir web install --frozen-lockfile && pnpm --dir web run build'
  fi
}

native_pnpm() {
  local found
  found="$(command -v pnpm 2>/dev/null)" || return 1
  # Under WSL, interop puts the Windows pnpm on PATH. Finding it is not the same
  # as being able to use it.
  case "$found" in /mnt/*) return 1 ;; esac
  pnpm --version >/dev/null 2>&1
}

if native_pnpm; then
  echo "==> Building frontends"
  pnpm --dir web install --frozen-lockfile
  pnpm --dir web run build
else
  echo "==> No native pnpm here; using the existing build in web/dist"
  [ -f "$MANIFEST" ] || die "No frontend build found ($MANIFEST).

$(build_hint)"

  # Tailwind scans the maud templates, so a .rs change under public/ can change
  # public.css. Without this, a stylesheet can ship one deploy behind its
  # markup — which looks like a styling bug, not a build one.
  stale="$(find web/src web/package.json web/vite.config.ts web/vite.public.config.ts \
                crates/server/src/public \
                -type f -newer "$MANIFEST" -print -quit 2>/dev/null || true)"
  [ -z "$stale" ] || die "web/dist is older than $stale

$(build_hint)"
fi

[ -s "$MANIFEST" ] || die "$MANIFEST missing after build"

# ---------------------------------------------------------------------------
# Binary
# ---------------------------------------------------------------------------

REL="$(date -u +%Y%m%d-%H%M%S)-$(git rev-parse --short=7 HEAD 2>/dev/null || echo manual)"

echo "==> Building youwin-server (target dir: $CARGO_TARGET_DIR)"
YOUWIN_BUILD="$REL" cargo build --release --locked --package youwin-server

# ---------------------------------------------------------------------------
# Assemble and ship — byte-for-byte the layout the workflow builds
# ---------------------------------------------------------------------------

staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT

mkdir -p "$staging/bin"
cp "$CARGO_TARGET_DIR/release/youwin-server" "$staging/bin/"
cp -r web/dist/public "$staging/public"
cp -r web/dist/write  "$staging/write"
cp -r static/. "$staging/public/"
cp -r deploy "$staging/deploy"

# Explicit modes, because every file on a /mnt/ drive reports 0777 and rsync
# would otherwise publish a world-writable tree.
find "$staging" -type d -exec chmod 755 {} +
find "$staging" -type f -exec chmod 644 {} +
chmod 755 "$staging/bin/youwin-server" "$staging"/deploy/*.sh "$staging/deploy/activate-youwin"

echo "==> Uploading release $REL to $TARGET"
rsync -rlptDz --delete "$staging/" "$TARGET:/srv/sites/youwin.dev/releases/$REL/"

echo "==> Activating"
ssh "$TARGET" "sudo /usr/local/bin/activate-youwin $REL"

echo "==> Done: $REL"
