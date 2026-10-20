# Clip Studio Paint brushes

Prism can import `.sut` brushes and downloaded `.tool` materials through File › Open, drag-and-drop, or the Brushes panel's
Import Brushes… button. The imported brush appears in a group named after its file and is
selected automatically. The brush library persists it across restarts.

Agents can use the same importer without a document:

```json
{"command":"brush.presets.importSut","params":{"path":"/path/to/brush.sut","select":true}}
```

The command also accepts base64 `data`, an optional `group`, and `replace` (default true).
Capability-restricted MCP/control callers must pass `data`; ambient filesystem `path` reads
are refused by the same policy as ABR imports.
It returns `format`, `group`, `count`, `imported` names, `warnings` (what was lost or
approximated and is worth acting on; the app lists them in a notice) and `notes` (facts that need
no action: the general CSP approximation, where the settings were read from, kept but
unrecognized display ids, a runnable non-brush sub-tool's destination). Re-import replaces the
same user group; built-in brushes are preserved. Invalid files leave the library unchanged.

## Import from CLIP STUDIO (downloaded brushes)

Brushes downloaded into an installed CLIP STUDIO PAINT can be imported without exporting a
`.sut` first. Window › Extensions › CLIP STUDIO Materials opens a dockable extension panel (it
floats at first; drag its tab into any panel group or column like other panels). It lists every
downloaded sub-tool by collection, with CLIP STUDIO's own preview images (`thumbnail/thumbnail.png`
per material, `thumbnail/catalogThumb.png` per collection), and imports the checked ones:

- **Search and filter**: the search field matches material and collection names and user tags;
  All / Brushes / Sub tools filter by kind and show counts. Collections fold; Select all checks a
  collection's ready materials.
- **Tags**: CLIP STUDIO's user tags (`Tag` and `TableTagElementMap` in the material catalog, keyed
  by the material's `MaterialModifier` row). A material counts its own tags and its collection's,
  since materials inside a collection are usually untagged. The Tags toggle opens every tag of the
  materials still shown, most used first; chosen tags combine (all must match) and stay beside the
  toggle with a remove mark and Clear tags.
- **Views**: one column of rows (the default) or two columns of cards with larger previews,
  switched by the list and grid buttons beside the kind filter.
- **Rows**: a click checks a ready row; the second line gives readiness, kind and the material's
  own tags; In library marks a material already imported into its collection's brush group
  (importing again replaces it). The tooltip lists every tag that applies.
- **Footer**: Import n (only once something is checked) and Open Files… for exported `.sut`,
  `.tool` and `.laf` files. The folder button picks another CLIP STUDIO user data folder.

Downloaded non-brush sub-tools (the catalog's `other_tool`, such as fill tools) import for the
Clip Studio workspace like non-brush `.sut` files. Auto actions, images and 3D materials are not
listed. Each row says whether it is ready:

- **Ready**: a downloaded `.tool` file is present, or PAINT keeps sub-tool settings for it (its download cache, or its current sub-tool
  library, which includes any edits made there; the row's tooltip and the import warnings say
  which).
- **Downloading**: the catalog still marks the download as pending.
- **Not prepared**: neither a local `.tool` file nor a sub-tool cache was found. Finish the
  download in CLIP STUDIO, then Refresh.
- **Unavailable**: the material is not in the local material folder.

Everything is read from the local CLIP STUDIO user data folder (`%APPDATA%\CELSYSUserData\CELSYS`
on Windows; other platforms need `root`): the material catalog, the two sub-tool databases and
the tip material files they reference. Nothing is written there, no account data or server is
used, and a pending SQLite write-ahead log is reported (close CLIP STUDIO to see the newest
downloads). Tips whose original raster layout is unsupported use the material thumbnail with a
warning, as for `.sut` files. The panel always shows the license caveat (folded to one line; click it for the full text): ASSETS materials can
be licensed for CLIP STUDIO software only, so check each material's terms and never
redistribute them.

Agents use the same commands:

```json
{"command":"brush.cspLibrary.list","params":{}}
{"command":"brush.cspLibrary.import","params":{"uuids":["<uuid from the list>"],"select":true}}
```

`list` returns `root`, `brushes` (`uuid`, `name`, `kind`, `catalog`, `status`, `source`, `guidance`,
`thumbnail`, `tags`), `catalogs` (`name`, `thumbnail`, `tags`), `ready` and `notices`. `import` takes up to 64 `uuids`, optional `root`, `group` (default: the
collection name), `replace` (default true: a re-import replaces the same brush in its group, not
the rest of the collection) and `select`; it returns `imported`, `count`, `brushes`, `groups`,
`warnings`, `notes` and per-brush `failed` reasons, and fails only when nothing was imported. Both read
the local filesystem, so capability-restricted MCP/control callers are refused.

The direct container reader validates C2F checksums, decodes its payload, and reads the
`MaterialToolVariantManager` database. It does not search arbitrary embedded bytes for a
SQLite header. Direct imports currently require the sub-tool to contain its dependencies;
tools with external tip/texture references may still need PAINT's cache and local material
resolver. A Ready row means a source is present; malformed or unsupported contents still
produce an import error.

## Downloaded auto actions (`.laf`)

Open or drop a `.laf` file to import its action set and show the Actions panel. Importing
does not execute the actions. Agents use `actions.importLaf` with `path` or base64 `data`;
capability-restricted callers must supply `data`. Existing names are rejected unless
`replace:true` is supplied. Actions persist with the preset library.

The initial reader supports loading active-layer transparency, clearing outside the current
selection, and deselecting, including the tested boundary-cleanup action. Unknown commands
or parameters reject the whole import; no partial action is silently installed. Set names,
disabled steps, dialog flags and source command identities are retained in Prism's common
Actions model. Each supported CSP source step becomes one independently editable Prism
step. Clear Outside Selection preserves the selection and creates one undo entry (currently
limited to 64 million canvas pixels). Mapped operations use Prism's selection and pixel
semantics, so identical CSP rendering is not claimed. See [Actions](actions.md).

The tested enclose-and-fill `.tool` (output operation 51) imports its settings, but remains
disabled in the Sub Tool panel because Prism does not implement that fill operation yet.

## Non-brush sub-tools

A `.sut` can also hold a CSP fill, selection, gradient, text, figure or other function
sub-tool. Prism reads its operation IDs (`docs/sut-analysis.md` › Sub-tool operation IDs):
brush-family sub-tools (brush, pen, pencil, eraser, copy stamp, dot pen) import as brushes as
before; every other kind is kept in the library with a warning but never appears in the
Brushes panel or becomes the current brush, so the Photoshop workspace looks unchanged.
`brush.presets.importSut` returns `brushes`, the number of imported brush presets.

Window › Workspace › Clip Studio switches the shell to CSP's tools (Pen, Pencil, Brush,
Airbrush, Decoration, Eraser, Blend, Fill, Gradient, Selection, Auto Select, Text, Figure, Move
Layer, Hand, Zoom, Eyedropper) with a Sub Tool and a Tool Property panel. Imported sub-tools
are listed under their tool by icon, then by operation. Fill (bucket), gradient and text
sub-tools run on Prism's Paint Bucket, Gradient and Type tools with their mapped options;
kinds without a Prism counterpart (enclose-and-fill, comic frames, rulers, …) are listed
greyed. Any Photoshop workspace switches back.

## Supported subset

The reader loads the current named sub-tool's settings from a SQLite rowid database (UTF-8)
and images from its referenced primary materials. Supported ustar/C2F black/alpha (8- or 1-bit) and gray materials
use the linked original raster rather than the material thumbnail. The encoded C2F prefix is
not decoded: complete schema records recovered from bounded plaintext pages must validate
the Layer → Mipmap → MipmapInfo → Offscreen links. Unsupported original material layouts
fall back to their PNG thumbnail with an explicit grain-fidelity warning. Raw PNG materials
remain supported. Node/variant identifiers retain
SQLite's signed 64-bit precision. Material arrays use bounded length-prefixed records
and exact paths; repeated tip references retain their positions. It imports size, opacity, flow,
hardness, spacing, thickness, rotation, flips, disabled anti-aliasing and independent pen-pressure responses. Multiple tips
follow CSP's tip order (`BrushPatternOrderType2`, else `BrushPatternOrderType`): repeat, reverse
(round trip), do not repeat (the last tip stays) or random per dab. The tip angle can follow the
pen's direction (where it leans), its axis rotation or the line direction, plus a random turn.
Colour tips keep their own colours (8-bit RGB beside the alpha shape, stored as tip files in the
preset store) unless *Change brush tip colour* is on, also in ribbons; with colour mixing a tip
mixes its mean colour (a warning when it has several colours). A brush that enables a tip image without referencing
one (seen in shipped brushes) imports with a round tip and a warning. Sampled tips retain black coverage and transparency
in 16-bit masks, and can paint 8/16/32-bit documents.
Pen pressure requires both the effector's supported-control bit and its active-control bit;
an available but disabled pressure control remains disabled after import.
Size, opacity, flow and per-tip texture density preserve their individual minimums and
2–16-point custom pressure curves. Two points interpolate linearly; additional points
are quadratic Bezier controls with midpoint joins. Invalid curves warn and disable only
the affected response. An enabled tilt control imports as `cspTilt` on the same
dynamic: its two stored values and curve scale the parameter by pen altitude, and a
mouse counts as a vertical pen (Grainy Pen paints its spray area at 50% with a mouse,
as in CSP). Speed and random controls still warn.

The Texture category maps onto Prism's brush texture: the referenced texture material
(luminance; transparency reads as white), texture density, invert, scale, brightness, contrast
and Apply by each plot, with pen-pressure density when applied per plot. Emphasize density is
approximated by extra contrast. Texture rotation is ignored. Codes 0–9 now use distinct CSP
texture equations: Normal, Multiply, Subtract, Compare, Outline, Overlay, Color dodge,
Color burn, Hard mix and Height. Other codes use Multiply with a warning. Brightness/contrast
and density follow the recovered calculation order; this float implementation does not reproduce
CSP's fixed-point rounding or guarantee identical stroke accumulation. A texture that can't
be read drops the texture, not the brush.

CSP spray imports independent particle size, count, distribution bias and base particle angle,
including separate particle-size/count pressure curves. A negative maximum bias places particles
on the brush area's circumference. The engine uses deterministic Prism randomness, with a
256-particle limit per step. Fixed-point placement, particle-size synchronization/physical units
and particle-angle input controls still warn. The stored random angle amount has no effect when
its controlling flag is off. Texture minification uses at most two 2×2 mip levels, dropping odd
trailing edges, with integer-coordinate nearest sampling up to 100% and bilinear above 100%.

CSP automatic spacing types 1–3 (scaled by hardness and thickness) follow the native
formula; types 4 and 5 are approximated. Anti-aliasing levels none/weak/medium/strong
sample the tip like CSP (one nearest sample, or 2×2 nearest samples of the original,
display-size or half-size tip level); a computed round tip's reduced levels are
approximated. Perceptual colour mixing averages in CSP's PQ cone space. Dual brushes, ribbons, start/end fade and unmapped targets, and colour
mixing types other than blending are not reproduced faithfully. Color mixing picks up
the layer colour under the brush (amount/density of paint, color stretch). Start and end by length or
percentage tapers size, flow, texture density and spray size/density with CSP's
quarter-sine ramp; the end appears when the stroke is committed. Import warnings explain these limits. This is an
initial compatibility path, not a claim of identical CSP rendering. Only the first named
sub-tool is imported; collections and other material layouts are not supported.

Limits: 64 MiB per SUT/database, 4096 rows per table, 32 b-tree levels, 64 tips,
2500 px per tip edge and 16 million total tip pixels. The brush preset store additionally
limits each persisted group to 16 MiB. Corrupt page links, overflow cycles, missing materials,
unsupported encodings and oversized PNGs produce errors.

## Implementation and validation

The implementation is native Rust and uses the existing Prism PNG codec and bounded zlib decoding. It does not execute
SQL or run Python, SQLite native libraries, or CSP2PC at runtime. Format research used
[CSP2PC](https://github.com/Leon-Schoenbrunn/CSP2PC) and the
[public SQLite file-format specification](https://www.sqlite.org/fileformat.html); no converter
code, template brushes or user-owned SUT files are included in the source tree.

Synthetic tests exercise masks, tip order, SQLite overflow pages, corrupt files and cancellation.
A follow-up [REA/Ghidra investigation](sut-analysis.md) records the installed Clip Studio
binary identity, observed reference framing, importer fixes and unresolved rendering questions.
Raw analysis and local brush files remain outside source control.

Curves and source-specific texture modes are also editable through `tools.setBrush`:

```json
{"command":"tools.setBrush","params":{"texture":{"cspMode":"outline"},"shapeDynamics":{"enabled":true,"size":{"control":"penPressure","minimum":0.1,"pressureCurve":[[0,0],[0.5,0.8],[1,1]]}}}}
```

Use `pressureCurve: null` to restore a parameter's linear pressure response. Curves belong
to each dynamic parameter; they do not rewrite the shared pen input. `texture.cspMode: null`
restores the existing Photoshop-style `texture.mode` behavior. Both fields survive preset
save/load. There is no dedicated visual curve editor yet.

Imported CSP spray brushes preserve particle diameter, count, distribution and
their independent pressure controls. Disabled CSP anti-aliasing uses nearest
sampling of the original tip while retaining fractional alpha; it is distinct
from Prism's Pencil threshold. Existing imported presets must be reimported to
pick up original material rasters and these source-specific settings. Grainy
Pen now retains its small particles, but its stroke density still differs from
the supplied CSP reference; see the comparison in the analysis report.

SUT tip thickness above 100% is preserved independently of roundness, including
Grainy Pen's vertical 240% setting (`cspTipScale: [1, 2.4]`). The scale applies
before tip rotation and does not enlarge the spray area. Reimport the SUT to
update previously saved presets. A live CSP comparison at 15px and 90% confirms
closer coverage, but faint edges, peak accumulation and grain distribution still
differ. Thickness dynamics remain unsupported.

### Watercolor edge

`BrushUseWaterEdge` now imports as `cspWaterEdge` (`radius` px, `opacity` and
`darkness` influences 0..1, `blur` px, `afterDrag`). When the stroke buffer is
composited, covered pixels within the radius of an empty buffer pixel gain extra
alpha in proportion to the strongest coverage nearby, with CSP's ordered-dither
grain, so the rim can exceed the stroke opacity (Grainy Pen reaches 100% at 90%
opacity). Darkness lowers the rim colour's value. With **Process after drag** the
edge appears when the stroke is committed, and a nonzero blur spreads it with an
integer Gaussian, including a faint halo just outside the stroke. Edge style,
strength and vividness, non-pixel units and the per-pixel colour variant warn or
stay unmapped. Prism treats pixels outside the stroke buffer as empty everywhere,
so a stroke touching the canvas border also gets a rim there. Reimport the SUT.

A local-only engine test can exercise the initial user-provided oracle without committing it:

```sh
PRISM_TEST_SUT=/path/to/세모네모펜.sut cargo test -p prism-engine local_sut_import -- --ignored
```

To check a whole local folder of brushes (searched recursively), run the folder report. It
imports each `.sut` into a fresh session, paints a test stroke, and prints a Markdown table
(status, tips, size, painted pixels, warnings, time) plus warnings ranked by frequency. Import
errors are listed; only a panic fails the test.

```sh
PRISM_TEST_SUT_DIR=/path/to/sut-folder PRISM_TEST_SUT_REPORT=/tmp/sut-report.md \
  cargo test -p prism-engine --lib local_sut_folder_report -- --ignored --nocapture
```

## Material-specific Tool Settings

New SUT imports retain explicit source and presentation metadata separately from
the rendering brush. Select the imported preset and open Brush Settings (F5) to
use Tool Settings / Advanced Settings. Ordinary presets keep the existing panel.
Old imported presets without this metadata must be imported again; their names or
textures are never used to guess their origin.

Tool Settings follows the imported visible property membership. Advanced Settings
provides eye buttons, category headings and **Restore imported display layout**.
Overrides are stored per preset, survive the preset store round trip and follow
save-as/rename operations. Hiding a control does not disable its brush effect.
Texture scale can expand beneath texture density. This expansion is view state.

The verified brush operator family supports editing size (px), opacity (%),
Prism blend modes from imported Normal, anti-aliasing on/off, local tip thickness
(%), texture density (%), texture scale (%) and the watercolor edge (on/off and
range; Advanced Settings adds opacity/darkness influence, process after drag and
blur). Unsupported CSP effects retain their imported values but are disabled with
explanations. Inactive color mixing is distinguished from unimplemented
stabilization and vector magnet effects.

The original material title is not yet decoded, so its label is “Imported
texture”. CSP's slider/indicator preference is shared UI state and was absent from
the differential exports; it is not synthesized as per-preset SUT metadata. This
initial panel uses sliders and does not claim to reproduce CSP's indicator steps.
Other unverified operator families still edit verified visible brush properties
using their actual runtime values. Additional hidden controls are not inferred
for these families; a notice explains that their extra settings are not mapped.

Automation uses `brush.sutDisplay.get`, `brush.sutDisplay.set` and
`brush.sutDisplay.reset`, with optional `name` (defaults to selected preset).
Set accepts `parameter` plus `visible`, and/or `showCategories`. Values continue
through `tools.setBrush`. For example, `{"parameter":1000,"visible":false}`
hides size while preserving its value; `{"showCategories":true}` enables headings.
`ui.set` accepts `sutDetails` and
`sutTextureScaleExpanded`; `ui.inspect` includes persisted `sut_panel` view state.
Native format evidence and remaining uncertainties are in `sut-analysis.md`.
