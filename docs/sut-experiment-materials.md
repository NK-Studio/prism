# SUT experiment materials: fade type 2, length units, colour mixing type 2

Local CLIP STUDIO materials to use as references for the three behaviours still open in
[sut-tool-property-backlog.md](sut-tool-property-backlog.md) › B. Found 2026-11-04 on the
maintainer's library (444 materials) with

```sh
cargo test -p prism-engine --test csp_library_local local_csp_experiment_materials -- --ignored --nocapture
```

which imports every ready brush and lists the ones whose source values (now retained in
`SutBrushMetadata.parameters`: `BrushInOutType`, `BrushUseIn/Out`, `BrushIn/OutRatio`,
`BrushIn/OutLengthUnit`, `BrushSpraySizeUnit`, `DualSizeUnit`, `DualSpraySizeUnit`,
`BrushWaterColor2`, …) hit one of the cases. Full values and import warnings:
`target/csp-check/experiment-materials.json` (ignored, local): 23 candidates plus 19 type-0/1
mixing controls. Corrected 2026-11-04 after review: R4_FIRE 1 is not a mixing control and
城シルエット is not a usable unit probe as stored (see below).

## 1. Start/end fade (`BrushInOutType` = 2)

**Result (2026-11-14, CSP PAINT PRO 5.0.4).** The setting is "How to specify" = **Fade** (0 by
length, 1 by percentage, 2 Fade). In Fade the Starting row and "Starting and ending by speed"
are greyed out; only Ending (194 for 물번짐구름 브러쉬, target Brush density with minimum 37 %)
is used. Measured with the brush at size 40, black, mouse strokes of 450 px on a 512 × 512 px
document: the density is full at the start, falls to a lower level by about 200 px and stays
there to the end of the stroke; fast (300 ms) and slow (1500 ms) strokes are the same, and every
new stroke starts at full density again. So the targets fall from full to their minimum over
the Ending length **measured from the start of the stroke**, by distance, and the stroke does
not run out. Prism now imports this as `CspInOutMode::Fade` (live and committed strokes alike).
The curve between is not fitted: it is level at first and drops late, which the quarter sine of
the other modes also does. Captures: `target/sut-experiments/fade-2.png`,
`labels/cloud-inout.png` (local, ignored).

| Material | Collection | Values | Today in Prism |
|---|---|---|---|
| 물번짐구름 브러쉬 | (own) | in on, out on, `BrushInLength` 20, `BrushOutLength` 194.44 (unit 0 = px) | warns "type 2 (fade) is not supported"; start/end dropped entirely |

The only fade brush in the library. Control for comparison: 背景の木〜 (in/out on, type 0, length 20/20 px).

Experiment: in CSP, draw slow and fast horizontal strokes of fixed length (e.g. 300 px and
1000 px) with 물번짐구름 at size, opacity and colour fixed; repeat with in/out off. Questions:
does the fade scale opacity, size or both; is the length in px of path or a time; does the out
length 194.44 (not an integer) suggest a converted unit; how does it combine with
`BrushInOutTarget`.

## 2. CSP length units (`*Unit` = 2)

**Result (2026-11-14, CSP PAINT PRO 5.0.4): unit 2 is pixels.** 茂み1 (`BrushSize` 100,
`BrushSizeUnit` 2) drawn with one horizontal mouse stroke at 100 % zoom on 512 × 512 px
documents: at 350 dpi the stroke is 75–90 px tall per column (the tip's silhouette), and at
72 dpi its regular part is about 90–100 px tall. A physical unit would give 100 mm = 1378 px
at 350 dpi. Lengths with unit 2 therefore do not depend on the resolution, and Prism reading
them as pixels is right; no conversion is needed. Every non-zero unit in the local library is 2
(`BrushSizeUnit` 16 presets, `BrushSpraySizeUnit` 20, `BrushWaterEdgeRadiusUnit` 8,
`BrushInLengthUnit` 3). The 600 dpi run was skipped (350 dpi already separates pixels from
mm/pt); the 72 dpi path was uneven because the real pointer moved during the drag. Captures:
`target/sut-experiments/unit-72.png`, `unit-350.png` (local, ignored).

Every non-pixel unit in the library is **2**. Its meaning is not traced (candidates: mm, a
percentage of the brush size, or points). Prism uses the stored number as pixels.

**Use the brush size to settle it.** `BrushSize` is always applied, so `BrushSizeUnit` = 2 shows
up in every stroke:

| Material | Collection | Unit keys = 2 | Value |
|---|---|---|---|
| **茂み1** (best probe) | 3分で茂みブラシ | `BrushSizeUnit`, `BrushSpraySizeUnit` | size 100 |
| 茂み2–3, 茂み下, 茂み下白黒, 葉っぱブラシ 1–3 | 3分で茂みブラシ | same (+ `BrushWaterEdgeRadiusUnit` on 葉っぱブラシ, 葉っぱブラシ3) | |
| 가을낙엽_도토리, _씨앗, _낙엽 | [기간한정무료] 가을낙엽 브러쉬 | `BrushSizeUnit`, `BrushSpraySizeUnit` | |
| 背景の木〜 | 異世界ファンタジー古城シルエットセット | `BrushSizeUnit`, `BrushSpraySizeUnit` | |
| R4_FIRE 1–2, R4_SPARKS 1–3 | FIRE BRUSH PACK | `BrushSpraySizeUnit`, `BrushWaterEdgeRadiusUnit` | |

Experiment: draw one stroke with 茂み1 (size 100) on canvases of the same pixel size at 72, 350
and 600 dpi, and read the size and unit CSP's Sub Tool Detail shows. A physical unit (mm/pt)
changes the drawn width with resolution; pixels would not.

城シルエット, 城シルエット 窓 and 城シルエット もやけ store a start length of 200 with
`BrushInLengthUnit` 2, but their start and end are **off** (`BrushUseIn` = `BrushUseOut` = 0), so
the stored length never reaches a stroke. Use them for the start/end unit only after turning
start/end on in CSP. Their end length 20 with unit 0 is the default stored by almost every brush
in the library and is not a meaningful comparison.

## 3. Colour mixing type 2 (`BrushWaterColor` = 2)

**Result (2026-11-14, CSP PAINT PRO 5.0.4).** The modes are Blend (0), Running color (1) and
**Smear** (2). In Smear, Density of paint is greyed out and Blending mode is available; in
Blend it is the other way round. Measured with NANAGOM PEN (size 25, amount of paint 78, colour
stretch 88, perceptual mixing) drawing green (99,190,143) across a blue (39,34,153) and a red
(191,100,100) band, fast mouse strokes, centre-line colours:

| Mode | On blue | On red | Opaque |
|---|---|---|---|
| Smear | (91,176,145) | (110,182,140) | yes |
| Blend, density 100 | the same within 1 level | the same | yes |
| Blend, density 30 | thinned | thinned | no (over white 193,229,211 → 124,200,161) |
| Smear with the stored density at 30 | as Smear at 100 | as Smear at 100 | yes |
| Running color | pure green | pure green | yes |

So Smear mixes exactly as Blend with the density of paint at 100 %, whatever value is stored.
Prism now imports type 2 that way (no warning). Running color (type 1) picked nothing up in
this setup and is still approximated by blending. Slow strokes were disturbed by the real
pointer and are not used. Captures: `target/sut-experiments/mix-strokes.png`,
`mix-density.png`, `labels/nanagom-ink.png` (local, ignored).

Mixing is on when `BrushUseWaterColor` (or `BrushUseWaterColor2`) is set; the type is only
meaningful then.

| Material | Collection | Mixing | Also |
|---|---|---|---|
| **NANAGOM PEN** | (own) | on, type 2 | simplest case: no spray, no other warnings |
| R4_SPARKS 1, 2, 3 | FIRE BRUSH PACK | on, type 2 | spray, unit-2 sizes, post correction |
| R4_FIRE 2 | FIRE BRUSH PACK | on, type 2 | as above |
| 물번짐구름 브러쉬 | (own) | stored type 2, mixing **off** | not a mixing reference |

Controls with mixing **on** and type 0 (found by the same scan, `mixingControls` in the JSON):
Hard Clouds 1, Hard Clouds 2, Cluttered Leaves (Oyun Nature), Rock Detailer (Painterly Rock
Texture Brushes) and the 15 CHERU highlight brushes (Hair Highlight 1–3, Moon, Sparkle, Star, Dot,
Soft, Natural, Heartbeat, Wave, Square, Anime, Lines, Cloud Highlight). R4_FIRE 1 is **not** a
control: its mixing is off (`BrushUseWaterColor` = 0), so R4_FIRE 1 vs 2 compares mixing on/off,
not type 2 against another type.

Prism approximates type 2 with type 0 blending (warning "color mixing type 2 is approximated
by blending"). Best experiment: in CSP, draw NANAGOM PEN over a two-colour background (half red,
half blue, plus a soft gradient), slow and fast, crossing the boundary; then change only its
mixing type to 0 and 1 and repeat. Compare how far the picked colour is carried and whether type 2
reads the canvas once per stroke, per dab or under the whole tip. A type-0 brush from the list
above gives a second, independent type-0 reference.

## Notes

- Names and values come from the local library only; no material, tip or texture is copied
  into the repository. Keep captures under ignored `target/` and record evidence in
  [sut-analysis.md](sut-analysis.md), as for earlier CSP comparisons.
- Re-run the scan after importing new materials; the test prints a table and rewrites the
  JSON.
