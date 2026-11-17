# Layer Style maths investigation

Started 2026-11-17 (maintainer request). The maintainer's research notes on Photoshop 2026
(27.4, macOS arm64) layer-effect code give formulas and constants for the soft blur, effect
offsets, distance transforms, contours and the bevel's lighting. Prism's layer-style renderer
(`crates/compose/src/effects.rs`, mirrored on the GPU in `crates/gpu/src/fx.rs` and
`compose.wgsl`) was reworked from them and measured against the Photoshop-authored oracles. This
is not a claim of Photoshop source recovery.

## Authorization and boundary

The maintainer asked on 2026-11-17 to rework the layer-style maths from their research notes
(outside this repository). Prism's code is written independently from the documented formulas
and constants; no binary, pseudocode or analysis database belongs in source control. Where the
notes mark a value as an estimate, or where a formula as written measured further from
Photoshop's own renderings than the alternative, Prism keeps what the oracles show and says so
below.

## Method

`cargo test --release -p prism-io --features corpus --test corpus` composites every corpus file
and compares it with the merged image Photoshop stored in it (pass: every channel within 2/255).
"Off" below counts pixels off by more than 2/255. The Photoshop oracles
(`corpus/photoshop/effects`, from https://github.com/storytold/photocraft-corpus) each isolate
one effect setting on a 192 × 192 document; single rows were compared pixel by pixel to tell
the candidate formulas apart.

## Formulas now in Prism

| Part | Research notes | Prism (`effects.rs`) | Status |
|---|---|---|---|
| Offset (shadows, satin) | `dx = −round(d·cos θ)`, `dy = round(d·sin θ)`, whole pixels, halves away from zero; whole-degree angles from an exact cosine / sine table | `offset`: `f64`, snapped to the half it means. In `f32`, 5 px at 60° or 210° rounded the wrong way (2 instead of 3) | as written |
| Soft blur | tent of radius `r = clamp(ceil(size), 2, 250)`, weights `(r − |k|)/r²`, two running sums | `blur_radius`, `tent_kernel`, `tent_line`; also the smooth bevel's height map, satin and Soften | as written; size 0 stays unblurred (not in the notes) |
| Spread / choke | whole-pixel dilation by the 5 × 5 chamfer transform | dilation `round(size × spread)`, the blur gets the rest `size − r` (was `size × (1 − spread)`) | measured: ag-psd group-drop-shadows 4785 → 0 off |
| Precise glow | alpha seeded as a signed value (`(a·512 − 65280)/2`), 5 × 5 chamfer, `32640 / (1 + s)` per pixel | `PreciseGlow`: seeds start `2(1+s)(1−a)` px away, strength `½·clamp(1 − (d − r)/(1+s))` before the Range (`s = size − r`); centre source is the complement | formula from the notes; the ½ and `s` measured (outer-glow-precise-spread40: solid for 6 px, fades over 11; inner-glow-center-precise-choke30: rises over 14) |
| Chisel height | the same distance transform | `bevel_chisel_h` of the seeded fields: half of a ramp over `1 + size` (`1 + ⌈size/2⌉` for emboss styles) | measured: inner chisel lower facet takes 71 % of the shadow, as Photoshop |
| Bevel slope | 3 × 3 Prewitt on 16-bit heights, `c = 1530 / D`, `D = max(1, depth × size)` | `prewitt`, `bevel_geom` (`D ≥ 1`) | as written |
| Emboss size | halved (arithmetic shift of the 16.16 size) | `⌈size/2⌉` for the blur and for `D` of smooth embosses; chisel embosses keep the whole size in `D` | measured: emboss-smooth-colored-normal 2438 → 3 off with the round-up, layer_effects 428 → 0 |
| Light vector, highlight / shadow split | `L = 255.9·(cos a cos e, sin a cos e, sin e)`; split at the flat value (estimate) | unchanged (already the same) | as written |
| Direction Down | angle + 180° | depth sign (equivalent) | as written |
| Outer bevel reach | — | shades the pixel just past the blur's reach when a neighbour is raised, as emboss styles already did | measured: bevel-outer-smooth 0.90 % → 0, bevel-outer-on-gradient-bg 0.68 % → 0 |
| Satin | `|A − B|` of the shape blurred and shifted both ways, inverted on request, × alpha | unchanged (full distance each way; half measured far worse) | as written |
| Noise | `int(noise %) × 255 × 2.56` | whole percents (0.5 % is no noise); the speckle itself unchanged | magnitude as written; per-pixel pattern is an estimate in the notes |
| Contour table | 256 entries, linear interpolation | unchanged (`curve_lut` + linear lookup) | as written |

## Results (2026-11-17)

Pass counts: Photoshop oracles 145 → 151 / 258 (floor raised to 151), io corpus 149 / 170 and
psd-tools 239 / 309 unchanged. Newly passing: text-with-layer-styles, bevel-inner-smooth,
bevel-outer-smooth, bevel-outer-on-gradient-bg, bevel-emboss-smooth,
bevel-stroke-emboss-inside-stroke. Pixels off, before → after:

| Oracle | Before | After |
|---|---|---|
| outer-glow-precise-spread40 | 19.8 % | 7.1 % |
| inner-glow-center-precise-choke30 | 18.3 % | 10.0 % |
| bevel-inner-chisel-hard / soft | 20.0 / 21.1 % | 8.7 / 10.6 % |
| bevel-outer-chisel-hard / soft | 13.7 / 13.6 % | 7.2 / 6.3 % |
| bevel-emboss-chisel-hard / soft | 19.7 / 21.6 % | 12.1 / 8.9 % |
| bevel-pillow-chisel-hard / soft | 20.0 / 22.2 % | 11.9 / 8.4 % |
| bevel-inner-smooth-a45-alt65-depth300 | 3.9 % | 0.35 % |
| effect-enums (psd-tools) | 7.1 % | 5.0 % |

Slightly worse (within 0.4 % of pixels): bevel-inner-gloss-double-ring-aa, satin-inverted-ramp-peak-aa,
bevel-stroke-chisel-hard.

## Unknowns

- **Chisel Soft.** Prism draws it as Chisel Hard: the extra blur it used to get moved every
  chisel-soft oracle further off. In Photoshop the two differ only at the outermost edge pixel and
  at creases; the rule is not in the notes.
- **Range on precise glows.** Both precise oracles use the default 50 % Range; the ½ before the
  Range is what makes them match, and other Ranges are untested.
- **Layer Knocks Out Drop Shadow.** The notes give `out = b < T ? min(b·(0xff000000/T) >> 16, a) : a`
  with an unknown threshold `T`; Prism keeps its knock-out through see-through fill.
- **Noise pattern**, **contour corner flags** and the **stacking order** are estimates in the notes;
  unchanged.
- **Satin with an anti-aliased contour** (satin-inverted-ramp-peak-aa, 33 % off) and **bevels on
  strokes** (bevel-stroke-*, ~30 % off) are the largest remaining gaps; neither is covered by the
  notes.
- **Defaults.** The notes read Photoshop's effect constructors (drop shadow 35 %, 90°, 3 px, 7 px;
  glow 35 % white; stroke 1 px inside black). Prism's new-effect defaults are still the dialog's
  classic values (75 %, 120°, 5 px …); which one Photoshop 2026's dialog shows on a fresh install
  has not been checked, so they are unchanged.
