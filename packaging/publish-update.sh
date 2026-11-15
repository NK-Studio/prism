#!/usr/bin/env bash
# Publish a built version to the in-app update feed, locally.
# The local twin of .github/workflows/publish-update.yml.
#
# Usage: packaging/publish-update.sh [--version X.Y.Z] [--min-supported X.Y.Z] [--dry-run]
#
# The feed site (Cloudflare Workers, https://prism-updates.a62201631.workers.dev) serves only the
# signed feed; the update packages and the installers are assets of the version's public GitHub
# Release (Cloudflare's static assets stop at 25 MiB per file). Steps:
#
# 1. Clones (or updates) the site repository next to this one (../prism-updates).
# 2. cargo xtask update publish: writes and signs stable/latest.json for
#    dist/release/prism-<v>-macos-*.app.zip and prism-<v>-windows-*.msi, pointing at
#    https://github.com/NK-Studio/prism/releases/download/v<v>/.
# 3. Uploads those packages and the first-install DMG (dist/release/prism-<v>-macos-*.dmg) to the
#    GitHub Release v<v> (creating it from the pushed tag if needed, with
#    packaging/release/notes.md as its notes) and checks each one downloads. The feed's download
#    link, for copies that can't update in place, is that release page.
# 4. Commits the site as a single commit and pushes it over the old one. Cloudflare Workers Builds
#    deploys on push.
# 5. Waits for the live feed to report the new version.
#
# Env:
#   PRISM_UPDATE_SECRET_KEY_FILE  secret key (default ~/prism-update.key)
#   PRISM_UPDATE_SITE_DIR         site checkout (default ../prism-updates)
#   PRISM_UPDATE_SITE_REPO        default NK-Studio/prism-updates
#   PRISM_UPDATE_BASE_URL         default https://prism-updates.a62201631.workers.dev
#   PRISM_RELEASE_REPO            repository whose release holds the packages (default NK-Studio/prism)
#   PRISM_UPDATE_GH_USER          gh account used to push and upload (default NK-Studio; empty: plain auth)
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
    -h | --help) sed -n '2,29p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

KEY="${PRISM_UPDATE_SECRET_KEY_FILE:-$HOME/prism-update.key}"
SITE="${PRISM_UPDATE_SITE_DIR:-$(cd "$ROOT/.." && pwd)/prism-updates}"
REPO="${PRISM_UPDATE_SITE_REPO:-NK-Studio/prism-updates}"
BASE="${PRISM_UPDATE_BASE_URL:-https://prism-updates.a62201631.workers.dev}"
RELEASE_REPO="${PRISM_RELEASE_REPO:-NK-Studio/prism}"
GH_USER="${PRISM_UPDATE_GH_USER-NK-Studio}"
TAG="v$VERSION"
PACKAGES_URL="https://github.com/$RELEASE_REPO/releases/download/$TAG"

[ -f "$KEY" ] || { echo "error: no secret key at $KEY (cargo xtask update keygen)" >&2; exit 1; }
command -v cargo >/dev/null || PATH="$HOME/.cargo/bin:$PATH"

shopt -s nullglob
PACKAGES=("$DIST/prism-$VERSION"-macos-*.app.zip "$DIST/prism-$VERSION"-windows-*.msi)
INSTALLERS=("$DIST/prism-$VERSION"-macos-*.dmg)
shopt -u nullglob
[ ${#PACKAGES[@]} -gt 0 ] || { echo "error: no update packages for $VERSION in $DIST" >&2; exit 1; }
[ ${#INSTALLERS[@]} -gt 0 ] ||
  warn "no prism-$VERSION-macos-*.dmg in $DIST: new Mac users will have nothing to install from the release"
# Everything that goes on the release: the update packages (also the Windows installers) and the DMG.
UPLOADS=("${PACKAGES[@]}" ${INSTALLERS[@]+"${INSTALLERS[@]}"})

# The chosen gh account's token, for this command only (no git or gh config changes).
TOK=""
if [ -n "$GH_USER" ] && command -v gh >/dev/null; then
  TOK="$(gh auth token --user "$GH_USER" 2>/dev/null || true)"
fi
site_git() {
  if [ -n "$TOK" ]; then
    TOK="$TOK" git -C "$SITE" -c credential.helper= \
      -c 'credential.helper=!f(){ echo username=x-access-token; echo "password=$TOK"; }; f' "$@"
  else
    git -C "$SITE" "$@"
  fi
}
gh_as() {
  if [ -n "$TOK" ]; then GH_TOKEN="$TOK" gh "$@"; else gh "$@"; fi
}

echo "==> Prism $VERSION -> $BASE (site: $SITE, packages: $PACKAGES_URL)"
if [ ! -d "$SITE/.git" ]; then
  mkdir -p "$SITE"
  git -C "$SITE" init -q
  git -C "$SITE" remote add origin "https://github.com/$REPO.git"
fi
site_git fetch -q origin
BRANCH="$(site_git ls-remote --symref origin HEAD 2>/dev/null | sed -n 's|^ref: refs/heads/\([^[:space:]]*\).*|\1|p')"
BRANCH="${BRANCH:-main}"
# Start from exactly what is deployed (wrangler.jsonc, _headers, .assetsignore, index.html).
git -C "$SITE" checkout -q -B "$BRANCH" "origin/$BRANCH"
git -C "$SITE" reset -q --hard "origin/$BRANCH"
git -C "$SITE" clean -qfdx

args=(--site "$SITE" --packages-url "$PACKAGES_URL" --dist "$DIST" --version "$VERSION" --secret "$KEY"
      --notes-url "https://github.com/$RELEASE_REPO/releases/tag/$TAG"
      --download-url "https://github.com/$RELEASE_REPO/releases/tag/$TAG")
[ -n "$MIN_SUPPORTED" ] && args+=(--min-supported "$MIN_SUPPORTED")
(cd "$ROOT" && cargo xtask update publish "${args[@]}")

echo "==> feed:"
cat "$SITE/stable/latest.json"
if [ "$DRY_RUN" = 1 ]; then
  echo "==> dry run: nothing uploaded or pushed ($SITE has the result); would upload:"
  printf '    %s\n' "${UPLOADS[@]}"
  exit 0
fi

# The packages go up before the feed that points at them.
command -v gh >/dev/null || { echo "error: gh is needed to upload the packages to the GitHub Release" >&2; exit 1; }
if ! gh_as release view "$TAG" --repo "$RELEASE_REPO" >/dev/null 2>&1; then
  echo "==> creating the GitHub Release $TAG"
  gh_as release create "$TAG" --repo "$RELEASE_REPO" --verify-tag --title "Prism $VERSION" \
    --notes "$(sed "s/{version}/$VERSION/g" "$ROOT/packaging/release/notes.md")"
fi
echo "==> uploading ${#UPLOADS[@]} file(s) to $RELEASE_REPO $TAG"
gh_as release upload "$TAG" --repo "$RELEASE_REPO" --clobber "${UPLOADS[@]}"
for p in "${UPLOADS[@]}"; do
  url="$PACKAGES_URL/$(basename "$p")"
  curl -fsSIL -o /dev/null "$url" || { echo "error: $url doesn't download (is the release still a draft?)" >&2; exit 1; }
  echo "    ok: $url"
done

# One commit replacing the previous one.
git -C "$SITE" checkout -q --orphan publish
git -C "$SITE" add -A
git -C "$SITE" -c user.name="Prism release" -c user.email="release@prism.invalid" commit -q -m "Prism $VERSION"
site_git push -q --force origin "publish:$BRANCH"
git -C "$SITE" checkout -q -B "$BRANCH" publish
git -C "$SITE" branch -q -D publish

echo "==> pushed; waiting for Cloudflare to serve $VERSION"
live=""
for _ in $(seq 1 60); do
  live="$(curl -fsS -H 'Cache-Control: no-cache' "$BASE/stable/latest.json" 2>/dev/null | sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' | head -1 || true)"
  if [ "$live" = "$VERSION" ]; then
    echo "==> live: $BASE/stable/latest.json reports $VERSION"
    exit 0
  fi
  sleep 5
done
echo "warning: the live feed doesn't report $VERSION yet (last seen: ${live:-nothing}); check the prism-updates Worker's builds in the Cloudflare dashboard" >&2
exit 1
