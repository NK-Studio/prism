# Filter menu investigation

Investigation date: 2026-11-16. This report separates what was read from the
binary, what was inferred and what is still unknown, filter by filter. It is not
a claim of Photoshop source recovery or pixel equivalence.

## Authorization and implementation boundary

The maintainer explicitly authorized using a read-only local analysis of their
installed Photoshop 2026 for the Filter menu (Noise, Render, Video, Sharpen,
Stylize, Distort, Pixelate, Blur, Blur Gallery, Other). This is a narrowly scoped
exception to the usual behaviour-only investigation rule, like the brush and SUT
investigations (`docs/pen-input-analysis.md`, `docs/sut-analysis.md`). It does
not authorize copying proprietary implementation, shaders, profiles, assets or
decompiler output into Prism. Prism's filters are independent Rust written from
the formulas and constants recorded in the maintainer's research notes; no
binary, pseudocode or analysis database belongs in source control.

Adjustments (Image › Adjustments) and Content-Aware Fill are out of scope for this
pass.

## Target identity

| Item | Observed value |
| --- | --- |
| Application | Adobe Photoshop 2026, macOS arm64 |
| Version | `27.4.0` (`CFBundleShortVersionString`) |
| Filter plug-in | `Contents/PlugIns/Required/Filters/Standard Multiplugin.plugin` (symbols present) |
| Core maths | `AdbePM.framework` (symbols present) |
| Names | Korean string table `tw10428_Photoshop_ko_KR.dat`, scripting dictionary `Photoshop.sdef` |

The research notes (outside this repository) list each function address and
table address they were read from. This document keeps only what Prism needs.

## How Prism decides what to ship

Each filter falls in one of three groups in the research:

- **Read**: formula and constants found in the binary.
- **Approximate**: names, options, ranges and defaults confirmed; the formula is a
  close guess.
- **Catalogued**: names and options only.

For Approximate and Catalogued filters Prism already had an implementation. The
maintainer chose to keep whichever is closer to Photoshop's real output: the same
test image is filtered in Photoshop and in Prism (existing and research variants)
and the lower error wins. The measurements are recorded per filter below.

Render and Video have no research formulas; their dialogs are matched to
Photoshop's (structure, options, defaults), and Prism's existing algorithms stay.

## Method

A synthetic 384×256 8-bit RGB test image (grey ramp, colour bars, 8 px checkerboard, a soft disc,
seeded noise, thin lines) is filtered by Photoshop 27.4 through ExtendScript (`app.open`,
`ArtLayer.apply…` or `executeAction`, saved as PNG), by Prism's `prism-cli run`, and by the
research C++ formulas. "MAE" is the mean absolute difference from Photoshop in 8-bit levels over
RGB; "max" the largest. Linear filters were also fitted by least squares (a kernel over a 5×5 or
7×7 window), which recovers Photoshop's kernel exactly when the filter is linear. Dialogs were
captured from Photoshop's own menus and compared with Prism's offscreen snapshots.

## Dialog layout

Photoshop's one-panel filters share one dialog: title centred over a hairline; a preview at real
pixels in the top left with − / zoom / + under it (drag to pan); OK, Cancel (Alt: Reset) and Preview
in a right-hand column; each number as "Label: [value] unit" over a slider; few-choice options as a
framed group of radio buttons (Add Noise › Distribution). Prism draws it in `ui-egui/src/filter_ps_ui.rs`
for every `filter.*` command whose options are numbers, choices and checkboxes. The in-dialog
preview runs the engine's pixel function on the visible area only.

## Per-filter record

### Noise

| Filter | Before (MAE / max) | Now | What Photoshop does |
| --- | --- | --- | --- |
| Median r 1 / 3 / 10 | 0 / 0.88 / 28.0 | 0 / 0 / 0 | Median over the square window (2r+1)², edge pixels repeated. Prism used a disc. The research formula was exact. |
| Dust & Scratches r 3 t 20, r 8 t 60 | 1.19 / 30.3 | 0 / 0 | The median of the same square; a pixel takes it in every channel when any colour channel differs from its median by more than the threshold (levels), else keeps every channel. The research formula (per channel) left 0.45–0.69. |
| Add Noise | σ about half (Uniform) to a quarter (Gaussian) of Photoshop's | σ within 1–3 % | Measured on flat grey: Uniform spreads evenly over ±(round(2.55 × amount) + 0.5) levels, half a level above zero; Gaussian has σ ≈ 2.6 × amount levels and its tails stop near 3.5σ (Prism: the sum of four even draws; kurtosis 2.7 against Photoshop's 2.8–2.9). |
| Despeckle | 2.33 | 2.33 (kept) | Not solved. Probes: an isolated 1 px speck of +72 keeps +10 at its centre and spreads +6 / +4 to side / corner neighbours; a long 40-level edge only loses 5 levels per side; a 10-level edge is blurred fully. Prism keeps its surface-blur approximation; the research one scored 2.35. |
| Reduce Noise | — | kept | Photoshop 27.4 refuses to run it on the test machine ("program error"), so neither dialog nor output could be captured. |

### Sharpen

| Filter | Before | Now | What Photoshop does |
| --- | --- | --- | --- |
| Sharpen | 4.85 | 0.005 | Fixed kernel: 2 at the centre, −¼ on the four sides (least-squares fit exact). |
| Sharpen More | 5.03 | 0.008 | Fixed kernel: 3 at the centre, −¼ on all eight neighbours. |
| Sharpen Edges | 2.24 | 0.048 | Sharpen's change weighted by the Sobel gradient size g (levels): w = clamp((g − 65) / 191, 0, 1). 98 % of pixels exact. |
| Unsharp Mask (r 1 / 3, t 0) | 0.34 / 0.27 | 0.03 / 0.26 | Same blur as Gaussian Blur (below). |
| Unsharp Mask threshold | 1.33 (t 10) | 0.07 | Soft: the amount-scaled difference loses `threshold` levels of its size (measured offset = T / amount at T 4, 10, 20, 40), so small differences don't change and larger ones fade in. |
| Smart Sharpen (not Legacy) | 1.27 / 1.70 | 0.55 / 0.97 | Close to an unsharp mask of 0.9 × amount over a blur of 0.9 × radius (a linear fit leaves 0.2, so most of it is linear). Not exact. Legacy = Unsharp Mask (0.03). Lens Blur / Motion Blur removal: a box of the radius / a streak of twice the radius (not measured). |

### Gaussian Blur (shared by Unsharp Mask, Smart Sharpen, High Pass)

Photoshop's kernel is separable and, for small radii, quantised: radii 2.2, 2.3 and 2.4 give the
same kernel, so do 2.5 / 2.6 and 2.9 / 3.0. Prism carries the 30 measured half-kernels for
0.1 … 3.0 (`algo/src/blur.rs`, `PS_GAUSS`), mixing neighbours for radii in between; beyond 3 it keeps
its σ = radius kernel and three box passes past 4, which already match within 0.1–0.2.

| Radius | 0.3 | 0.5 | 1 | 1.5 | 2 | 3 | 5 | 10 | 25 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Before | 5.52 | 3.94 | 0.79 | 0.55 | 0.64 | 0.10 | 0.18 | 0.14 | 0.21 |
| Now | 0.03 | 0.04 | 0.06 | 0.05 | 0.08 | 0.06 | 0.18 | 0.14 | 0.21 |
| Research | 5.52 | 3.94 | 0.79 | 0.55 | 0.58 | 0.60 | 0.56 | 0.63 | 0.65 |

### Render and Video (dialogs only)

Photoshop 27.4's Render menu is Flame, Picture Frame, Tree, Clouds, Difference Clouds, Lens Flare
and Fibers (no Lighting Effects any more; Prism keeps it). Dialogs matched: Fibers (Variance and
Strength sliders, Randomize button, no Preview checkbox), Lens Flare (Brightness %, Lens Type as a
list of four radio buttons, click or drag in the preview to place the centre), De-Interlace (no
preview: Eliminate Odd / Even Fields and Create New Fields By Duplication / Interpolation as radio
lists, Interpolation by default). Flame, Picture Frame and Tree have Photoshop's large tabbed
dialogs and keep Prism's own. Korean names follow Photoshop's menus (구름 효과 1/2 are its names
for Clouds / Difference Clouds).

### Stylize

| Filter | Before | Now | What Photoshop does |
| --- | --- | --- | --- |
| Find Edges | 26.4 | 0.08 | 255 − the Sobel gradient size, per channel (Prism divided it by 4, the research by 2). |
| Solarize | 0 | 0 | Unchanged. |
| Emboss (135°/3/100, 45°/1/200, −30°/8/80) | 20.7 / 13.4 / 37.2 | 0.56 / 1.05 / 0.45 | The research formula: per channel, grey + amount × (bilinear sample half the height ahead − half behind along the angle). Prism used rounded offsets on luminance. |
| Trace Contour (128 Upper, 80 Lower) | 61.0 / 41.1 | 0 / 0 | Upper outlines the pixels at or under the level that touch (4-neighbours) one above it; Lower the pixels at or over it that touch one under it. Upper also outlines pure-white (255) pixels against anything darker, whatever the level. Both Prism and the research had the sides swapped. |
| Wind / Blast | almost no streaks | streak statistics close | About one start per pixel per row in random order; from a start, while brighter (sum of channels) than the next pixel downwind, Wind sets the next to the mean of the two, so streaks fade by halves (191, 143, 107… after repeated passes); Blast copies the start over the next 10 pixels, so streak lengths are multiples of 10. "From the Right" streaks to the right. Mean streak 10.8 px (Photoshop) against 13.5 (Prism) for Wind. |
| Stagger | — | kept | Not solved: rows move by an amount that depends on their content (a 60 px dark run lets a white block move ≈ 38 px, a 10 px run ≈ 7 px). |
| Diffuse, Tiles, Extrude, Oil Paint | — | kept | Random or brush-based; compared by eye only. |

Dialogs: Extrude and Tiles have no preview and no sliders (Extrude's Type and Depth mode are radio
buttons on their rows); Wind has no Preview checkbox and lists Method and Direction as radio
buttons; Emboss and Oil Paint set angles with a wheel; Oil Paint groups Brush and Lighting (the
Lighting checkbox enables Angle and Shine); Diffuse's Mode and Trace Contour's Edge are framed radio
groups (Trace Contour defaults to Upper). Photoshop's names live in the catalogue's `filter`
context.

### Distort

| Filter | Before | Now | What Photoshop does |
| --- | --- | --- | --- |
| Twirl 50 / −300 | 39.0 / 60.4 | 1.19 / 6.15 | Research formula: θ = −angle/57 × (1 − r)² inside the ellipse inscribed in the bounds (Prism used the inscribed circle and a linear falloff). |
| Pinch 50 / −80 | 20.1 / 21.6 | 1.45 / 1.88 | Research formula: sample at the offset × (1 + u²(1 − u/2)² × amount/32), u = 1 − r, inside the ellipse. |
| Spherize 100 / −60 horizontal | 24.9 / 2.69 | 2.68 / 2.69 | Research formula over the ellipse (asin for positive amounts, sin for negative). |
| ZigZag Around Center / Out From Center | 16.9 / 10.4 | 2.29 / 1.60 | The research formula's shape, w = u(1 − cos 2π·ridges·u), but over the inscribed ellipse (u = 1 − r; the research used the long side, twice as far): Around turns by −amount/57 × w, Out moves the sample amount × w / 4 px toward the centre. |
| ZigZag Pond Ripples | 22.0 | 19.0 | Not solved (best fit: half of each). |
| Polar Coordinates (both ways) | 109 / 80.5 | 7.1 / 7.7 | Counter-clockwise from 12 o'clock over the inscribed ellipse; the corners repeat the edge rows; nearest source pixel (bilinear sampling scored 16 / 6.5 against the same geometry). The research and Prism ran clockwise. |
| Ripple | 18.3 | own field | Photoshop tiles stored 100 × 100 displacement tables (not copied); Prism now builds its own 100-periodic field of the same strength and grain (RMS 0.74 / 1.07 / 3.20 px, grain 7 / 8 / 14 px) instead of a regular sine. |
| Shear, Displace, Wave | — | kept | Shear's curve and Displace's map file need their own controls; Wave is random. |

Dialogs: no Preview checkbox in any Distort dialog; Spherize, Twirl, Pinch and ZigZag show a small
grid diagram of the distortion under the buttons (Prism runs a ruled square through the filter);
Polar Coordinates is a radio list; Twirl writes its unit as "degrees" (도).

### Pixelate

| Filter | Before | Now | What Photoshop does |
| --- | --- | --- | --- |
| Mosaic 10 / 7 | 0.002 / 0.004 | same | Already matched. |
| Fragment | 7.84 | 0 | The mean of four copies offset 4 px diagonally (Prism offset them along the axes). |
| Facet | 1.81 | kept | Not solved; the research scored 2.04. |
| Color Halftone | 97.1 | kept | Prism's dots look like Photoshop's but sit on a different grid phase, so the per-pixel error stays high; the research formula is a different pattern. |
| Mezzotint, Crystallize, Pointillize | — | kept | Random. |

Dialogs: Color Halftone has no preview and only fields (Max. Radius in pixels; Screen Angles
(Degrees): Channel 1–4); Mosaic's unit is "square"; Mezzotint, Crystallize and Pointillize have no
Preview checkbox. Mezzotint's Korean type names other than 작은 점 (seen) are inferred.

### Blur

| Filter | Before | Now | What Photoshop does |
| --- | --- | --- | --- |
| Blur | 3.63 | 0 | ½ at the centre, ⅛ on the four sides. |
| Blur More | 5.91 | 0.001 | [1 2 1; 2 2 2; 1 2 1] / 14. |
| Average, Box Blur 3 / 10, Surface Blur 5/15 | 0 | 0 | Already matched. |
| Motion Blur 0°/10, 30°/25 | 0.04 / 1.54 | kept | Prism and the research agree. |
| Radial Blur Zoom 20 | 13.8 | 3.0 | Samples along the ray over amount/2 % of the distance, centred on the pixel (Prism went inward only). Spin (2.78) kept. |
| Smart Blur 3/25 | 1.74 (≈ no change) | kept | Averaging the neighbours within the radius whose own channel differs by under the threshold gets 1.41; not adopted yet. |
| Gaussian Blur | see above | | |
| Lens Blur, Shape Blur | — | kept | Need depth maps / custom shapes. |

Dialogs: Smart Blur has no Preview checkbox; Motion Blur's angle has a wheel; Radial Blur has no
preview: Amount, Blur Method and Quality as radio lists, and the Blur Center box (click to move the
centre).

### Blur Gallery

Field, Iris, Tilt-Shift, Path and Spin Blur open Photoshop's Blur Gallery workspace (pins on the
canvas, a Blur Tools panel). Prism shows them in the classic dialog for now; the workspace is
still to be built.

### Other

| Filter | Before | Now | Note |
| --- | --- | --- | --- |
| Custom (sharpen, weighted blur + offset) | 0 / 0.01 | same | Already matched. |
| High Pass 10 / 1.5 | 0.52 / 0.32 | same | Uses the measured Gaussian. |
| Maximum / Minimum 3 Squareness | 0 / 0 | same | |
| Maximum 4 Roundness | 0.36 | same | |
| Offset (wrap, repeat) | 0 / 0 | same | |
| HSB/HSL | — | kept | Not compared (its script event isn't known). |

Dialogs: Offset has no preview pane but a Preview checkbox, "pixels right / pixels down" units and
a framed Undefined Areas group; HSB/HSL has no preview, Input Mode and Row Order radio lists;
Minimum / Maximum keep Preserve as a menu (방형 / 원형).
