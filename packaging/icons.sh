#!/usr/bin/env bash
# Generate every platform icon from the owner-supplied Prism PNG.
# Requires macOS sips/iconutil; outputs are committed, so packaging needs neither.
# packaging/icons.sh [--png] (--png remains accepted for existing build scripts).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$ROOT/assets/app-icon"
SOURCE="$DIR/prism-1024.png"
if [[ "$#" != 0 && !( "$#" == 1 && "$1" == "--png" ) ]]; then
  echo "usage: packaging/icons.sh [--png]" >&2
  exit 1
fi
command -v sips >/dev/null || { echo "error: sips not found (macOS required)" >&2; exit 1; }
command -v iconutil >/dev/null || { echo "error: iconutil not found (macOS required)" >&2; exit 1; }
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
# Keep the PNG artwork, alpha and existing margins; do not add padding again.
SET="$TMP/prism.iconset"
mkdir -p "$SET"
for s in 16 32 128 256 512; do
  sips -z "$s" "$s" "$SOURCE" --out "$SET/icon_${s}x${s}.png" >/dev/null
  sips -z "$((s * 2))" "$((s * 2))" "$SOURCE" --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns -o "$TMP/prism.icns" "$SET"
ICO_PNGS=()
for s in 16 20 24 32 40 48 64 128 256; do
  sips -z "$s" "$s" "$SOURCE" --out "$TMP/ico-$s.png" >/dev/null
  ICO_PNGS+=("$TMP/ico-$s.png")
done
(cd "$ROOT" && cargo run --profile fast --locked -q -p xtask -- ico "$TMP/prism.ico" "${ICO_PNGS[@]}")
for s in 16 24 32 48 64 128 256 512; do
  mkdir -p "$DIR/hicolor/${s}x${s}/apps"
  sips -z "$s" "$s" "$SOURCE" --out "$DIR/hicolor/${s}x${s}/apps/com.nkstudio.prism.png" >/dev/null
done
# The scalable desktop icon is self-contained, using the same artwork as the PNGs.
python3 - "$SOURCE" "$DIR" <<'PYICON'
import base64
import pathlib
import sys
source, directory = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
encoded = base64.b64encode(source.read_bytes()).decode("ascii")
svg = ('<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" '
       'width="1024" height="1024" viewBox="0 0 1024 1024">'
       '<title>Prism — NK Studio</title><image width="1024" height="1024" '
       f'xlink:href="data:image/png;base64,{encoded}"/></svg>\n')
for relative in ("prism.svg", "prism-small.svg", "hicolor/scalable/apps/com.nkstudio.prism.svg"):
    output = directory / relative
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(svg, encoding="utf-8")
PYICON
cp "$TMP/prism.icns" "$DIR/prism.icns"
cp "$TMP/prism.ico" "$DIR/prism.ico"
echo "macOS, Windows and FreeBSD icons written from $SOURCE"
