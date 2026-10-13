#!/usr/bin/env bash
# Publish a built version to the in-app update feed (https://prism-updates.vercel.app), locally.
# The local twin of .github/workflows/publish-update.yml.
#
# Usage: packaging/publish-update.sh [--version X.Y.Z] [--min-supported X.Y.Z] [--dry-run]
#
# 1. Clones (or updates) the site repository next to this one (../prism-updates).
# 2. cargo xtask update publish: copies dist/release/prism-<v>-macos-*.app.zip and
#    prism-<v>-windows-*.msi to releases/<v>/, writes and signs stable/latest.json, keeps the
#    newest three versions.
# 3. Commits the site as a single commit and pushes it over the old one (so old packages don't
#    pile up in git history). Vercel deploys on push.
# 4. Waits for the live feed to report the new version.
#
# Env:
#   PRISM_UPDATE_SECRET_KEY_FILE  secret key (default ~/prism-update.key)
#   PRISM_UPDATE_SITE_DIR         site checkout (default ../prism-updates)
#   PRISM_UPDATE_SITE_REPO        default NK-Studio/prism-updates
#   PRISM_UPDATE_BASE_URL         default https://prism-updates.vercel.app
#   PRISM_UPDATE_GH_USER          gh account used to push (default NK-Studio; empty: plain git auth)
set -euo pipefail
# shellcheck source=env.sh
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"

MIN_SUPPORTED=""
DRY_RUN=0
while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --min-supported) MIN_SUPPORTED="$2"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h | --help) sed -n '2,22p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

KEY="${PRISM_UPDATE_SECRET_KEY_FILE:-$HOME/prism-update.key}"
SITE="${PRISM_UPDATE_SITE_DIR:-$(cd "$ROOT/.." && pwd)/prism-updates}"
REPO="${PRISM_UPDATE_SITE_REPO:-NK-Studio/prism-updates}"
BASE="${PRISM_UPDATE_BASE_URL:-https://prism-updates.vercel.app}"
GH_USER="${PRISM_UPDATE_GH_USER-NK-Studio}"

[ -f "$KEY" ] || { echo "error: no secret key at $KEY (cargo xtask update keygen)" >&2; exit 1; }
command -v cargo >/dev/null || PATH="$HOME/.cargo/bin:$PATH"

# git with the chosen gh account's token, for this command only (no git config changes).
site_git() {
  if [ -n "$GH_USER" ] && command -v gh >/dev/null && TOK="$(gh auth token --user "$GH_USER" 2>/dev/null)"; then
    TOK="$TOK" git -C "$SITE" -c credential.helper= \
      -c 'credential.helper=!f(){ echo username=x-access-token; echo "password=$TOK"; }; f' "$@"
  else
    git -C "$SITE" "$@"
  fi
}

echo "==> Prism $VERSION → $BASE (site: $SITE)"
if [ ! -d "$SITE/.git" ]; then
  mkdir -p "$SITE"
  git -C "$SITE" init -q
  git -C "$SITE" remote add origin "https://github.com/$REPO.git"
fi
site_git fetch -q origin
BRANCH="$(site_git ls-remote --symref origin HEAD 2>/dev/null | sed -n 's|^ref: refs/heads/\([^[:space:]]*\).*|\1|p')"
BRANCH="${BRANCH:-main}"
# Start from exactly what is deployed (old version folders, vercel.json, index.html).
git -C "$SITE" checkout -q -B "$BRANCH" "origin/$BRANCH"
git -C "$SITE" reset -q --hard "origin/$BRANCH"
git -C "$SITE" clean -qfdx

args=(--site "$SITE" --base-url "$BASE" --dist "$DIST" --version "$VERSION" --keep 3 --secret "$KEY")
[ -n "$MIN_SUPPORTED" ] && args+=(--min-supported "$MIN_SUPPORTED")
(cd "$ROOT" && cargo xtask update publish "${args[@]}")

echo "==> feed:"
cat "$SITE/stable/latest.json"
if [ "$DRY_RUN" = 1 ]; then
  echo "==> dry run: not pushed ($SITE has the result)"
  exit 0
fi

# One commit replacing the previous one.
git -C "$SITE" checkout -q --orphan publish
git -C "$SITE" add -A
git -C "$SITE" -c user.name="Prism release" -c user.email="release@prism.invalid" commit -q -m "Prism $VERSION"
site_git push -q --force origin "publish:$BRANCH"
git -C "$SITE" checkout -q -B "$BRANCH" publish
git -C "$SITE" branch -q -D publish

echo "==> pushed; waiting for Vercel to serve $VERSION"
for _ in $(seq 1 60); do
  live="$(curl -fsS -H 'Cache-Control: no-cache' "$BASE/stable/latest.json" 2>/dev/null | sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' | head -1 || true)"
  if [ "$live" = "$VERSION" ]; then
    echo "==> live: $BASE/stable/latest.json reports $VERSION"
    exit 0
  fi
  sleep 5
done
echo "warning: the live feed doesn't report $VERSION yet (last seen: ${live:-nothing}); check the Vercel dashboard" >&2
exit 1
