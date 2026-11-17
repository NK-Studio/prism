# Image › Adjustments investigation

Investigation started 2026-11-16 (maintainer request). One adjustment at a time: Photoshop's
dialog is captured and Prism's is rebuilt to the same structure; the maths is whichever of
Prism's existing implementation and the maintainer's research notes is closer to Photoshop's real
output, measured. This is not a claim of Photoshop source recovery.

## Authorization and boundary

The maintainer asked on 2026-11-16 to rebuild Image › Adjustments (Filter-menu items excluded)
from their research notes (outside this repository) and from screenshots of Photoshop's dialogs.
Where the notes give a formula read from the binary, Prism uses it as written (the maintainer's
instruction of 2026-11-17: no measured comparison needed); Prism's code is independent, written
from the formulas and constants only. No binary, pseudocode or analysis database belongs in
source control.

## Method

Photoshop 27.4 (macOS arm64) is driven through ExtendScript (`app.open`, `executeAction`,
`saveAs` PNG) on synthetic 8-bit RGB images: a 512×256 chart (grey ramp over every level, then
hue × saturation × value) and a 256-wide grey ramp. Prism runs the same settings through
`prism-cli run`; the research formulas are ported to a throwaway script for the comparison. "MAE"
is the mean absolute difference from Photoshop in 8-bit levels over RGB. Dialogs are captured from
Photoshop's own menus (`agent-desktop screenshot`) and compared with Prism's offscreen snapshot.

## Brightness/Contrast

### Dialog (Photoshop 27.4)

Title centred over a hairline. Left: "Brightness" and "Contrast" (ko: 명도, 대비), each a label
with its number field at the right and a slider under it; "Use Legacy" (레거시 사용) at the bottom
left. Right column: OK, Cancel (Alt: Reset, dialog stays open), Auto; "Preview (Opt+P)" at the
bottom under the buttons. The brightness slider is linear over −150..150. The contrast slider has
0 in the middle: the left half runs to −50, the right half to 100 (−41 sits 0.78 of the way left,
50 halfway right). With Use Legacy the field accepts −100..100.

Prism: `ui-egui/src/adjust_ps_ui.rs`, same structure, Auto included.

### Formulas (from the research notes, `compose/src/bright_contrast.rs`)

- **Modern brightness.** `B = b · 0.02`, split into `s1 = min(|B|, 2)` and `s2 = max(|B| − 2, 0)`.
  A stage is a line of slope `p = 1.3703509847201236^s` (= 2^(s/2.2)) up to output 0.5, then a
  cubic Hermite to (1, 1) with end slope `k = max(0.1, 1/(1 + 12 (p − 1)))`. Brighter runs stage 1
  then 2; darker the inverse of stage 2 then of 1 (secant method, 30 steps, |Δf| < 1e-10).
- **Modern contrast.** `x += c · 0.00095 · (x > 0.5 ? 1 − (4(x − 0.75))² : (4(x − 0.25))² − 1)`,
  after the brightness.
- **Tables.** 8-bit documents: `x = i · (1/255 as f32)`, `int(x · 255 + 0.5)`. Other depths: the
  16-bit table, 4097 entries over 0..32768. Prism samples the table into one LUT for every
  channel, on the CPU and the GPU.
- **Legacy.** Integer lines about the **mean level** `m` (pivot): contrast ≥ 0:
  `D = 255 − ⌊255 c/100⌋`, `m ± (|i − (m − b)| · 255 + D/2) / D` (100: threshold at `m − b`);
  contrast < 0: `M = ⌊255 c/100⌋ + 255`, `m + b ± (M |i − m| + 127) / 255`. The 16-bit table moves
  `m` and `b` to `(v · 257 + 1) / 2`.
- **Mean level.** The mean of a 256-bin histogram of `0.299 R + 0.587 G + 0.114 B`, rounded.
  Prism stores it with the adjustment (`Adjustment::BrightnessContrast::mean`, the PSD `brit`
  block's mean field and descriptor `means`); the command computes it from the active layer when
  `mean` isn't given (layers over 1000 px are sampled on a grid). A quick check in Photoshop 27.4
  confirmed the pivot is this mean (grey images with means 60, 195 and 49, and a colour image whose
  luminance mean is 84: pivot 84 in every channel).
- **Auto.** The image is halved (2×2 box) until its long side is ≤ 500 px. Photoshop builds a
  target curve with a trained model (`BuildCurveFromMachineLearning`) that the research didn't
  port; the stand-in target stretches the luminance histogram after clipping 0.1 % at each end,
  weighted by each level's pixel count. (brightness, contrast) are fitted from (0, 0) by
  Nelder–Mead (start simplex ±10; the notes say the original optimiser's exact method wasn't read)
  minimising `sqrt(Σ w (f(x) − y)² / Σ w)` over the 16-bit table (values cut toward zero, out of
  range → `f64::MAX`), at most 10000 evaluations, tolerance 0.0001. The result is cut toward zero;
  a value beyond its slider becomes a fixed one (modern brightness ±75, contrast −25 / 50; legacy
  ±50). Because of the stand-in target, Auto's values are not expected to equal Photoshop's (the
  13 measured below).

### Earlier black-box measurements (before the research formulas)

Kept for reference: on the 512×256 chart Prism's old curve was 0.1–3 levels MAE (max 45 at
b −150) and its legacy path 28 levels MAE at b −60 c 70 (contrast was always applied first, about
mid-grey). The research formulas reproduce the measured ramp tables within one level (compose
tests `modern_brightness_contrast_matches_photoshop`, `legacy_brightness_contrast_matches_photoshop`).

### Photoshop's Auto on 13 test images (measured)

| Image (levels) | Brightness | Contrast |
| --- | --- | --- |
| chart | 1 | −41 |
| grey 0..255 | 14 | −19 |
| grey 10..245 | 4 | 30 |
| grey 40..215 | 8 | 76 |
| grey 100..150 | 15 | 76 |
| grey 150..240 | −7 | 87 |
| grey 20..100 | 32 | 56 |
| grey 60..90 | 29 | 56 |
| grey 5..60 | 52 | 15 |
| grey 0..128 | 64 | −15 |
| grey 128..255 | −20 | 17 |
| dark (gamma noise, mean 49) | 52 | 64 |
| bright (mirror of dark, mean 205) | −16 | 99 |

## Levels

### Dialog (Photoshop 27.4)

Title centred over a hairline. "Preset:" with a dropdown (Default) and a gear menu; a frame headed
by "Channel: RGB" holding "Input Levels:" (histogram, black / grey / white triangle handles, fields
0 / 1.00 / 255) and, under a hairline, "Output Levels:" (gradient, two handles, fields 0 / 255).
Right column: OK, Cancel, Auto, Options…, the Set Black / Gray / White Point eyedroppers, Preview.

Prism (`ui-egui/src/adjust_ps_ui.rs`): the same structure. The preset dropdown has Default and
Custom (shown once anything differs; choosing Default resets); Photoshop's built-in presets and the
gear menu (save / load presets) and Options… (Auto Color Correction Options) aren't there yet. Alt
turns Cancel into Reset. The gamma field shows two decimals (1.00).

### Maths

The research notes give Levels only as its definition (`((x − in black) / (in white − in black))^(1/γ)`
into the output range), not read from the binary. Prism's Levels already is that formula with
Photoshop's whole-level rounding (fitted on the PSD corpus), so it is kept.

### Eyedroppers (from the research notes)

`adjust_params::levels_eyedropper` (and the `eyedropper` param of `image.adjustments.levels` /
`layer.newAdjustmentLayer.levels`, sampled as for Curves), written from the research's
`fx::levels` (the maintainer's reference implementation):

- **Target colours** (Photoshop's defaults): shadows 0, midtones 128, highlights 255.
- **Set Black / White Point.** Per channel, input black (white) = the sample's level (rounded),
  output black (white) = the shadow (highlight) target. The input ends are kept 2 levels apart:
  with `n = black − white + 2 ≥ 1`, white moves up `n / 2` and black down `(n + 1) / 2`, then both
  shift back inside 0..255. (The research marks the gap of 2 and the black/white per-channel rule
  as inferred.)
- **Set Gray Point.** Per channel `γc = L(sample; input black, white) ÷ L(128; output black,
  white)` with `L(x) = log2(|hi − lo| / (x − min))` (`x` rounded and kept in `[min + 1, max − 1]`;
  under one level → 0.001; `γ = 1` when undefined), then every `γc` divided by the geometric mean
  `exp(ln(γr γg γb) / 3)`, ×100 rounded. The colour goes, the brightness stays: (200, 128, 100)
  gives 0.45 / 1.28 / 1.74 and a neutral 149.
- RGB edits the red, green and blue records; Grayscale the composite record (black and white only).

## Curves

### Dialog (Photoshop 27.4)

Title centred over a hairline. "Preset:" with a dropdown (Default) and a gear menu. A frame headed
by "Channel: RGB" holding, at the left, the point / pencil mode buttons, "Output:" and the
On-image Adjustment tool; the graph (4 × 4 grid, histogram behind it, a vertical gradient bar at
its left and a horizontal one under it, with a black and a white input slider triangle under that);
"Input:" under the bar; the Set Black / Gray / White Point eyedroppers and "Show Clipping" in the
last row. A middle column of the Curve Display Options: "Show Amount of:" (Light (0-255) /
Pigment/Ink %), "Grid size:" (4 × 4 / 10 × 10 buttons), "Show:" (Channel Overlays, Histogram,
Baseline, Intersection Line). Right column: OK, Cancel, Smooth (only for the pencil), Auto,
Options…, Preview. The fields beside Output and Input show while a point is selected. Korean:
양 표시 / 빛(0-255) / 안료/잉크 % / 격자 크기 / 표시 / 채널 오버레이 / 막대 그래프 / 기준선 /
교차 선 / 매끄럽게 / 클리핑 표시.

Prism (`ui-egui/src/adjust_ps_ui.rs` `curves`, graph `adjust_editors::curves_graph`): the same
structure. Pigment/Ink % reverses both axes and the bars and shows ink % in the fields (CMYK
documents start in it); the input sliders move the curve's first and last points sideways; the graph
is drawn as measured on Photoshop's (2× screenshots driven with `cua-driver`, dark theme): graph 79
on a dialog of 83, grid grey 53 at 46 %, baseline darker (about 56), frame 1.5 px grey 48, gradient
bars 3 px (the vertical one 2 px left of the frame, the horizontal one right under it), curve 2 px,
point squares 9 px cut off at the graph's edge (the selected one solid), house-shaped input sliders.
The Intersection Line (grey 118 at 50 %) crosses the graph through the selected point for as long as
it is selected, during the drag and after it wherever the pointer is. With no point selected, Output
and Input show the level under the pointer as plain text. Option/Alt+2…5 pick the channels; Alt
turns Cancel into Reset. Cmd+Z steps back through the dialog's own edits and Shift+Cmd+Z forward (a
whole drag is one step; up to 100), in Brightness/Contrast and Levels too; not checked in Photoshop
yet (it had quit when this was written). Auto sets the channel's ends at its 0.1 % clip levels (as
Levels' Auto; Photoshop's own default algorithm wasn't checked). The view options are view state,
not parameters. Not there yet: the pencil mode and Smooth, the On-image Adjustment tool, Show
Clipping, Options…, the gear menu and Photoshop's built-in presets.

### Maths

The research notes give Curves only as their own approximation (a Fritsch–Carlson monotone curve
through three fixed points), not read from the binary. Prism keeps its natural cubic spline through
the points (overshoot allowed, clamped, constant outside the end points; fitted on the PSD corpus).
The eyedroppers keep Prism's per-channel point placement.

## Exposure

### Dialog (Photoshop 27.4, measured by driving it)

Title centred over a hairline. "Preset" (no colon) with a dropdown (Default) and a menu button;
then Exposure, Offset and Gamma Correction (ko: 노출 / 오프셋 / 감마 교정), each a label with its
field at the right and a slider with a round knob under it. Right column: OK, Cancel, the Set
Black / Gray / White Point eyedroppers, "Preview(Opt+P)". Measured with real drags:

- Exposure: −20..+20, linear (0 in the middle; three quarters right reads +11.86).
- Offset: −0.5..+0.5, linear.
- Gamma Correction: 9.99 at the left end, 1 in the middle, 0.01 at the right end, each half linear
  (three quarters right reads 0.42).
- Fields drop trailing zeros and write a "+" only before values of 1 and more: "+20", "+11.86",
  "+1", "+9.99", but "0.5", "0.42", "0.01", "-0.5", "-20", "0". (Typing into the fields was not
  captured.)

Prism (`ui-egui/src/adjust_ps_ui.rs` `exposure`): the same layout, fields and sliders; Alt turns
Cancel into Reset; Cmd+Z / Shift+Cmd+Z as in the other dialogs. Not there yet: Photoshop's
built-in presets and the preset menu.

### Maths

The research notes give Exposure only as their own estimate (linear light × 2^exposure + offset,
then 1/gamma), which is the formula Prism already uses through the document's exposure tone curve
(a 2.2 power for RGB, fitted on the PSD corpus), so it is kept.

### Eyedroppers

From Adobe's description (not the research notes): Set Black Point sets the Offset so the sample
goes to zero, Set White Point sets the Exposure so it goes to white, Set Gray Point sets the
Exposure so it goes to middle grey (level 128). Prism (`adjust_params::exposure_eyedropper`, the
`eyedropper` param of `image.adjustments.exposure`) works on the sample's luminance
(0.299 R + 0.587 G + 0.114 B) in the same linear light; the luminance weighting is inferred, not
measured.

## Vibrance

### Dialog (Photoshop 27.4, measured by driving it)

Title centred over a hairline; Vibrance and Saturation (ko: 생동감 / 채도), each a label with its
field at the right and a slider under it, both linear over −100..+100 (0 in the middle); fields
written "+100", "+53", "0", "-100". Right column: OK, Cancel, "Preview(Opt+P)". No preset.

Prism (`ui-egui/src/adjust_ps_ui.rs` `vibrance`): the same; Alt turns Cancel into Reset; Cmd+Z /
Shift+Cmd+Z as in the other dialogs. The menu item and the dialog say 생동감 as Photoshop 27.4
does (the plain "Vibrance" string elsewhere stays 활기).

### Maths

Unchanged: the research notes only estimate Vibrance; Prism's `compose::adjust::vibrance_lin` is
already fitted to Photoshop's output.

## Hue/Saturation

### Dialog (Photoshop 27.4, measured by driving it)

A redesigned dialog. "Preset:" with a dropdown and a menu button; a row with the On-image
Adjustment tool and round range swatches (Master as a colour wheel, then Reds, Yellows, Greens,
Cyans, Blues, Magentas; the chosen one ringed) in place of the old channel dropdown; "Hue:",
"Saturation:" and "Lightness:" (ko: 색조 / 채도 / 밝기), each with its field and a slider whose
track is a spectrum, grey → red, black → white; a row with Colorize, the three range eyedroppers
(Eyedropper, Add to Sample, Subtract from Sample; off on Master) and **Invert** (반전); then
"Before - After" (이전 - 이후): one bar, input hues on its upper half, the result on its lower half,
under it the range's sliders (fully-on part light grey, fall-offs darker, white handles) and the
degrees "315°/345°" at the left and "15°\45°" at the right. Right column: OK, Cancel,
"Preview(Opt+P)". Fields carry no "+" (180, 100, -100). Measured:

- Invert swaps a range for the other hues: reds 315/345/15/45 become 15/45/315/345; the swatch
  then shows the range colour split with its complement.
- Colorize takes the foreground colour's hue (a red foreground gave 359) and saturation 25; the
  swatches, eyedroppers and Invert go grey and the hue slider runs 0..360.
- Hue −180..180 (180 at the right end), Saturation −100..100, Lightness −100..100.

Prism (`ui-egui/src/adjust_ps_ui.rs` `hue_saturation`, parts `adjust_editors::hs_*`): the same,
except the On-image Adjustment tool, Photoshop's presets and the preset menu. The eyedroppers'
rules are inferred (`adjust_params::hue_range_eyedropper`: Eyedropper centres the fully-on part
on the sample's hue keeping the widths, Add stretches the nearer edge out to it, Subtract pulls
the nearer edge in to it). Alt: Reset; Cmd+Z as in the other dialogs. Maths unchanged (the
research notes only estimate Hue/Saturation).

Photoshop crashed twice while it was being driven (SIGBUS, 07:29 and 08:54; earlier it refused new
documents with a disk error), so the remaining checks wait for a stable Photoshop.

## Presets

The "Preset:" dropdowns of Levels, Curves, Exposure, Hue/Saturation, Black & White and Channel Mixer list Photoshop's built-in
presets (`ui-egui/src/adjust_presets.rs`), in its order, after Default; Custom shows once the
settings match none. The settings were **observed, not copied**: Photoshop 27.4 was asked through
its scripting interface to make an adjustment layer from each of its presets, and the layer's
settings were read back (2026-11-17). The Korean names are Photoshop's own (its Korean UI strings,
`preset` context in `ko.tsv`). Curves' "(RGB)" presets are offered for RGB documents only.

- Levels: Darker (input 15/255), Increase Contrast 1/2/3 (10/245, 20/235, 30/225), Lighten Shadows
  (gamma 1.6), Lighter (0/230), Midtones Brighter (gamma 1.25), Midtones Darker (gamma 0.75).
- Curves (RGB): Color Negative, Cross Process (per channel), Darker, Increase Contrast, Lighter,
  Linear Contrast, Medium Contrast, Negative, Strong Contrast (composite points).
- Exposure: Minus 1.0, Minus 2.0, Plus 1.0, Plus 2.0 (exposure only).
- Hue/Saturation: Cyanotype (colorize 215/25), Increase Saturation More (+30), Increase Saturation
  (+10), Old Style (−40 saturation, +5 lightness), Red Boost (hue −5, +20), Sepia (colorize 35/25),
  Strong Saturation (+50), Yellow Boost (hue +5, +20); all on the master.

Not there yet: the preset menu (save / load / delete) and the user's own saved presets.

## Color Balance

### Dialog (Photoshop 27.4, measured by driving it)

Redesigned: "Tone" (톤) with three round swatches, black "Shadows", grey "Midtones", white
"Highlights" (어두운 영역 / 중간 영역 / 밝은 영역; the chosen one ringed); "Cyan to Red",
"Magenta to Green", "Yellow to Blue" (녹청에서 빨강 / 자홍에서 초록 / 노랑에서 파랑), each a field
written "+58" and a slider on a colour track; "Preserve Luminosity" (광도 유지) at the bottom left;
OK, Cancel, "Preview(Opt+P)" at the right. Each tone keeps its own values.

Prism (`ui-egui/src/adjust_ps_ui.rs` `color_balance`): the same, with Photoshop 25.4's measured
gestures kept (double-click zeroes a slider, the wheel steps it, Shift ×10; Alt: Reset zeroes every
tone and shows Midtones) and Cmd+Z. Maths unchanged. Photoshop quit again when its dialog was
cancelled (as on Hue/Saturation).

## Black & White

### Dialog (Photoshop 27.4, measured by driving it)

"Preset:" with its twelve presets and a menu button; "Reds:", "Yellows:", "Greens:", "Cyans:",
"Blues:", "Magentas:" (빨강 / 노랑 / 녹색 / 녹청 / 파랑 / 마젠타 계열), each a field written "40%"
(no "+": "300%", "-200%") and a slider linear over −200..300 % on a black → colour → white track;
"Tint" (색조) with its colour swatch, then "Hue" (42°) and "Saturation" (20%) of the tint, greyed
while Tint is off; OK, Cancel, Auto, "Preview(Opt+P)". The default tint is 225/211/179 (hue 42,
saturation 20 in HSB). Closing the dialog with OK (not Cancel) kept Photoshop running.

Prism (`ui-egui/src/adjust_ps_ui.rs` `black_white`): the same, except Auto (Photoshop's rule for
it isn't known). The tint's Hue and Saturation edit its colour in HSB, keeping its brightness. The
twelve presets (Blue Filter … Yellow Filter, observed through Photoshop's scripting) are in
`adjust_presets`. Maths unchanged.

## Photo Filter

### Dialog (Photoshop 27.4, captured)

"Filter" (필터) as a radio with a dropdown of its twenty filters, named in English even in the
Korean UI (Warming Filter (85), (LBA), (81), Cooling Filter (80), (LBB), (82), Red, Orange, Yellow,
Green, Cyan, Blue, Violet, Magenta, Sepia, Deep Red, Deep Blue, Deep Emerald, Deep Yellow,
Underwater); "Color" (색상) as a radio with the colour swatch (live under Filter too); "Density"
(조밀도) "25%" with a slider; "Preserve Luminosity"; OK, Cancel, "Preview(Opt+P)".

Prism (`ui-egui/src/adjust_ps_ui.rs` `photo_filter`): the same; a custom colour falls back to
Warming Filter (85) when Filter is chosen; Alt: Reset; Cmd+Z. Filter colours and maths unchanged.

## Channel Mixer

### Dialog (Photoshop 27.4, measured by driving it)

"Preset" with six presets (all monochrome: Black & White Infrared, with Blue / Green / Orange / Red /
Yellow Filter, each "(RGB)"; observed through Photoshop's scripting) and a menu button; "Output
Channel" (출력 채널) with round red / green / blue swatches; "Monochrome" (단색); "Red", "Green",
"Blue" (빨강 / 녹색 / 파랑) rows with fields ("200", "-200", no "+") and sliders linear over −200..200
on black → colour → white tracks; "Total" (총합) as text, "200 %" with a warning sign once it passes
100; a hairline; "Constant" (상수); OK, Cancel, "Preview(Opt+P)".

Prism (`ui-egui/src/adjust_ps_ui.rs` `channel_mixer`): the same (Monochrome shows one grey swatch
and starts from Photoshop's 40/40/20 mix); Alt: Reset; Cmd+Z. Maths unchanged.

## The rest of the menu (Photoshop 27.4, captured 2026-11-17)

Captured one after another from Photoshop's own menus (Photoshop quit twice while this was
driven: once when Posterize was confirmed, before the others; nothing but a test document was
open). Prism draws them in `ui-egui/src/adjust_ps_more.rs`, each with OK, Cancel (Alt: Reset), Preview
and Cmd+Z like the others; maths unchanged. Photoshop controls Prism's adjustments don't model are
left out rather than shown doing nothing.

- **Posterize** (redesigned): "Levels" with its field and a 2..255 slider.
- **Threshold** (classic): "Threshold Level:" with its field centred over the histogram, a
  house-shaped slider under it; Preview without a shortcut in its label.
- **Gradient Map** (classic): "Gradient Used for Grayscale Mapping" (the gradient, ▾ for the
  presets; Prism: a click on the gradient edits its stops in place of Photoshop's Gradient Editor)
  and "Gradient Options" (Dither, Reverse; Photoshop's Method isn't modelled).
- **Selective Color** (redesigned): "Preset", nine round swatches (Reds … Blacks), Cyan / Magenta /
  Yellow / Black rows ("0%", colour tracks), Method: Relative / Absolute.
- **Color Lookup** (redesigned): 3DLUT File / Abstract / Device Link radios with their dropdowns,
  Dither. Prism: 3DLUT File only (Load 3D LUT… opens the file picker, then the looks).
- **Shadows/Highlights** (classic): Shadows and Highlights amounts; Show More Options adds Tone /
  Radius and the Adjustments group (Color "+20", Midtone, Black / White Clip "0.01%"), in
  hairline-separated groups with right-aligned labels. Prism leaves out Load…, Save… and Save As
  Defaults.
- **HDR Toning** (classic): Preset with a gear menu; "Method: Local Adaptation" framing Edge Glow
  (Radius, Strength, Smooth Edges), Tone and Detail (Gamma, Exposure, Detail), Advanced (Shadow,
  Highlight, Vibrance, Saturation) and the collapsed Toning Curve and Histogram. Prism: no presets
  or Smooth Edges; the toning curve uses Curves' graph. (Photoshop showed radius 7 px and strength
  0.52; Prism's defaults stay 30 / 0.5, not checked against a fresh Photoshop.)
- **Match Color** (classic): Destination Image ("Target: name (RGB/8)"), Image Options
  (Luminance, Color Intensity, Fade, Neutralize), Image Statistics (Use Selection in Source /
  Target, Source, Layer, Load / Save Statistics, a source thumbnail). Prism leaves out Ignore
  Selection, the statistics files and the thumbnail.
- **Replace Color** (classic): Eyedropper / Add / Subtract and the "Color:" swatch, Localized Color
  Clusters, Fuzziness, the Selection / Image preview, then Hue / Saturation / Lightness with the
  Result swatch; Load…, Save…. Prism: the Eyedropper (one colour), Fuzziness, the preview (cached
  per document revision), the replacement and Result.

Found on the way: the parameter-form parser read a note after the parameter list
(`{…} (color: …)`) as part of the last parameter, so Replace Color opened with lightness −100 (and
other commands with a note lost their last default); fixed in `filter_dialog::parse_spec`.

## Invert, Desaturate and Equalize (Photoshop 27.4, measured 2026-11-17)

Each applied by script to 8-bit test images (a hue/saturation chart, uneven random colours, grey
steps, a low-contrast ramp, two colours) and the saved results compared pixel by pixel.

- **Invert**: `255 − v` on every channel, exact. Prism unchanged.
- **Desaturate**: each pixel becomes the grey `(max + min) / 2` (HSL lightness), and at 8 bits the
  half is **rounded down** (exact on every pixel; rounding to nearest is off by one on half the
  chart). Prism's Hue/Saturation path rounded up, so `pixels::desaturate_surface` now rounds 8-bit
  RGB halves down; other depths and colour models take the Hue/Saturation path as before.
- **Equalize**: one table for all three channels, from the histogram of every R, G and B value
  together (not luminosity: off by up to 69 levels). Level `v` sits at `below(v) + ⌊count(v)/2⌋`;
  the darkest level present maps to 0, the brightest to 255, the rest in proportion, rounded.
  Exact on all six images, the selection case included (`⌊count/2⌋` rather than `count/2` was
  settled by one level that came out 178 rather than 179). Prism's previous table
  (`(cdf − first) / (total − first)`) was off by up to 30 levels. An image with only one level
  can't be equalized: Photoshop refuses ("…only one brightness value"), and so does Prism, leaving no
  history step.
- **Equalize with a selection**: Photoshop opens "균일화" (Equalize) with an "Options" box,
  *Equalize Selected Area Only* / *Equalize entire image based on selected area*, and OK / Cancel;
  no Preview. Both use the selected pixels' histogram; the first applies the table to the
  selection, the second to the whole layer (both measured exact). The dialog opens on the last
  choice; the first time it showed the second, which Prism uses as its default
  (`selectedAreaOnly: false`). Without a selection there is no dialog.

## Still open

- Auto's target curve (Photoshop's trained model) is a stand-in.
- The other adjustments, in menu order.
