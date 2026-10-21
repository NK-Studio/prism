# CLIP STUDIO special rulers: perspective and parallel snapping (design)

Status: phase 1 implemented 2026-10-21 (model, `.prism`, snapping, commands, canvas snap and
overlay). Phase 2 (CLIP STUDIO import) implemented 2026-10-21: the "Isometric ruler and grid"
material adds from Window › Extensions › CLIP STUDIO Materials (Layers filter) or
`layer.cspMaterial.import`. Phase 3 handle editing implemented 2026-10-21
(`special_ruler_ui::handles`): with the Move tool on the ruler layer each parallel direction has
an anchor (drag: move its guides) and an angle handle 64 pt along the line (drag: turn the
direction and its guides around the anchor, ⇧ = 15° steps); converging points drag too. A
direction with no guide on the document gets a virtual anchor at the document centre, so its
line shows. `layer.specialRuler.edit` takes `guides`. Still design: the layer-row badge,
`BrushEnableSnap` and phase 4 (more ruler kinds).

Goal: brush strokes in Prism snap to CLIP STUDIO PAINT-style special rulers, and the
downloaded "Isometric ruler and grid" material (ASSETS content 1758445) loads into a Prism
document. Once loaded, painting along its three isometric directions works the way it does in
PAINT.

Photoshop has no equivalent (its Vanishing Point filter projects pixels onto planes and does not
constrain strokes), so this is a CLIP STUDIO compatibility feature on `codex/feature-sut-brush`.
It follows the same rules as the SUT work: Rust only, everything is a command, never panic on
material input.

## 1. Evidence

The evidence comes from read-only decoding of locally downloaded materials, using the same C2F
container framing Prism already implements (`crates/io/src/csp_container.rs`), plus PAINT's
public manual for behaviour. No REA or Ghidra work was used for this design. Decoded databases
stay outside the repository.

### Where the material is

`CLIPStudioCommon/Material/56/04/59d3271d89-…/` is a catalog with two `image` items:

| Item | `systemtag` | Content |
|---|---|---|
| `2dedbf35ba-…/data/material_0.layer` "Isometric ruler" | `OtherLayer Ruler SpecialRuler PersRuler` | 1280×720 canvas, one layer carrying a perspective ruler |
| `8f65f2e32e-…/data/material_0.layer` "Isometric grid" | `Resizable` | raster image material (no ruler) |

A `.layer` file is C2F: `HEAD`, then a 5130-byte encoded `dATA` (Blowfish prefix), then a
plaintext `dATA`, then `TAIL`. Decoded, it is an SQLite "layer material" canvas that uses the
same table family as a `.clip` document: `Canvas`, `Layer`, `Offscreen`/`Mipmap*`,
`SpecialRulerManager`, `Ruler*`, `ParamScheme`, `ElemScheme`.

Two more ruler materials from "ISOMETRIC TOOLS" (content 1842998) were decoded for comparison:
`469ab6458b-…` (isometric 30°) and `9ba2b64bfb-…` (2:1 isometric).

### Ruler tables (from `ParamScheme`, identical in all three)

`Layer.SpecialRulerManager` points to a `SpecialRulerManager` row. That row has one
linked-list head per ruler kind. The rows of each kind are chained by `NextIndex`, and `0` ends
the chain.

| Kind | Head column | Columns |
|---|---|---|
| Parallel line | `FirstParallel` → `RulerParallel` | `Snap`, `Rotate`, `CenterX/Y` |
| Curve parallel | `FirstCurveParallel` → `RulerCurveParallel` | `Snap`, `CurveKind`, `PointData` (blob) |
| Multi curve | `FirstMultiCurve` → `RulerMultiCurve` | `Snap`, `CurveKind`, `OffsetAngle`, `CenterX/Y`, `PointData` |
| Radial line (emit) | `FirstEmit` → `RulerEmit` | `Snap`, `CenterX/Y` |
| Radial curve | `FirstCurveEmit` → `RulerCurveEmit` | `Snap`, `CurveKind`, `PointData` |
| Concentric circle | `FirstConcentricCircle` → `RulerConcentricCircle` | `Snap`, `RadiusX/Y`, `Rotate`, `CenterX/Y` |
| Guide | `FirstGuide` → `RulerGuide` | `Snap`, `IsHorz`, `CenterX/Y` |
| Perspective | `FirstPerspective` → `RulerPerspective` | `Flag`, `PerspectiveType`, `EyeLevelHandleX/Y`, `MoveHandleX/Y`, `GridOriginX/Y`, `GridFlag`, `GridSize`, `CameraNear`, `FirstVanishIndex` |
| Vanishing point | `RulerPerspective.FirstVanishIndex` → `RulerVanishPoint` | `Flag`, `VanishPointX/Y`, `ParallelAngle`, `GuideNumber`, `GuideDataSize`, `Guide` (blob) |
| Symmetry | `FirstSymmetry` → `RulerSymmetry` | `Snap`, `LineNumber`, `LineSymmetry`, `Rotate`, `CenterX/Y` |

`Layer` also carries `RulerRange`, `GuideMove` and `RulerVectorIndex`.

### The isometric ruler, decoded

```
RulerPerspective  Flag 0x70  PerspectiveType 3  FirstVanishIndex 1
RulerVanishPoint 1  Flag 0x11  VP (-8415698, -4980577)  ParallelAngle  30.616°
RulerVanishPoint 2  Flag 0x11  VP ( 3435817, -2031814)  ParallelAngle 149.396°
RulerVanishPoint 3  Flag 0x11  VP (     -223,     -268)  ParallelAngle 270.000°
```

This is a three-point perspective ruler whose three vanishing points are all made parallel, so
it is three families of parallel lines at about 30°, 150° and 90°: an isometric ruler.

### Findings and how sure we are

| Finding | Confidence | Evidence |
|---|---|---|
| A vanishing point with `Flag & 0x10` is parallel. `ParallelAngle` gives its direction, and `VanishPointX/Y` is only a handle position. | Strong inference | All 11 vanishing points in the 3 samples are `0x11`. In `9ba2b64bfb` the "VPs" sit inside the canvas at (452, 764) and (2159, 764), but the angles are 333.44° and 206.56° (2:1 isometric, atan ½ = 26.57°). Vanishing point 3 of the isometric ruler is at (−223, −268) with angle 270°. Which bit means "parallel" and which means "snap on" is **not separated yet**: every sample is `0x11`. |
| `ParallelAngle` is in degrees, counter-clockwise with y up, so the pixel-space direction is `(cos a, −sin a)`. Only the line matters (mod 180°). | Medium | In the isometric ruler, the `Guide` points of VP 1, (631, 566) → (1056.9, 314), lie at −30.6° in y-down pixel space, which is +30.6° with y up. VP 2 agrees. The guides in the other two samples do not lie on their angle (probably stale from before the points were made parallel). All three angle sets are symmetric, so this is the only sign evidence. |
| `Guide` is `GuideNumber` records of `GuideDataSize` = 16 bytes: two big-endian f64 values, an (x, y) handle point on a guide line. | Medium | The blob decodes to plausible canvas coordinates in all samples. |
| `PerspectiveType` 3 = three-point. A ruler may hold more vanishing points than its type number. | Observed | `469ab…` and `9ba2…` are type 3 with 4 points; the 4th is parallel at 180° (horizontal). |
| `Layer.LayerVisibility` bit 2 (value 4) = ruler shown. | Weak inference | Layers with rulers are 5; layers without are 1. |
| `Layer.RulerRange` = CSP's "Show in all layers / same folder / editing layer only". | Unknown mapping | All samples are 0. |
| `layerData.xml` `scaling` (6 for the ruler, 2 for the grid) | Unknown | It governs how PAINT places the material on a canvas of another size. |
| `LayerType` / `LayerFolder` | Partly known | `LayerFolder != 0` is a folder. Raster layers appear as `LayerType` 0 or 1. |
| Per-brush snapping is `Variant.BrushEnableSnap` in `EditImageTool.todb`. | Observed | In the local database: 207 rows 1, 46 rows 0, 183 NULL (non-brush rows). Prism does not read it yet. |

To settle the open items, the maintainer can make **one** extra material in PAINT: a
three-point perspective ruler with one normal vanishing point, one parallel point and one point
with snapping off, `RulerRange` set to "same folder", saved as a layer material. One sample like
that separates the flag bits, the range values and the angle sign.

### Behaviour to match (PAINT manual and observation)

- Special rulers snap **without a distance threshold**. While snapping is on and the ruler is
  shown for the layer being edited, every stroke of a snapping tool follows the ruler.
- For a perspective ruler, the stroke's start point and its first movement choose the line: the
  line through the start towards each snapping vanishing point (or along each parallel
  direction). The one whose direction is closest to the initial drag wins and is held for the
  whole stroke.
- A parallel line ruler works the same way with one direction. (Its perpendicular is not a
  candidate.)
- Snapping is on when View › Snap to Special Ruler is on **and** the sub-tool's "Enable
  snapping" (`BrushEnableSnap`) is on.
- Each vanishing point can be excluded from snapping individually.
- Show range: all layers, layers in the same folder, or only while that layer is being edited.

## 2. Design

### 2.1 Document model (`prism-doc`, L1): `crates/doc/src/special_ruler.rs`

```rust
/// CLIP STUDIO special rulers attached to one layer (PAINT stores them per layer).
pub struct SpecialRulers {
    pub rulers: Vec<SpecialRuler>,   // ≤ MAX_RULERS (64)
    pub shown: bool,                 // CSP LayerVisibility bit 2
    pub range: RulerRange,
}
pub enum RulerRange { AllLayers, SameFolder, EditingLayer }
pub struct SpecialRuler { pub snap: bool, pub kind: RulerKind }
pub enum RulerKind {
    Parallel { angle_deg: f64, center: [f64; 2] },
    Perspective(PerspectiveRuler),
    // Later phases: Radial, ConcentricCircle, Guide, CurveParallel, MultiCurve, RadialCurve.
}
pub struct PerspectiveRuler {
    pub points: Vec<VanishingPoint>, // ≤ MAX_VANISHING_POINTS (16)
    pub eye_level: Option<[f64; 2]>, // handle; drawn only when a point is not parallel
    pub grid: Option<PerspectiveGrid>, // phase 4
}
pub struct VanishingPoint {
    pub snap: bool,
    pub target: VanishTarget,
    pub handle: [f64; 2],            // where the point/handle is drawn
    pub guides: Vec<[f64; 2]>,       // ≤ 16 guide handle points
}
pub enum VanishTarget { Point([f64; 2]), Parallel { angle_deg: f64 } }
```

- `Layer` gets `pub special_rulers: Option<SpecialRulers>`, attached to the layer as in PAINT.
  This is what makes "drop the ruler material in as a layer" and "show range" work.
  `Layer::duplicate` copies it. Deleting the layer deletes it. Merge Down keeps the lower
  layer's rulers. Flatten drops them, as PAINT does.
- Coordinates are document pixels in `f64`. The constructor rejects anything non-finite. Stored
  values are capped at |v| ≤ 1e9, so the material's ±8.4e6 handle values survive, but snapping
  never uses parallel handle coordinates.
- Angles are normalised to [0, 360) when stored. Snapping works mod 180°.
- Implemented as designed, with these refinements: `SpecialRuler` flattens its kind in JSON
  (`{"snap":…,"kind":"parallel","angleDeg":…}`), all fields are camelCase, and loading a
  `.prism` whose rulers fail `SpecialRulers::validate` is a corrupt-file error. A layer whose
  set is empty, shown and range "all" drops the set (`None`).
- Use the name "special ruler" (CSP's term, 특수 자) everywhere. `rulers.rs` already means the
  View › Rulers bars, and `Ruler` means the Analysis measure line.

### 2.2 Native format and PSD

- `.prism`: add `special_rulers` to `manifest.rs` / `convert.rs` with `#[serde(default)]`.
  The compile-time completeness check in `prism-format` enforces this.
- PSD: Photoshop has nothing to map this to. Phase 1 drops the rulers on PSD export and reports
  one warning ("special rulers are kept only in .prism"). A private tagged layer block
  (round-tripped through `psd_blocks`) is a phase 4 option, only after checking that Photoshop
  and the psd-tools/ag-psd corpus readers skip it cleanly.

### 2.3 Snapping geometry (`prism-engine`, pure): `crates/engine/src/special_ruler_snap.rs`

This follows the pattern of `snap.rs`: geometry with no UI.

```rust
/// Candidate directions for a stroke starting at `start`, from every ruler that applies to
/// `layer` (show range + shown + ruler snap + vanishing-point snap).
pub fn candidates(doc: &Document, layer: LayerId, start: [f64; 2]) -> Vec<SnapLine>;
pub struct SnapLine { pub origin: [f64; 2], pub dir: [f64; 2], pub source: RulerRef }

/// Per-stroke state machine: collapse to `start` until the pointer has moved `lock_px`, then
/// lock to the candidate whose |sin(angle between dir and the drag)| is smallest, then project.
pub struct StrokeSnapper { … }
impl StrokeSnapper {
    pub fn new(cands: Vec<SnapLine>, start: [f64; 2], lock_px: f64) -> Self;
    pub fn push(&mut self, p: [f64; 2]) -> [f64; 2];
    pub fn locked(&self) -> Option<&SnapLine>;   // for the overlay
}
```

- A point vanishing point gives the line from `start` to the point, skipped when the two
  (almost) coincide. A parallel point gives `(cos a, −sin a)`. A parallel line ruler gives its
  angle.
- The lock distance reuses `stroke_constraint::LOCK_DISTANCE` (3 screen points / zoom). PAINT's
  exact threshold is unverified.
- Every input is checked with `is_finite`, and the number of candidates is bounded by
  64 rulers × 16 points.

### 2.4 Where the snap is applied: the UI drag pipeline, like ⇧

`canvas.rs` already constrains pointer positions before they enter `Drag::points`
(`stroke_constraint::constrain`, canvas.rs ≈3375 and ≈3666). Ruler snapping goes in the same
place:

1. On pointer down with a snapping tool, build `StrokeSnapper` from `candidates(doc,
   active_layer, start)` if View › Snap to Special Ruler and the brush's `snap_to_ruler` are on.
   Store it on `Drag` next to `constrain`.
2. On every move: ⇧ wins (the Photoshop 0/45/90° constraint). Otherwise the point goes through
   `snapper.push`.
3. `LiveStroke` and the committed `paint.stroke` / `paint.pencil` receive the **snapped** points.
   The journal, replays and the live preview therefore all match by construction, with no new
   stroke parameters.

Tools in phase 1: every `Tool::is_brushlike` tool (Brush, Pencil, Eraser, Mixer Brush and the
other brush-driven tools). Snapping needs View › Snap (the master switch) and View › Snap To ›
Special Ruler. A ⇧-click connection does not snap. The ruler overlay follows View › Extras and
uses a fixed violet canvas colour like the guide defaults (it is not a chrome token). Retouch tools,
shape tools and the Lasso come later.

For agents there is a query, `specialRuler.snapPoints` {points, layer?, zoom?}, which returns
{points, line}. It runs the same `StrokeSnapper`, so an agent snaps its points and then calls
`paint.stroke` with them.

### 2.5 Commands (`crates/engine/src/special_ruler_cmds.rs`, new module with `specs()`)

| id | journal | params |
|---|---|---|
| `layer.specialRuler.list` | no | `{layer?}` → the rulers on that layer, plus the rulers that apply to it through show range |
| `layer.specialRuler.addParallel` | yes | `{layer?, angle, center:[x,y], snap?}` |
| `layer.specialRuler.addPerspective` | yes | `{layer?, points:[{x,y}|{parallel:angle}], eyeLevel?:[x,y], snap?}` |
| `layer.specialRuler.edit` | yes | `{layer?, index, …same fields…, point?:i}`, with `coalesce` for handle drags |
| `layer.specialRuler.delete` | yes | `{layer?, index?}` (omit `index` to delete all) |
| `layer.specialRuler.setShow` | yes | `{layer?, shown?, range?:"all"\|"folder"\|"editing"}` |
| `specialRuler.snapPoints` | no | see §2.4 |
| `layer.cspMaterial.import` | yes | `{uuid}` or `{path}` → new layer(s) above the active one, one history step |

All of them get graceful-failure tests and `panic_hunt` coverage. View › Snap to Special Ruler
is a pure view toggle, so it is a shell `UI_COMMANDS` entry (`view.snapTo.specialRuler`) with a
new `SnapTo::special_ruler: bool` (default on, `view_cmds.rs`). It goes under View › Snap To below Document
Bounds and has no default shortcut: CSP's ⌘2 collides with Photoshop's.

### 2.6 CLIP STUDIO import (`prism-io`, L4)

As built (phase 2):

- `prism_io::sut::layer_material::read` decodes a `material_0.layer` (the whole file decodes with
  the C2F cipher) and walks `Canvas.CanvasRootFolder` → `LayerFirstChildIndex` /
  `LayerNextIndex` (folders flattened, ≤ 64 layers, depth ≤ 16, loops are errors). Pixels come
  from `ResizableOriginalMipmap` when set, else `LayerRenderMipmap`, through the SUT tile decoder
  (`material::mipmap_raster`); a layer whose pixels fail keeps no pixels and gets a warning.
- Rulers: perspective rulers and their vanishing points (`Flag & 0x10` → parallel at
  `ParallelAngle`, else a point at `VanishPointX/Y`; `snap` is always on, the bit is still
  unseparated). Guides are `GuideNumber` × 16-byte BE f64 pairs. A broken chain, a non-finite
  value or a bad guide blob skips that ruler with a warning; the other seven kinds are counted
  and reported as skipped (no `RulerParallel` sample exists, so its `Rotate` unit is unknown).
  `RulerRange` ≠ 0 is reported and treated as "all layers".
- `icedata/layerData.xml` gives `systemtag` and `isTiling`. A tiling image becomes a pattern fill
  layer (`Fill::Pattern`, not linked, phase at the placed tile origin). Its tile drops a wholly
  transparent last column and row: the pixel blocks cover one pixel less than the stored size,
  and the "Isometric grid" only joins up at 428 × 258 of its 429 × 259 canvas.
- `sut::library` lists `MaterialKind == "image"` materials in `Library::layers` (status `ready`
  when `data/material_0.layer` resolves, else `missing`); `import_layer(root, uuid)` and
  `read_layer_file(path)` read one. `brush.cspLibrary.list` returns them as `layers`.
- `layer.cspMaterial.import {uuid, root?} | {path}` adds the layers above the active one in one
  history step, the first layer of the chain on top, the material canvas centred on the
  document at its own pixel size (no resolution scaling: `scaling` is still unknown).
- UI: the CLIP STUDIO Materials panel lists layer materials in All and in a new Layers filter
  (`ui.set {cspLibraryFilter}`); Import adds the checked ones to the open document (it says so
  when no document is open). Since 2026-10-22 it works like PAINT's Material palette: the folder
  tree comes from the catalog's `FolderTag` / `TableFolderTagElementMap` (colon paths, Japanese
  system roots translated, a download pack's `-<content id>` hidden), favourites from
  `CatalogMetaData.Favorite`, the user's own materials (`Downloaded = 0`, not `.:Install:`) are
  listed, and a material dragged onto a canvas is used there (`layer.cspMaterial.import` takes
  `at`). Installed defaults keep only metadata and previews locally and are not listed.

The original design follows.

Design (`crates/io/src/csp_layer.rs`):

```rust
pub struct CspLayerMaterial { pub canvas: (u32, u32, f32 /*dpi*/), pub layers: Vec<CspLayer> }
pub struct CspLayer { pub name: String, pub folder: bool, pub raster: Option<MaterialRaster>,
                      pub rulers: Option<SpecialRulers>, pub offset: [i32; 2] }
pub fn read_layer_material(bytes: &[u8], ctl: &Interrupt) -> Result<CspLayerMaterial, String>;
```

- Decode with `csp_container::decode` and read the result with the existing bounded SQLite
  reader. Raster tiles reuse `sut::material`'s tile decoder (8-bit mono / gray / colour + alpha).
  Unsupported pixel kinds give a layer without pixels plus a warning. They never cause a failure.
- Each `NextIndex` chain is followed with a seen-set and a hard cap (64 rulers, 16 vanishing
  points, 16 guides). A cycle, a dangling index or a `Guide` blob that is not
  `GuideNumber × GuideDataSize` bytes is an `Err` for that ruler only. The rest of the material
  still imports.
- Ruler kinds the model does not hold yet are counted and reported ("2 concentric-circle rulers
  were skipped"), never guessed.
- Placement: the material canvas's centre maps to the document centre, with no scale. Parallel
  directions do not depend on placement. Point vanishing points and handles are translated.
  `scaling` stays unknown until the maintainer sample above exists.
- Library: `sut::library::list` keeps its brush output unchanged and adds a second list,
  `layers`, for `MaterialKind == "image"`. The `systemtag` from `icedata/layerData.xml`
  (`Ruler`, `PersRuler`, `Resizable`) lets the panel tell rulers from images. The same
  traversal and link rules apply.
- UI: the CLIP STUDIO Materials panel gets a "Layer materials" section with an Add to document
  action that runs `layer.cspMaterial.import`.

### 2.7 Overlay and layer row (`ui-egui`)

- `crates/ui-egui/src/special_ruler_ui.rs` draws every ruler that applies to the active layer,
  after the guides (canvas.rs ≈2629). It draws vanishing-point handles, the eye-level line
  (non-parallel only), guide lines through the guide handles and, for parallel points, a line at
  the angle through the handle. During a locked stroke it also draws the locked line.
- Colours come from new `theme::Tokens` entries (`special_ruler`, `special_ruler_inactive`), in
  CSP's violet family, never hard-coded.
- Layer rows show a ruler badge when `special_rulers` is set. Clicking the badge toggles `shown`
  through `layer.specialRuler.setShow`.
- Handle editing (dragging a vanishing point or guide) is phase 3, through `edit` with
  `coalesce`.

### 2.8 Brush option

`BrushSettings` gets `snap_to_ruler: bool` (default true). The SUT import maps
`BrushEnableSnap`: 1 → true, 0 → false, NULL → true. The CSP Tool Property shows it as "Enable
snapping" (스냅 가능; the Korean label must be checked against PAINT's own UI). It also gets a row
in `docs/csp-subtool-gaps.md`. Editing `ko.tsv` must keep its mixed line endings.

## 3. Phases

| Phase | Scope | Done when |
|---|---|---|
| 1. Model + snapping | §2.1–2.5 for parallel and perspective rulers, the View toggle, the overlay | A perspective ruler built by command snaps Brush, Pencil and Eraser strokes. Live and committed pixels are identical. `.prism` round-trips the ruler. |
| 2. CSP import (done) | §2.6 and the panel section | The local "Isometric ruler" material imports as a layer, and a 28° drag lands exactly on the 30.6° line. The "Isometric grid" image imports as pixels (as built: a seamless pattern fill). |
| 3. Editing + brush option | Handle dragging, the layer-row badge, `BrushEnableSnap` | Agent and UI handle edits are one undo step each. A SUT brush with snapping off ignores the ruler. |
| 4. More rulers | Radial, concentric circle, guide, curve rulers; perspective grid; shape and Lasso snapping; private PSD block | Each kind imported from a PAINT sample snaps like PAINT. |

## 4. Tests

- Geometry unit tests: each of the three isometric directions wins for drags within ±(90° − ε)
  of it. A point vanishing point converges. The stroke collapses to its start before the lock.
  NaN or coincident input passes through unchanged. ⇧ beats the ruler.
- Importer: a synthetic SQLite fixture wrapped with `csp_container::plain_fixture` covers each
  chain, cycle and bad blob. An opt-in local test (like `crates/engine/tests/csp_library_local.rs`)
  imports the three real materials and checks the decoded angles.
- `.prism` round trip of every `RulerKind`, and PSD export reporting its warning.
- UI harness: a drag on the iso ruler commits `paint.stroke` points that are all on one 30.6°
  line through the start (within 1e-9), and the live preview equals the commit.
- `panic_hunt` covers every new command. `cargo xtask layers`, the parity docs and the
  scorecard row are updated as usual.

## 5. Open questions for the maintainer

1. Can you supply the one PAINT sample from §1? It decides the flag bits, the `RulerRange`
   values and the angle sign.
2. Is the PSD private block (phase 4) wanted at all, or should rulers stay `.prism`-only?
3. Should rulers also snap Photoshop's own shape and Line tools, or only brush-like tools?
