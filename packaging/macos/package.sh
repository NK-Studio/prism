#!/usr/bin/env bash
# Build, sign and (optionally) notarize the macOS release artifacts:
#
#   $DIST/prism-<version>-macos-<arch>.dmg          Prism.app on a drag-to-Applications DMG
#   $DIST/prism-cli-<version>-macos-<arch>.zip      the headless CLI
#   $DIST/prism-<version>-macos-<arch>.app.zip      Prism.app for in-app updates (crates/updater)
#
# Usage: packaging/macos/package.sh [--arch universal|aarch64|x86_64] [--skip-build]
#
# Signing (env):
#   MACOS_SIGN_IDENTITY   codesign identity (name or SHA-1). Default "-" = ad-hoc (local testing;
#                         Gatekeeper will reject the result on other Macs). In CI, import-cert.sh sets it.
#   MACOS_KEYCHAIN        keychain holding the identity (optional)
# Notarization (env; one complete set, and a real identity; see notary.sh):
#   APPLE_API_KEY, APPLE_API_KEY_ID, APPLE_API_ISSUER (App Store Connect API key), or
#   APPLE_ID, APPLE_PASSWORD (app-specific password), APPLE_TEAM_ID
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/macos"
# shellcheck source=notary.sh
. "$HERE/notary.sh"

ARCH=universal
SKIP_BUILD=0
while [ $# -gt 0 ]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,17p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$ARCH" in
  universal) TARGETS=(aarch64-apple-darwin x86_64-apple-darwin) ;;
  aarch64) TARGETS=(aarch64-apple-darwin) ;;
  x86_64) TARGETS=(x86_64-apple-darwin) ;;
  *) echo "unknown --arch $ARCH" >&2; exit 2 ;;
esac

# Keep in sync with LSMinimumSystemVersion in Info.plist.in.
export MACOSX_DEPLOYMENT_TARGET=11.0
IDENTITY="${MACOS_SIGN_IDENTITY:--}"
SHORT_VERSION="${VERSION%%-*}"
WORK="$CARGO_TARGET_DIR/macos-package"
APP="$WORK/Prism.app"
DMG="$DIST/prism-$VERSION-macos-$ARCH.dmg"
CLI_ZIP="$DIST/prism-cli-$VERSION-macos-$ARCH.zip"
UPDATE_ZIP="$DIST/prism-$VERSION-macos-$ARCH.app.zip"

NOTARIZE=0
if [ "$IDENTITY" = "-" ]; then
  warn "macOS: ad-hoc signing (no MACOS_SIGN_IDENTITY); artifacts are not notarized"
elif [ -n "$NOTARY_KIND" ]; then
  NOTARIZE=1
  notary_auth
else
  warn "macOS: neither APPLE_API_KEY / APPLE_API_KEY_ID / APPLE_API_ISSUER nor APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID complete; signed but not notarized"
fi

echo "==> Prism $VERSION for macOS ($ARCH), identity: $IDENTITY, notarize: $NOTARIZE"

# ---- build -------------------------------------------------------------------------------------
if [ "$SKIP_BUILD" = 0 ]; then
  args=()
  for t in "${TARGETS[@]}"; do args+=(--target "$t"); done
  (cd "$ROOT" && cargo build --release --locked -p prism -p prism-cli --features heif "${args[@]}")
fi

rm -rf "$WORK"
mkdir -p "$WORK/bin"
for bin in prism prism-cli; do
  inputs=()
  for t in "${TARGETS[@]}"; do inputs+=("$CARGO_TARGET_DIR/$t/release/$bin"); done
  lipo -create -output "$WORK/bin/$bin" "${inputs[@]}"
  lipo -info "$WORK/bin/$bin"
done

# ---- signing helpers ---------------------------------------------------------------------------
sign() {
  # Hardened runtime + secure timestamp for a real identity; ad-hoc can't be timestamped.
  local ts=(--timestamp)
  [ "$IDENTITY" = "-" ] && ts=(--timestamp=none)
  local kc=()
  [ -n "${MACOS_KEYCHAIN:-}" ] && kc=(--keychain "$MACOS_KEYCHAIN")
  codesign --force --sign "$IDENTITY" ${kc[@]+"${kc[@]}"} "${ts[@]}" "$@"
}

notarize() {
  local file="$1" out id status
  echo "==> notarizing $(basename "$file") (this can take a few minutes)"
  out="$(xcrun notarytool submit "$file" "${NOTARY_AUTH[@]}" \
    --wait --timeout 1h --output-format json)" || true
  echo "$out"
  id="$(printf '%s' "$out" | plutil -extract id raw -o - - 2>/dev/null || true)"
  status="$(printf '%s' "$out" | plutil -extract status raw -o - - 2>/dev/null || true)"
  if [ "$status" != "Accepted" ]; then
    if [ -n "$id" ]; then
      xcrun notarytool log "$id" "${NOTARY_AUTH[@]}" || true
    fi
    echo "error: notarization of $(basename "$file") failed (status: ${status:-unknown})" >&2
    exit 1
  fi
}

# ---- Prism.app ----------------------------------------------------------------------------
echo "==> assembling $APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
# Executable and icon carry the display name (CFBundleExecutable / CFBundleIconFile).
cp "$WORK/bin/prism" "$APP/Contents/MacOS/Prism"
cp "$ROOT/assets/app-icon/prism.icns" "$APP/Contents/Resources/Prism.icns"
# Licences of the embedded craft-fonts fonts (only when built with CRAFT_FONTS_DIR).
if [ -n "${CRAFT_FONTS_DIR:-}" ]; then
  mkdir -p "$APP/Contents/Resources/Licenses"
  copy_font_licences "$APP/Contents/Resources/Licenses"
fi
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$SHORT_VERSION/g" \
  -e "s/@BUILD_SHA@/${PRISM_BUILD_SHA:-unknown}/g" \
  "$HERE/Info.plist.in" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"

# Sign inside-out: nested code first, then the bundle itself (no --deep on the final signature).
# Today the only nested code is the main executable; frameworks/helpers would be signed here too.
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP/Contents/MacOS/Prism"
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"

if [ "$NOTARIZE" = 1 ]; then
  ditto -c -k --keepParent "$APP" "$WORK/Prism-notarize.zip"
  notarize "$WORK/Prism-notarize.zip"
  xcrun stapler staple "$APP"
  xcrun stapler validate "$APP"
  spctl --assess --type execute -vvv "$APP"
fi

# ---- update package ----------------------------------------------------------------------------
# The signed (and, in CI, notarized and stapled) bundle, zipped with ditto so signatures and
# extended attributes survive. The in-app updater unpacks it with ditto and swaps the whole bundle.
echo "==> building $UPDATE_ZIP"
rm -f "$UPDATE_ZIP"
ditto -c -k --keepParent "$APP" "$UPDATE_ZIP"

# ---- DMG ---------------------------------------------------------------------------------------
echo "==> building $DMG"
STAGE="$WORK/dmg"
mkdir -p "$STAGE"
ditto "$APP" "$STAGE/Prism.app"
ln -s /Applications "$STAGE/Applications"
# Finder window layout: background, icon size and positions (packaging/macos/dmg/README.md).
mkdir -p "$STAGE/.background"
cp "$HERE/dmg/background.tiff" "$STAGE/.background/background.tiff"
cp "$HERE/dmg/dmg-layout.DS_Store" "$STAGE/.DS_Store"
rm -f "$DMG" "$WORK/raw.dmg"
# makehybrid + convert builds the image without attaching a device, unlike `create -srcfolder`,
# which is flaky on CI runners ("Resource busy") and hangs in sandboxed sessions.
# The volume name has no version: .DS_Store finds the background through an alias that includes it.
hdiutil makehybrid -hfs -hfs-volume-name "Prism" -hfs-openfolder "$STAGE" -o "$WORK/raw.dmg" "$STAGE"
hdiutil convert "$WORK/raw.dmg" -format UDZO -imagekey zlib-level=9 -o "$DMG"
rm -f "$WORK/raw.dmg"
sign "$DMG"
codesign --verify --strict --verbose=2 "$DMG"
if [ "$NOTARIZE" = 1 ]; then
  notarize "$DMG"
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
  spctl --assess --type open --context context:primary-signature -vvv "$DMG"
fi

# ---- CLI ---------------------------------------------------------------------------------------
echo "==> building $CLI_ZIP"
CLI_DIR="$WORK/prism-cli-$VERSION-macos-$ARCH"
mkdir -p "$CLI_DIR"
cp "$WORK/bin/prism-cli" "$CLI_DIR/"
copy_docs "$CLI_DIR"
# Same Developer ID and hardened runtime as the app, with an explicit reverse-DNS identifier
# (codesign would otherwise use the bare file name).
sign --options runtime --identifier com.nkstudio.prism-cli "$CLI_DIR/prism-cli"
codesign --verify --strict --verbose=2 "$CLI_DIR/prism-cli"
rm -f "$CLI_ZIP"
ditto -c -k --keepParent "$CLI_DIR" "$CLI_ZIP"
# notarytool accepts a zip of the bare binary. A bare Mach-O can't carry a stapled ticket, so
# Gatekeeper looks the notarization up online when a quarantined copy first runs.
if [ "$NOTARIZE" = 1 ]; then notarize "$CLI_ZIP"; fi

"$WORK/bin/prism-cli" --version
echo "==> done (check the artifacts as shipped with packaging/macos/verify.sh --arch $ARCH)"
ls -lh "$DMG" "$CLI_ZIP" "$UPDATE_ZIP"
