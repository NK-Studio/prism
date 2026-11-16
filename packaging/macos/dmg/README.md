# macOS DMG window

What Finder shows when the DMG opens: a 660 × 400 pt window with Prism.app on the left, the
`Applications` link on the right, a dotted arrow between them and the drag-to-install line under
them in Korean and English, on a white field that darkens slightly towards the bottom. The look
follows Ramus's DMG (NK Studio's other app), so the two install the same way.

`package.sh` copies these files into the image; nothing here is generated at build time, so the
DMG still builds with `hdiutil makehybrid` (no mounted device, no Finder scripting, nothing extra
installed in the signing job). The mounted disk also carries the app icon
(`assets/app-icon/prism.icns` as `.VolumeIcon.icns`, see `package.sh`).

| File | What |
|---|---|
| `make-background.swift` | Writes `background.svg`: the gradient, the arrow and the two lines of text. macOS only, with Pretendard installed. |
| `background.svg` | Source of the background. Its text (Pretendard Medium 15 pt and Regular 12 pt, SIL OFL 1.1) is outlined, so rendering it needs no fonts. |
| `background.tiff` | The background at 1x (660 × 400 px, 72 dpi) and 2x (1320 × 800 px, 144 dpi) in one HiDPI TIFF (lossless RGB, Deflate, sRGB). Goes to `.background/background.tiff`. |
| `dmg-layout.DS_Store` | Finder's view settings for the volume: window size, icon size 128, label size 13, Prism.app at (170, 190), `Applications` at (490, 190), and the background. Goes to `.DS_Store` in the image; named so it isn't mistaken for (or ignored like) a Finder-generated `.DS_Store`. |
| `generate.py` | Writes `background.tiff` and `dmg-layout.DS_Store` from the SVG and the layout above. |

## The volume name has no version

The mounted volume is called `Prism`, not `Prism <version>`. The layout points at the background
through an alias that includes the volume name, and Finder resolves it by that name: with a
versioned name the window keeps its size and icon positions but shows no background (tested).
The DMG file name (`prism-<version>-macos-<arch>.dmg`) still carries the version.

## Rules

- **Finder draws the icon labels in black in light and dark mode** when a window has a background,
  so the whole field stays light.
- **Nothing goes inside the icon boxes:** the arrow runs from x = 268 to 394 at y = 190, clear of
  both 128 pt icon boxes (106–234 and 426–554), and the text sits under the label strip.
- **The arrow and the icons share one line:** `arrowY` in `make-background.swift` and the icons'
  y in `generate.py` are both 190.

## Regenerate

To change the artwork or the wording, edit `make-background.swift` and run it on a Mac with
[Pretendard](https://github.com/orioncactus/pretendard) installed (it stops if a glyph would
come from another font):

```sh
swift packaging/macos/dmg/make-background.swift
```

Then (or after changing only the layout constants in `generate.py`) run, on any OS:

```sh
pip install 'pillow>=12' ds_store==1.3.3 mac_alias==2.2.3
python3 packaging/macos/dmg/generate.py   # needs resvg on PATH
```

`generate.py`'s two outputs are byte-for-byte reproducible and carry nothing from the machine
that ran it: the SVG renders without any fonts, and `dmg-layout.DS_Store` is written from scratch,
its background alias holding only the volume name and `/.background/background.tiff`. The window
is 660 × 432 with Finder's 32 pt title bar; the content area is 660 × 400.

To look at the result without a release build, stage a folder as `package.sh` does (app,
`Applications` link, `.background/background.tiff`, `.DS_Store`), run its two `hdiutil` lines and
open the image. No other volume named `Prism` may be mounted: Finder would look for the
background on that one.

The build pipeline follows VectorCraft's `packaging/macos/dmg/` (storytold/vectorcraft#493); the
look follows Ramus's `scripts/make-dmg.sh` and `scripts/make-dmg-background.swift`.
