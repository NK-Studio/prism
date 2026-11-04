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
| ~~P1~~ done 2026-11-04 | 1020 | `BrushFlow` | Brush density | 83 | `flow` | slider 0–100 % |
| ~~P1~~ done 2026-11-04 | 1060 | `BrushRotation` | Direction | 64 | `angle` (negated); also `csp_ribbon` quarter turns | slider −180–180° (keep ribbon in sync) |
| ~~P1~~ done 2026-11-04 | 1030 | `BrushHardness` | Hardness | 22 | `hardness` | slider 0–100 %; re-derive auto spacing (1042) when it changes |
| ~~P1~~ done 2026-11-04 | 1131 | `BrushSpraySize` (+`Unit`) | Particle size | 19 | `csp_spray.size` | length slider, px; unit conversion is open (see B) |
| ~~P1~~ done 2026-11-04 | 1133 | `BrushSprayDensity` | Particle density | 16 | `csp_spray.count` | integer slider, capped at `csp_spray::MAX_PARTICLES` |
| ~~P1~~ done 2026-11-04 | 1002 | `BrushSizeSyncViewScale` | Specify by size on screen | 15 | `csp_size_by_view` | checkbox |
| ~~P2~~ done 2026-11-04 | 1102 | `BrushChangePatternColor` | Change brush tip color | 10 | `color_dynamics` (per tip) | checkbox enabling 1103/1105/1107 |
| ~~P2~~ done 2026-11-04 | 1103 | `BrushHueChange` | Hue | 7 | `color_dynamics.hue_jitter` | slider 0–180° |
| ~~P2~~ done 2026-11-04 | 1105 | `BrushSaturationChange` | Saturation | 7 | `color_dynamics.saturation_jitter` | slider 0–100 |
| ~~P2~~ done 2026-11-04 | 1107 | `BrushValueChange` | Luminosity | 7 | `color_dynamics.brightness_jitter` | slider 0–100 |
| ~~P2~~ done 2026-11-04 | 1135 | `BrushSprayBias` | Spray deviation | 9 | `csp_spray.bias` | slider 0–100 |
| ~~P2~~ done 2026-11-04 | 1130 | `BrushUseSpray` | Spray effect | 8 | `csp_spray` on/off | checkbox; off → `None`, on → defaults from source |
| ~~P2~~ done 2026-11-04 | 1061 | `BrushRotationInSpray` | Direction of particle | 8 | `csp_spray.angle` | slider −180–180° |
| ~~P2~~ done 2026-11-04 | 1052 | `BrushVerticalThicknes` | Thickness direction | 8 | `csp_tip_scale` axis | horizontal/vertical toggle beside 1050 |
| ~~P2~~ done 2026-11-04 | 1071 | `BrushPatternOrderType(2)` | Tip order | 6 | `csp_tip_order` | list: Repeat / Reverse / Do not repeat / Random |
| ~~P3~~ done 2026-11-04 | 1043 | `BrushContinuousPlot` | Continuous spraying | 3 | `build_up` | checkbox (CSP rate not traced) |
| ~~P3~~ done 2026-11-04 | 50 | `Stickness` | Stickiness | 3 | `csp_stabilizer.stickiness` | integer slider 0–100 |
| ~~P3~~ done 2026-11-04 | 1202 | `BrushWaterEdgeAlphaPower` | Opacity influence | 1 | `csp_water_edge.opacity` | already in 1201's advanced block: map the ID to that block |
| ~~P3~~ done 2026-11-04 | 1203 | `BrushWaterEdgeValuePower` | Darkness influence | 1 | `csp_water_edge.darkness` | same as 1202 |
| ~~P3~~ done 2026-11-04 | 1270 | `BrushChangeStrokeColor` | Change color per stroke | 1 | `color_dynamics` (per stroke) | checkbox; warn that per-tip + per-stroke together uses per-tip |
| ~~P2~~ done 2026-11-04 (from D) | 1074 | `BrushPatternReverseHorizontal` | Flip horizontal (brush tip) | 6 | `flip_x` | checkbox |
| ~~P2~~ done 2026-11-04 (from D) | 1075 | `BrushPatternReverseVertical` | Flip vertical (brush tip) | 2 | `flip_y` | checkbox |
| ~~P3~~ done 2026-11-04 (from D) | 1087 | `BrushColorMixingMode` | Color mixing method | 3 | `csp_mixing.perceptual` | toggle inside the colour-mixing block (needs `csp_mixing`) |
| ~~P3~~ done 2026-11-04 (from D) | 1911 | `DualBrushCompositeMode` | Dual brush blending mode | 2 | `dual_brush.mode` / additive modes (`csp_dual_mode`) | list in the dual sub-panel (1910) |
| ~~P3~~ done 2026-11-04 (from D) | 101 | `TextureForPlot` | Apply to each brush tip (texture) | 1 | `texture.each_tip` | checkbox in the texture block |
| ~~P3~~ done 2026-11-04 (from D) | 102 | `TextureCompositeMode` | Texture blending mode | 1 | `texture.csp_mode` | list of the CSP texture modes |
| ~~P3~~ done 2026-11-04 (from D) | 113 | `TextureReverseDensity` | Reverse density | 1 | `texture.invert` | checkbox in the texture block |
| ~~P3~~ done 2026-11-04 (from D) | 114 | `TextureStressDensity` | Emphasize density | 1 | `texture.csp_emphasize` (+0.5 contrast at render time; approximation, warns) | checkbox; faithful effect stays open |
| ~~P3~~ done 2026-11-04 (from D) | 1101 | `BrushSubColor` | Sub color (brush tip color change) | 1 | `csp_sub_color` (fixed) or `color_dynamics.fg_bg.jitter` (random) | slider 0–100 % beside 1103–1107 |

P1 status (2026-11-04): all six rows have Tool Property controls (`sut_panel::control`);
hardness re-derives automatic spacing, direction keeps a ribbon's quarter turns in sync, particle
size keeps a brush-synchronized ratio and is shown in pixels (unit conversion stays open, see B).
Test: `p1_rows_have_controls_and_survive_hostile_values`.

P2 status (2026-11-04): the nine group-A P2 rows have controls. Change brush tip color turns
per-tip colour jitter on/off (colour tips keep their colours); hue/saturation/luminosity edit the
jitter amounts; Spray effect off removes `csp_spray`, on restores the imported particle values;
thickness direction swaps the squashed axis; tip order applies to multi-tip brushes only.
Tests: `p2_rows_have_controls_and_survive_hostile_values`,
`spray_switched_on_restores_the_imported_particles_and_thickness_axis_follows_the_brush`.

Acceptance for A: none of these IDs produces an "Unsupported CSP setting" row; every control
changes the preview strip; save/reload of the preset keeps the edit.

## B. Partly supported (approximated or warns)

| Pri | ID | Key | CSP label | Presets | Today | To build |
|---|---|---|---|---|---|---|
| ~~P2~~ done 2026-11-04 | 1162 | `BrushInOutTarget` | Starting and ending: effect | 18 | targets decoded into `csp_in_out`; unknown targets warn | target checklist (size, opacity, density, …) |
| ~~P2~~ done 2026-11-04 | 1171 | `BrushInLength` (+`Unit`/`Ratio`) | Starting | 6 | pixels only; fade type 2 unsupported | length/percent slider; units; fade |
| ~~P2~~ done 2026-11-04 | 1181 | `BrushOutLength` (+`Unit`/`Ratio`) | Ending | 6 | as 1171 | as 1171 |
| ~~P2~~ done 2026-11-04 | 1080 | `BrushWaterColor` | Color mixing (mode) | 7 | type 2 approximated by blending | mode list; faithful type 2 |
| ~~P3~~ done 2026-11-04 | 1910 | `UseDualBrush` | Dual brush | 2 | `dual_brush`; several dual settings ignored with a warning | checkbox + dual sub-panel |
| ~~P3~~ done 2026-11-04 | 51000 | `DualSize` (+`Unit`) | Dual brush size | 1 | applied | slider inside the dual sub-panel |

Spray, start/end and water-edge lengths are read as pixels; CLIP STUDIO units (mm, pt, %
of canvas resolution) need the document resolution. One shared length-with-unit control
should cover 1131, 1171, 1181, 1201 and 51000.

## C. Not read at all (behaviour missing)

| Pri | ID | Key | CSP label | Presets | Notes |
|---|---|---|---|---|---|
| P2 (named row, behaviour open) | 1250 | `BrushUseReferLayer` | Refer to other layers (draw beside reference lines) | 12 | needs a reference-layer concept; today warns only |
| P2 (named row, behaviour open) | 1151 | `BrushRevision` (+`BrushUseRevision`) | Post correction | 6 | smoothing after the stroke; trace interaction with start/end |
| ~~P3~~ done 2026-11-04 | 1110 | `BrushRibbon` | Ribbon | 1 | rendered (`csp_ribbon`) but no control; add a checkbox (valid only with tip images and no spray) |
| P3 (named row, behaviour open) | 1500 | `StampAdjustPosition` | Adjust position (copy stamp) | 1 | copy-stamp sub-tool; also 1510 `StampSetReferPointMode` |
| ~~P2~~ done 2026-11-04 (from D) | 1003 | `BrushAtLeast1Pixel` | At least 1 pixel (inferred) | 12 | Chainmail pens. Imported into `csp_at_least_1px`: a dab under 1 px is laid as a full-ink 1 px dab instead of fading with its area; checkbox row. Behaviour inferred from the key name, not yet compared with CSP |
| P2 (from D) | 1230 | `BrushEnableSnap` | Enable snapping (inferred) | 6 | R4_FIRE, 木目ブラシ; whether the brush snaps to rulers/guides |
| P3 (from D) | 1120 | `BrushBlendPatternByDarken` | Blend tip images by darken (inferred) | 2 | 法線ブラシ 円/四角; overlapping tip images combine with Darken instead of normal |
| P3 (from D) | 1913 | `ChangeRGBByDual` | Dual brush affects colour (inferred) | 2 | D/Kaminari, D/Raigeki; dual brush changes RGB, not only alpha |
| P3 (from D) | 1022 | `BrushAdjustFlowByInterval` | Adjust density by spacing (inferred) | 1 | 물번짐구름 브러쉬; density compensated for the dab spacing |

## D. IDs with no traced key

None (2026-11-04). The former D rows were traced and moved to A (1074, 1075, 1087, 1911, 101,
102, 113, 114, 1101: already imported, panel work only) and C (1003, 1230, 1120, 1913, 1022:
not read). Their keys come from the local ID → key table `target/rea-tools/csp/display-properties.json`
(ignored, local), which pairs each property ID with the key name in CLIP STUDIO's native settings
reader/writer (method in [sut-analysis.md](sut-analysis.md) › property-name initialization and
settings reader/writer). The table agrees with every ID already known from the default
sub-tools (1020, 1030, 1102, 1110, 1131, 1133, 1151, 1171, 1250, 1910, 51000). The keys are
traced; the CSP labels marked "inferred" are read from the key names, not from CSP's UI.
New unknown IDs found by a re-scan go here until their key is traced.

## Status 2026-11-04 (after P3)

- Every ID in groups A, B and C now has a named Tool Property row; none is drawn as
  "Unsupported CSP setting". Groups A and B and ribbon (1110) edit the brush; starting/ending
  can be turned on, switched between length (px) and percent of the stroke, and target any of
  size, density, texture density, particle size and density with a minimum.
- Still behaviour work, shown as named read-only rows: refer to other layers (1250), post
  correction (1151), copy-stamp adjust position (1500). Fade type 2, CSP length units and a
  faithful colour mixing type 2 stay open (B).
- Group D IDs still need their keys traced before rows can be built. (Superseded the same day:
  all D keys traced; the rows are now in A and C marked "from D" and still show as
  "Unsupported CSP setting" until built.)
- Tests: `remaining_backlog_rows_are_named_and_survive_hostile_values`,
  `ending_switched_off_last_removes_the_taper_and_on_starts_with_a_size_target`.

## Status 2026-11-04 (after group D)

Done since the previous status (commit f4306f6): every group-D key traced; 1074/1075 flip
checkboxes; 1003 at least 1 pixel (`csp_at_least_1px`, imported and editable). Tests:
`csp_at_least_1px_lays_sub_pixel_dabs_as_full_1px_dabs`, `flip_and_at_least_1px_rows_toggle_the_brush`.

### Remaining work, in the recommended order

Updated 2026-11-04 after commit 73a6618. Done: groups A and B rows, ribbon, every former group-D
key, 1003/1074/1075, and the seven imported group-D settings (101, 102, 113, 114, 1087, 1911,
1101; test `imported_texture_mixing_dual_and_sub_color_rows_edit_the_brush`). Emphasize density
is now `Texture::csp_emphasize`, applied by `Texture::effective_contrast` at render time.
Commits f4306f6, e024e59 and 73a6618 are local on `codex/sut-brush-response`, not pushed yet.

**0. Check what just landed (before new work).**
- Look at the new rows in the running app or the offscreen snapshot (Tool Property, Advanced
  Settings) with real materials: Chainmail pen (1003), Deciduous Trees (1074), 法線ブラシ (1075),
  Crystal sparkles/base (101, 102, 113, 114, 1101), R4_SPARKS (1087), D/Kaminari (1911). Check
  layout in Korean and in a narrow panel.
- Re-run the local scan (below): only C IDs should still be listed by `VariantShowParam` without
  a control.

**1. Behaviour still missing (C).** Each needs rendering or tool work, not only a row. Today
1250, 1151 and 1500 are named read-only rows; 1230, 1120, 1913 and 1022 still show as
"Unsupported CSP setting" because their keys are not in `PARAMETER_KEYS`.

| Order | ID | Key | Setting | Presets | Work | Done when |
|---|---|---|---|---|---|---|
| 1 | 1230 | `BrushEnableSnap` | Enable snapping (inferred) | 6 | Import into a brush flag; when off, the stroke ignores ruler/guide snapping. Hook: `engine::special_ruler_snap` (`StrokeSnapper`, used by `ui-egui/src/canvas.rs` and `special_ruler_ui.rs`) and `engine/src/snap.rs`. Checkbox row. First confirm in CSP what the option snaps to. | a brush with snapping off draws freely across a ruler; on snaps as today; test for both |
| 2 | 1151 | `BrushRevision` (+`BrushUseRevision`) | Post correction | 6 | Smooth the committed stroke after pen-up (CSP's post correction), independent of the live stabilizer (`csp_stabilizer`, `BrushSettings::csp_correction`). Trace how it interacts with start/end taper and pressure. Replace the read-only row with an on/off + level control. | R4_FIRE and 木目ブラシ strokes smoothed after pen-up; warning removed; ABR/Photoshop smoothing unaffected |
| 3 | 1250 | `BrushUseReferLayer` | Refer to other layers | 12 | Largest: needs a reference-layer concept in the document (CSP's "Set as reference layer") before the brush can stop at its lines. Write a short design first (doc model field, UI, `.prism`/PSD round-trip). | design agreed; then brush stops at reference-layer lines |
| 4 | 1120 | `BrushBlendPatternByDarken` | Blend tip images by darken (inferred) | 2 | Overlapping tip images of one stroke combine with Darken instead of normal. Confirm in CSP with 法線ブラシ 円/四角, then add to the dab compositor and a checkbox row. | matched CSP capture of 法線ブラシ |
| 5 | 1913 | `ChangeRGBByDual` | Dual brush affects colour (inferred) | 2 | The dual brush changes RGB as well as alpha. Confirm with D/Kaminari, D/Raigeki; then the dual brush compositor and a checkbox in the dual block. | matched CSP capture |
| 6 | 1022 | `BrushAdjustFlowByInterval` | Adjust density by spacing (inferred) | 1 | Density compensated for dab spacing (물번짐구름 브러쉬). Measure in CSP how density changes with spacing, then scale `flow` per dab. | stroke density stays constant across spacing changes as in CSP |
| 7 | 1500 | `StampAdjustPosition` (+1510 `StampSetReferPointMode`) | Adjust position (copy stamp) | 1 | Copy-stamp sub-tool option; belongs with the clone stamp tool work, not the brush. | copy stamp honours the option |

Rows marked "inferred" have traced keys; their meaning comes from the key name. Check each in
CSP's UI before implementing.

**2. Approximations that need a CSP experiment first (B).** Procedures and materials:
[sut-experiment-materials.md](sut-experiment-materials.md).
- Fade type 2 (`BrushInOutType` 2): 물번짐구름 브러쉬; today start/end is dropped with a warning.
- Length unit 2 (`*Unit` = 2): 茂み1 (size 100) at 72/350/600 dpi; then one shared
  length-with-unit conversion for 1000, 1131, 1171, 1181, 1201 and 51000.
- Colour mixing type 2: NANAGOM PEN with only the type switched; type-0 controls are listed in
  the experiment document.
- Emphasize density (114) stays a +0.5 contrast approximation until compared with CSP.

**3. Verification only.**
- 1003 at least 1 pixel: compare with CSP using the Chainmail pens (behaviour inferred).
- Confirm the "inferred" labels against CSP's UI and correct the English and Korean labels.

**4. Housekeeping.**
- `save_ui` has three failing ui-egui tests on this machine (temp folder path with a trailing
  backslash); they fail on the base commit too and are unrelated to SUT work.
- Workspace clippy has pre-existing `collapsible_match` and one boolean lint in other ui-egui
  files (analysis_ui, canvas, gradient_ui, native_dialogs, transform_tool) and in
  `prism-affinity`; run clippy with `--no-deps` on SUT crates until they are fixed.

When a row lands, mark it done in its table and update this list.

## Re-measuring

The counts come from importing every ready material with `brush.cspLibrary.list` /
`brush.cspLibrary.import` (as `crates/engine/tests/csp_library_local.rs` does) and listing, per
preset, each `sut.original.visible` ID for which `prism_paint::sut::is_known_parameter` is false
(non-brush families also check `sut_catalog::item`/`owner`). Re-run after each group lands and
update the tables.
