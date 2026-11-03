# SUT compatibility audit — 2026-11-03

Implementation follow-up: [Spec.1.2.2](../doc/Spec.1.2.2.md) records correction-policy
separation, legacy SUT store migration and fixed sub-color blending. The measurements
below are the pre-fix baseline; original-application parity remains unverified.

Goal: preserve Photoshop behavior for ABR presets and CLIP STUDIO behavior and
editable options for SUT presets. This is an investigation, not a rendering fix.
Import success, warning-free import and matching strokes are separate milestones.

## The 444 materials

The current local library contains **444 materials: 154 brushes, 2 other sub-tools
and 288 image/layer materials**. `csp_library_ui::materials` chains the sub-tools
and layers; its “All materials” count is not a SUT brush count. The read-only
`prism_io::sut::library::list` inventory reproduced these counts. No alternate
library is needed to explain the screenshot.

Rebuilt and ran `cargo test -p prism-engine --test csp_library_local
every_local_csp_material_imports_and_paints -- --ignored --nocapture`:

- 156/156 sub-tool materials ready and imported; no reported failed imports.
- 154 brush presets produced nonempty strokes; no reported stroke errors.
- 47 materials emitted warnings (60 warning occurrences, categories below overlap).
- 132 materials reported unrecognized display parameter IDs in import notes.
  This measures missing editable controls, not 132 failed rendering implementations.
- Four contact sheets and the per-material UUID/name/warning report are in ignored
  `target/csp-check/`. The first sheet was visually inspected.

Limits: the existing test draws one seeded pressure path, caps size at 40 px and
uses 8-bit RGB with a white background. It does not compare CSP output, test all
options, replay real tablet timing, test all depths or exercise the 288 layer
materials. Its test success alone does not assert absence of warnings or failed
rows; those report fields were checked separately. The session's default 10%
Photoshop-style smoothing also affects these strokes (see below).

## Evidence and provenance

Installed CLIP STUDIO PAINT: version 5.0.4.0, 92,879,872 bytes, SHA-256
`2e08e084c7f2256d4bba1d6a5bfcdd3498b8cd026667e1952e116c56c9ac9a2a`,
matching the executable in [the earlier investigation](sut-analysis.md).

REA 6.4.0 `inspect-pe-resources` succeeded on the original executable:
`ev_4e7f6c4657e76d656bb57d7873367abd6c5e9c02c37b336f083f660cdc4580c3`.
This establishes resource inventory and identity, not brush behavior. The earlier
REA native-open overlapping-header limitation was not retested or claimed fixed.

Direct Ghidra 12.1.4 re-examined three functions in the existing `Persist` project
with `-readOnly -noanalysis`; all three decompilations completed, and the project
changes were discarded. This is **not REA Evidence**. The initial launcher failed
because it attempted to save Java preferences outside the workspace; rerunning
with a workspace-local home/AppData succeeded without changing the installed app.

- `0x142b5b1b0`: reads stabilization properties 0x28, 0x29, 0x2a and stickiness 0x32.
- `0x141afbd40`: sets the stabilization/start window and speed-dependent parameters.
  The initial window uses the level at levels <=2, otherwise half the smaller of
  level/stickiness, clamped to 2..15; the speed scale is 2*level+100.
- `0x1422e09a0`: enabled color amounts can pass through as constants when their
  effector flags equal the enabled-only state; an input/random controller is not
  required for a nonzero effect. Field-to-property mapping comes from the earlier
  settings-builder trace, not from this function alone.

Private outputs, seeds, Rust probes and hashes are retained in ignored
`target/rea-tools/csp/audit-2026-11-03/`. No proprietary code or assets are added
to source. No new matched CSP drawing capture was made.

## Highest-priority behavioral gap: two correction systems run in series

Prism has a CSP-specific `CspStabilizer`; SUT stabilization is not wholly missing.
However:

1. `crates/engine/src/lib.rs` initializes tool smoothing to 0.1.
2. `BrushSettings::picked_over` in `crates/paint/src/brush.rs` unconditionally
   preserves that tool smoothing when choosing any preset, including SUT.
3. `crates/paint/src/dynamics.rs` feeds CSP stabilization into `Smoother` for live
   input, end-of-stroke processing and the full-path start/end calculation.

Thus SUT output depends on a Photoshop-style tool option as well as its own
stabilization. Even a SUT with stabilization off can inherit extra smoothing.
This is a source-confirmed coupling; its user-perceived magnitude varies by input.

A standalone Rust probe using the freshly built paint library used 60 samples,
8 ms apart, size 6, CSP stabilization 12 with speed adjustment and a sinusoidal
path. CSP stabilization alone produced 129 dabs, ending at
(127.857559, 17.631408); picking it over a tool with smoothing 0.1 produced 128,
ending at (127.569980, 18.155023). This proves the coupling changes output, not
which result matches CSP pixels.

The CSP stabilizer also still lacks confirmed device-type scaling, release-sample
pressure handling and the early catch-up termination described in
[Stabilization and Adjust by speed](sut-analysis.md#stabilization-and-adjust-by-speed-2026-10-16).
Post correction is a separate missing feature; it must not be replaced by simply
raising Photoshop smoothing. Conversely, the current exponential/pulled-string
implementation does not establish exact Photoshop fidelity for ABR brushes.

## Measured active gaps in this library

| Gap | Materials with warning | Current effect / source |
|---|---:|---|
| Post correction | 22 | `BrushUseRevision`/`BrushRevision` ignored; `io/src/sut/blend.rs` |
| Constant sub-color blend | 11 | No random input -> amount becomes zero; `blend::jitter` |
| Particle rotation input | 9 | 2026-11-04: inputs 0x10/0x20/0x40/0x100/0x200 and random 0x80 applied (`CspSpray.angleInput`); several inputs at once use the first |
| Spray particle size units | 6 | Treats values as pixels; `mod.rs` |
| Color mixing type 2 | 5 | Approximated with blending; `mod.rs::color_mixing` |
| Missing dual-tip assets | 2 | 2026-11-04: dual brush kept with the present tips (round tip when none), with a warning |
| Dual input/random dynamics and rotation | 1 | Ignored; `io/src/sut/dual.rs` |
| Size effector control 0x40 | 1 | Additional control unsupported; `io/src/sut/effector.rs` |
| Watercolor-edge units | 1 | Value used in pixels; `mod.rs` |
| Emphasize texture density | 1 | Approximated by contrast; `mod.rs` |
| Start/end fade type 2 | 1 | Unsupported; `mod.rs` |

The screenshot's **葉っぱブラシ3** was reproduced in this run: imported, produced a
stroke and emitted the same constant sub-color warning. It also has eight
unrecognized display parameter IDs. The warning describes a real omitted color
effect, not a corrupt brush or a smoothing problem. 葉っぱブラシ and 葉っぱブラシ2
also hit this color gap.

Warning counts understate total incompatibility. The importer also approximates
asymmetric random color ranges, cannot combine per-tip and per-stroke color
changes, and cannot faithfully target only the sub color. Dual texture, ribbon,
flow, tip order and other options are unsupported even when absent from this
particular library's active warning set. Several stroke blend formulas and ribbon
details remain inferred rather than validated against CSP. See the individual
evidence/unknown sections in `sut-analysis.md`.

## Editable options are a separate requirement

The October 24 changes already added generic catalog controls to the SUT panel;
the October 21 totals in `csp-subtool-gaps.md` are historical, not a fresh score.
Current `sut.rs` retains allowlisted numeric settings; unknown display IDs can
survive without an editable control. A shown or saved value is not necessarily
used by the renderer. In particular, the October 24 dev log records direction
1060 and spray enable 1130 as kept-only.

For each option, track independently: decoded, retained, editable, applied to
live/committed strokes, saved/restored, compared with CSP. A complete Sub Tool
Detail equivalent needs the full option inventory, not only the rows a preset
chooses to expose in Tool Property.

## Implementation order and acceptance criteria

1. **Separate brush behavior by preset origin.** Explicitly preserve ABR/Photoshop
   tool smoothing while SUT uses CSP correction settings. Do not infer origin
   merely from `csp_stabilizer.is_some()` because level-zero SUTs have none.
   Cover ABR -> SUT -> ABR switching, live vs committed input and save/reload.
2. **Implement CSP color evaluation independently.** Constant offsets/sub-color
   mixing, signed/random ranges, input curves, color targets and combined
   per-stroke/per-tip evaluation need their own semantics. First regression:
   the leaf brushes with contrasting foreground/background colors.
3. **Finish correction behavior.** Trace post correction and its interaction with
   pen-up, pressure and start/end taper; capture slow/fast strokes and sharp
   corners. Validate the remaining stabilization release/device differences.
4. **Close the measured rendering gaps.** Prioritize particle rotation/units,
   type-2 mixing and dual dynamics. Diagnose the two missing dual-tip assets
   separately from unsupported rendering. Add DPI-aware units and remaining
   texture/edge/start/end behavior with bounded input handling.
5. **Complete the option surface and matched comparison suite.** Inventory every
   active local option and unknown ID; ensure editing changes the stroke and
   survives save/reload. Preserve per-material UUID/name diagnostics.

For the 154 brushes, collect matched CSP and Prism results with the same brush,
canvas DPI, zoom, colors, pressure/tilt/rotation/time input and stroke geometry.
Include slow/fast curves, corners, dots, pen-up tails, repeated passes, colored
backgrounds and 8/16/32-bit Prism regression cases. For random brushes compare
distribution/coverage as well as images; equal seeds do not imply equal RNGs.
Agree tolerances for trajectory, width, alpha/color, texture, latency and release
tail before calling a brush compatible. Keep ABR behavior regressions separate.

The two non-brush sub-tools and 288 layer materials need their own import/editing
checks to cover the user's entire 444-material library. This audit does not claim
they passed brush rendering tests or that any brush has achieved pixel parity.
