# SUT Tool Property backlog: "Unsupported CSP setting" IDs

Spec sheet for the CLIP STUDIO brush property IDs that Prism's Tool Property panel shows as
**"Unsupported CSP setting N"** (Korean: "지원하지 않는 CSP 항목 N"). Each row says what the
item is, what the importer does with it today, and what has to be built. Work top to bottom.

Measured 2026-11-04 on the maintainer's local CLIP STUDIO library (444 materials, 154 imported
brush-family presets, 2 other sub-tools). The other sub-tools showed no unknown IDs. Counts are
the number of presets whose `VariantShowParam` lists the ID. ID → key mapping comes from
[csp-subtool-gaps.md](csp-subtool-gaps.md) › Appendix. Related: [sut-compatibility-audit.md](sut-compatibility-audit.md),
[sut-analysis.md](sut-analysis.md).

## Why a row is "unsupported"

The brush-family panel (`crates/ui-egui/src/sut_panel.rs`, `ITEMS`) has controls for 18 IDs
only: 1000, 10, 30, 1081, 1083, 1085, 20, 1050, 1040, 1042, 100, 110, 120, 1201, 40, 41, 42,
1241 (`prism_paint::sut::PARAMETER_KEYS`). Any other ID in `VariantShowParam` is drawn as a
bare "Unsupported CSP setting N" row. The non-brush catalogue (`prism_paint::sut_catalog`) is
not used for brush-family presets.

"Unsupported" in the panel does **not** always mean the stroke ignores the setting: group A
below already renders; only the editable control is missing.

## Common checklist for adding a panel row

1. Add `(id, key)` to `PARAMETER_KEYS` in `crates/paint/src/sut.rs` (and any compound source
   keys to `RETAINED_KEYS`) so the source value is kept and `available()` finds it.
2. Add the row to `sut_panel::ITEMS` with CLIP STUDIO's label and category (canonical CSP
   order), and a `control()` arm that edits the `BrushSettings` field the importer fills.
3. Edits go through `brush_panel::commit_gesture` (`tools.setBrush`), never the source values.
4. Korean label and category in `crates/ui-egui/src/i18n/ko.tsv` (the file has mixed line
   endings: split on `\r?\n` when scripting). `catalog_labels_and_categories_have_korean_translations` must pass.
5. Tests: the row replaces the unsupported row; editing changes the brush and the preview
   pixels; hostile source values (NaN, negative, huge) clamp without panicking.
6. Re-run the local scan (below) and update the counts here.

## A. Rendered on import, no panel control (panel work only)

| Pri | ID | Key | CSP label | Presets | Imported into | Control to build |
|---|---|---|---|---|---|---|
| P1 | 1020 | `BrushFlow` | Brush density | 83 | `flow` | slider 0–100 % |
| P1 | 1060 | `BrushRotation` | Direction | 64 | `angle` (negated); also `csp_ribbon` quarter turns | slider −180–180° (keep ribbon in sync) |
| P1 | 1030 | `BrushHardness` | Hardness | 22 | `hardness` | slider 0–100 %; re-derive auto spacing (1042) when it changes |
| P1 | 1131 | `BrushSpraySize` (+`Unit`) | Particle size | 19 | `csp_spray.size` | length slider, px; unit conversion is open (see B) |
| P1 | 1133 | `BrushSprayDensity` | Particle density | 16 | `csp_spray.count` | integer slider, capped at `csp_spray::MAX_PARTICLES` |
| P1 | 1002 | `BrushSizeSyncViewScale` | Specify by size on screen | 15 | `csp_size_by_view` | checkbox |
| P2 | 1102 | `BrushChangePatternColor` | Change brush tip color | 10 | `color_dynamics` (per tip) | checkbox enabling 1103/1105/1107 |
| P2 | 1103 | `BrushHueChange` | Hue | 7 | `color_dynamics.hue_jitter` | slider 0–180° |
| P2 | 1105 | `BrushSaturationChange` | Saturation | 7 | `color_dynamics.saturation_jitter` | slider 0–100 |
| P2 | 1107 | `BrushValueChange` | Luminosity | 7 | `color_dynamics.brightness_jitter` | slider 0–100 |
| P2 | 1135 | `BrushSprayBias` | Spray deviation | 9 | `csp_spray.bias` | slider 0–100 |
| P2 | 1130 | `BrushUseSpray` | Spray effect | 8 | `csp_spray` on/off | checkbox; off → `None`, on → defaults from source |
| P2 | 1061 | `BrushRotationInSpray` | Direction of particle | 8 | `csp_spray.angle` | slider −180–180° |
| P2 | 1052 | `BrushVerticalThicknes` | Thickness direction | 8 | `csp_tip_scale` axis | horizontal/vertical toggle beside 1050 |
| P2 | 1071 | `BrushPatternOrderType(2)` | Tip order | 6 | `csp_tip_order` | list: Repeat / Reverse / Do not repeat / Random |
| P3 | 1043 | `BrushContinuousPlot` | Continuous spraying | 3 | `build_up` | checkbox (CSP rate not traced) |
| P3 | 50 | `Stickness` | Stickiness | 3 | `csp_stabilizer.stickiness` | integer slider 0–100 |
| P3 | 1202 | `BrushWaterEdgeAlphaPower` | Opacity influence | 1 | `csp_water_edge.opacity` | already in 1201's advanced block: map the ID to that block |
| P3 | 1203 | `BrushWaterEdgeValuePower` | Darkness influence | 1 | `csp_water_edge.darkness` | same as 1202 |
| P3 | 1270 | `BrushChangeStrokeColor` | Change color per stroke | 1 | `color_dynamics` (per stroke) | checkbox; warn that per-tip + per-stroke together uses per-tip |

Acceptance for A: none of these IDs produces an "Unsupported CSP setting" row; every control
changes the preview strip; save/reload of the preset keeps the edit.

## B. Partly supported (approximated or warns)

| Pri | ID | Key | CSP label | Presets | Today | To build |
|---|---|---|---|---|---|---|
| P2 | 1162 | `BrushInOutTarget` | Starting and ending: effect | 18 | targets decoded into `csp_in_out`; unknown targets warn | target checklist (size, opacity, density, …) |
| P2 | 1171 | `BrushInLength` (+`Unit`/`Ratio`) | Starting | 6 | pixels only; fade type 2 unsupported | length/percent slider; units; fade |
| P2 | 1181 | `BrushOutLength` (+`Unit`/`Ratio`) | Ending | 6 | as 1171 | as 1171 |
| P2 | 1080 | `BrushWaterColor` | Color mixing (mode) | 7 | type 2 approximated by blending | mode list; faithful type 2 |
| P3 | 1910 | `UseDualBrush` | Dual brush | 2 | `dual_brush`; several dual settings ignored with a warning | checkbox + dual sub-panel |
| P3 | 51000 | `DualSize` (+`Unit`) | Dual brush size | 1 | applied | slider inside the dual sub-panel |

Spray, start/end and water-edge lengths are read as pixels; CLIP STUDIO units (mm, pt, %
of canvas resolution) need the document resolution. One shared length-with-unit control
should cover 1131, 1171, 1181, 1201 and 51000.

## C. Not read at all (behaviour missing)

| Pri | ID | Key | CSP label | Presets | Notes |
|---|---|---|---|---|---|
| P2 | 1250 | `BrushUseReferLayer` | Refer to other layers (draw beside reference lines) | 12 | needs a reference-layer concept; today warns only |
| P2 | 1151 | `BrushRevision` (+`BrushUseRevision`) | Post correction | 6 | smoothing after the stroke; trace interaction with start/end |
| P3 | 1110 | `BrushRibbon` | Ribbon | 1 | rendered (`csp_ribbon`) but no control; add a checkbox (valid only with tip images and no spray) |
| P3 | 1500 | `StampAdjustPosition` | Adjust position (copy stamp) | 1 | copy-stamp sub-tool; also 1510 `StampSetReferPointMode` |

## D. IDs with no traced key

These appear in local brushes but are absent from the default sub-tool tables, so the key and
meaning must first be traced (within the scope set in [sut-analysis.md](sut-analysis.md)).
Record evidence there, then move each row to A–C.

| ID | Presets | Seen on |
|---|---|---|
| 1003 | 12 | Chainmail pens (next to 1002: likely a size-unit/view item) |
| 1230 | 6 | R4_FIRE, 木目ブラシ |
| 1074 | 6 | Deciduous Trees, Evergreens, Messy Branches (next to tip order 1071) |
| 1087 | 3 | R4_SPARKS (colour-mixing range 1081–1085) |
| 1075 | 2 | 法線ブラシ 円/四角 |
| 1120 | 2 | 法線ブラシ 円/四角 |
| 1911, 1913 | 2 | D/Kaminari, D/Raigeki (dual brush group 1910) |
| 101, 102 | 1 | Crystal sparkles (texture group 100–120) |
| 113, 114 | 1 | Crystal base (texture group) |
| 1101 | 1 | Crystal base (colour jitter group 1102–1107) |
| 1022 | 1 | 물번짐구름 브러쉬 (density group 1020) |

## Re-measuring

The counts come from importing every ready material with `brush.cspLibrary.list` /
`brush.cspLibrary.import` (as `crates/engine/tests/csp_library_local.rs` does) and listing, per
preset, each `sut.original.visible` ID for which `prism_paint::sut::is_known_parameter` is false
(non-brush families also check `sut_catalog::item`/`owner`). Re-run after each group lands and
update the tables.
