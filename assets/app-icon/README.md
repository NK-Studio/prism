# Prism app icon

`prism-1024.png` is the owner-supplied 1024 × 1024 RGBA Prism logo and
the canonical artwork for all platforms. The ICNS, ICO, hicolor PNGs and
self-contained SVG wrappers are derived from it, preserving alpha and margins.

Regenerate on macOS with `packaging/icons.sh` (or `packaging/icons.sh --png`).
The script requires `sips`, `iconutil`, Python 3 and Cargo. No font files are used.
All outputs are committed, so native packaging uses them directly.

The ICO contains 16, 20, 24, 32, 40, 48, 64, 128 and 256 px images. The ICNS
contains standard 1×/2× representations up to 1024 px. FreeBSD desktop icons
include 16, 24, 32, 48, 64, 128, 256 and 512 px PNGs and a scalable SVG.

The original copyright and license are retained in `LICENSE.txt`; asset
provenance is listed in the root `ATTRIBUTION.md`.
