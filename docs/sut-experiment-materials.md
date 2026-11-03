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
`target/csp-check/experiment-materials.json` (ignored, local). 23 candidates.

## 1. Start/end fade (`BrushInOutType` = 2)

| Material | Collection | Values | Today in Prism |
|---|---|---|---|
| 물번짐구름 브러쉬 | (own) | in on, out on, `BrushInLength` 20, `BrushOutLength` 194.44 (unit 0 = px) | warns "type 2 (fade) is not supported"; start/end dropped entirely |

The only fade brush in the library. Controls for comparison: 背景の木〜 (in/out on, type 0,
length 20/20 px) and 城シルエット (type 0, in length 200 with unit 2).

Experiment: in CSP, draw slow and fast horizontal strokes of fixed length (e.g. 300 px and
1000 px) with 물번짐구름 at size, opacity and colour fixed; repeat with in/out off. Questions:
does the fade scale opacity, size or both; is the length in px of path or a time; does the out
length 194.44 (not an integer) suggest a converted unit; how does it combine with
`BrushInOutTarget`.

## 2. CSP length units (`*Unit` = 2)

Every non-pixel unit in the library is **2**. Its meaning is not traced (candidates: mm, a
percentage of the brush size, or points). Prism uses the stored number as pixels.

| Material | Collection | Unit keys = 2 | Lengths |
|---|---|---|---|
| 城シルエット, 城シルエット 窓, 城シルエット もやけ | 異世界ファンタジー古城シルエットセット | `BrushInLengthUnit`, `BrushSizeUnit`, `BrushSpraySizeUnit` (+ `BrushWaterEdgeRadiusUnit` on もやけ) | in length 200, out 20 (out unit 0) |
| 茂み1–3, 茂み下, 茂み下白黒 | 3分で茂みブラシ | `BrushSizeUnit`, `BrushSpraySizeUnit` | size 100 |
| 葉っぱブラシ, 葉っぱブラシ3 | 3分で茂みブラシ | + `BrushWaterEdgeRadiusUnit` | |
| 葉っぱブラシ2 | 3分で茂みブラシ | `BrushSizeUnit`, `BrushSpraySizeUnit` | |
| 背景の木〜 | 異世界ファンタジー古城シルエットセット | `BrushSizeUnit`, `BrushSpraySizeUnit` | |
| 가을낙엽_도토리, _씨앗, _낙엽 | [기간한정무료] 가을낙엽 브러쉬 | `BrushSizeUnit`, `BrushSpraySizeUnit` | |
| R4_FIRE 1–2, R4_SPARKS 1–3 | FIRE BRUSH PACK | `BrushSpraySizeUnit`, `BrushWaterEdgeRadiusUnit` | |
| コピースタンプツール | 写真からシームレス画像作成キット | `BrushSizeUnit` | |

Experiment: open 城シルエット in CSP's Sub Tool Detail and read the unit shown next to the
start length (and the brush size). Then draw on canvases of the same pixel size at 72, 350
and 600 dpi: a physical unit (mm/pt) changes the drawn length with resolution, a brush-relative
unit changes it with brush size. 城シルエット is the best probe: its start length (200, unit 2)
and end length (20, unit 0) differ in unit inside one brush.

## 3. Colour mixing type 2 (`BrushWaterColor` = 2)

| Material | Collection | Mixing | Also |
|---|---|---|---|
| NANAGOM PEN | (own) | on, type 2 | simplest case: no spray, no other warnings |
| R4_SPARKS 1, 2, 3 | FIRE BRUSH PACK | on, type 2 | spray, unit-2 sizes, post correction |
| R4_FIRE 2 | FIRE BRUSH PACK | on, type 2 | as above; R4_FIRE 1 is the same pack with type ≠ 2 (control) |
| 물번짐구름 브러쉬 | (own) | stored type 2, mixing **off** | not a mixing reference |

Prism approximates type 2 with type 0 blending (warning "color mixing type 2 is approximated
by blending"). Experiment: with NANAGOM PEN, draw over a two-colour background (e.g. half red,
half blue, and a soft gradient) in one stroke crossing the boundary, slow and fast, then switch
the CSP mixing type to 0/1 and repeat. Compare how far the picked colour is carried and whether
type 2 reads the canvas once per stroke, per dab, or under the whole tip. R4_FIRE 1 vs R4_FIRE 2
gives a same-pack A/B.

## Notes

- Names and values come from the local library only; no material, tip or texture is copied
  into the repository. Keep captures under ignored `target/` and record evidence in
  [sut-analysis.md](sut-analysis.md), as for earlier CSP comparisons.
- Re-run the scan after importing new materials; the test prints a table and rewrites the
  JSON.
