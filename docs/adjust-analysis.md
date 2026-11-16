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

## Still open

- Auto's target curve (Photoshop's trained model) is a stand-in.
- The other adjustments, in menu order.
