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
