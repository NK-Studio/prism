# Windows pen input validation — 2026-10-14

The input changes preserve paired position, pressure, pose and time before winit
turns pen input into touch/mouse events. They do not implement a recovered
Photoshop pressure curve or add an artificial tapered tail.

## Observed comparison settings

A read-only Photoshop Action Manager query on this machine returned the following
current paintbrush settings. The local descriptor is saved in
`target/photoshop-current-brush.json`; no document or brush options were changed.

| Setting | Photoshop | Equivalent Prism comparison setting |
| --- | --- | --- |
| Tip | Round, angle 0°, roundness 100% | Computed round tip, angle 0°, roundness 1 |
| Diameter | 25 px | size 25 |
| Hardness | 100% | hardness 1 |
| Spacing | 5% | spacing 0.05 |
| Minimum diameter | 0% | minimum size 0 |
| Size control | Pen pressure, size override enabled | pressureSize true |
| Opacity / flow | 100% / 100% | opacity 1 / flow 1 |
| Opacity pressure | Disabled | pressureOpacity false |
| Smoothing | Enabled with value 0% | smoothing amount 0 |
| Airbrush / texture / scatter | Disabled | build_up false; no texture or scatter |

These are current settings, not a reconstruction of either attached screenshot.
Use a fresh RGB document with the same zoom and depth in both applications for
physical comparisons. Record settings with the input trace. A scripted path in
Photoshop does not reproduce a physical pressure stream.

## Automated coverage

- Safe native mapping tests cover first missing pressure, previous-contact
  isolation, bounded queues, stale history, timestamp wrap, cancellation, device
  changes, pressure capability and window routing. Native window tests exercise
  the gesture-status hook and subclass cleanup without a tablet.
- Canvas integration tests send native packets alongside deliberately incorrect
  full-pressure emulated mouse/touch events. They cover weak same-frame taps,
  stationary pressure, per-sample time, delayed releases after cancellation,
  tiled documents and widgets covering the canvas.
- Control tests cover optional timestamps, legacy point format, cancellation,
  atomic validation, trace limits and trace replay.
- Renderer and engine tests check stationary growth, pressure decrease without
  erasure, unchanged stationary input, pressure/position smoothing and airbrush
  timing. RGB 8/16/32f and mask tests compare live preview, final pixels, frame
  batching and one-step Undo/Redo.

The synthetic engine preview is `target/pen-pressure-preview.png`. It was visually
inspected: stationary pressure produces progressively larger round marks and
the supplied decreasing-pressure path narrows continuously. This is an engine
regression illustration, not a Photoshop pixel-match claim.

The final engine unit suite passes 987 tests (11 ignored), including the existing
zoom/pulled-string live-versus-commit regression. A bug found during that run was
fixed: moving a pressured pointer inside a frozen pulled-string radius must not
repeatedly ink its anchor. The desktop application unit suite passes 76 tests;
the GPU/CPU canvas integration suites pass all 11 tests (`canvas_16f`,
`drag_preview_canvas`, `live_stroke_canvas`). `panic_hunt` passes and the layering
check reports 29 crates without violations.

Final focused runs pass all 22 pen UI/control/window-lifecycle tests, 84 paint
tests, 21 tablet tests and both pressure integration tests across RGB 8/16/32f
and masks. The modified tablet and paint crates pass strict all-target Clippy.
The app/UI all-target Clippy run passes with only the two baseline lint categories
below allowed (`--no-deps -- -D warnings -A clippy::collapsible_match
-A clippy::nonminimal_bool`). The desktop executable builds at
`target/debug/prism.exe`; an already-running `target/fast/prism.exe` is unchanged.

The full UI unit run passes 1360 tests with 13 ignored, but three Windows
save-dialog/path tests remain failing (`a_full_path_in_the_name_is_taken_as_the_location`,
`a_picked_file_sets_the_folder_name_and_format`, and
`saving_over_a_file_open_in_another_tab_warns`). The first two compare a normalized
folder against a temporary-directory string retaining its trailing separator.
Those save paths are outside this change. Sandbox-only export failures disappear
when the tests run with ordinary Windows temporary-directory access.

Unqualified workspace-dependent Clippy is blocked by existing `collapsible_match`
and `nonminimal_bool` findings in unrelated preview/dialog/transform code. Keep
these baseline lint findings distinct from validation of the changed pen code.

## Stationary long-press cursor regression (2026-10-14)

The user clarified that only the brush cursor disappears during a stationary
pen hold. The actual canvas harness reproduced loss of `cursor_image` at frame
48 (about 0.8 seconds at 60 Hz), with native pen packets plus winit's emulated
touch and pointer events. This is egui's independent long-touch recognition,
which clears the canvas interaction even after the Windows hold gesture has
been disabled.

The native input plugin now removes only Windows Ink packets' matching emulated
`Touch` events before egui processes them. It reads the backend first so viewport
binding still sees the original touch IDs. Pointer motion, primary/secondary
buttons, unrelated finger touches and the non-native touch fallback remain.
This does not change stroke cancellation or add repeated stationary dabs.

The regression now preserves the brush cursor for every frame of a three-second
hold, opens no brush picker, and commits just one point as one stroke on lift.
The focused pen/control/window/cursor suite passes 37 tests (one optional visual
test ignored); the cursor PNG generation test also passes separately, and
`target/tool-cursor-preview.png` was inspected on black, gray and white.
The 29-crate layer check passes. Physical pen/driver cursor behavior remains
part of the real-device acceptance below.

## Smoothing 100 endpoint comparison (2026-10-14)

A subsequent read-only query during the user's smoothing-100 comparison returned
`smooth:100`, `smoothingValue:255`, `smoothingCatchup:true`,
`smoothingCatchupAtEnd:false`, `smoothingRadiusMode:false`, and
`smoothingZoomCompensation:true`. Size pressure override was enabled and opacity
pressure override disabled. The observation is saved in
`target/photoshop-smoothing-100.json`. These current options differ from the
earlier 0% observation above; Photoshop was not modified by the query.

Prism previously defaulted `catchUpOnEnd` to true. On release, its smoother
therefore appended the last raw point after the filtered endpoint; declining
pressure along this extra segment produced a pointed extension. A synthetic
80-sample regression reproduced that extension before the change. The default
is now false, matching the observed Photoshop comparison setup. Explicit true
values in saved/imported brushes and command parameters still extend the stroke.
No minimum pressure, rounded cap overlay or artificial taper was added. Turning
smoothing off still passes through the original pressure samples.

`target/smoothing-end-comparison.png` shows the same synthetic pressure/position/
time stream: previous default on the left, new default on the right. It was
inspected directly. This is a Prism option comparison, not a pixel-match claim
against physical Photoshop strokes. The size of Photoshop's filtering lag and
its internal smoothing algorithm remain unverified.

For existing presets with the option explicitly enabled, disable **Catch-up on
Stroke End** in the smoothing gear menu to reproduce this comparison. The option
is independent of **Stroke Catch-up**, which remains enabled. Adobe documents
these separately in its [stroke smoothing guide](https://helpx.adobe.com/photoshop/desktop/repair-retouch/clean-restore-images/create-smoother-more-polished-brush-strokes-with-stroke-smoothing.html).

Regression coverage includes smoothing 0/50/100 and both endpoint options,
RGB 8/16/32f and masks, per-point and batched delivery, live/commit pixel equality,
and one-step Undo/Redo. Shift-click connections explicitly complete their clicked
endpoint in both preview and commit without changing the session's freehand
smoothing option. The full UI regression exposed this dependency on the old
default; the existing endpoint assertion was preserved and preview equality added.

The brush suite passes 85 tests and the engine suite 987 (11 ignored). Strict
paint Clippy passes. The UI suite passes 1371 tests (13 ignored), excluding only
the same three pre-existing Windows save-path failures listed above.
Unqualified engine Clippy still hits existing
`nonminimal_bool` in `color_cmds.rs`/`fill_cmds.rs`, `manual_range_contains` in
`fill_cmds.rs`/`image_cmds.rs` tests, and `type_complexity` in the brush comparison
example. These unrelated findings are kept separate from the new code.
The engine all-target check passes with those three categories allowed.

The 36 MP release smoke benchmark (`pen_pressure::benchmark_smoothing_end_on_36mp`)
uses a 6000×6000 white document, 500 timed pressure samples, a 40 px brush and
16-sample live-preview batches at smoothing 100. After one warm-up per option,
three whole-stroke runs measured 107.97/109.69/111.35 ms with end catch-up on and
42.20/42.94/44.15 ms with it off (medians 109.69 and 42.94 ms). The new default
avoids repeatedly rendering an optional tail. This small same-machine comparison
is not a full-document performance budget or Photoshop benchmark. Raw output:
`target/smoothing-perf.log`. The updated desktop build and `panic_hunt` pass;
layering remains 29 crates without violations.

## 36.2 MP performance smoke comparison

Release, the same benchmark cases before and after the renderer change, one run
each on this machine while other validation work was active. Times are milliseconds;
the run is too small and noisy to establish a performance budget or improvement.

| Brush / surface | Before | After |
| --- | ---: | ---: |
| Textured RGBA8 | 417.7 | 427.9 |
| Per-tip RGBA8 | 629.5 | 616.9 |
| Chalk RGBA8 | 369.2 | 389.1 |
| RGBA16 | 433.5 | 450.3 |
| RGBA32F | 457.2 | 459.3 |
| CMYKA8 | 442.6 | 447.6 |

`cargo xtask perf --quick` also completed successfully (all four quick benches).
Its results are in `target/perf/results.json` and `target/perf/summary.md`. The
baseline belongs to an Apple M4 Pro machine class, so the tool explicitly skipped
regression comparison on this Windows/RTX 3070 Ti machine; quick mode does not
enforce full-document budgets. No baseline or scorecard performance claim was
updated. Several general layout operations still exceed a 16 ms frame target.

## Synthetic pen injection (2026-11-14)

A physical tablet can't be used after every change. `prism-tablet` can now inject
pen strokes with pressure, tilt, rotation and timing through
`CreateSyntheticPointerDevice(PT_PEN)` + `InjectSyntheticPointerInput` (Windows 10
1809+). These are OS-level Windows Ink packets. They take the same path as a
physical pen: winit's emulated mouse/touch events, Prism's native history
capture, `pen_input` and the brush. The same stroke can also drive another
application to produce a comparison oracle. CLIP STUDIO PAINT must use
**TabletPC** (Windows Ink) in its tablet preferences; Wintab mode does not see
synthetic pens.

```sh
cargo run -p prism-tablet --example pen_inject -- --window Prism --list     # find the target
cargo run -p prism-tablet --example pen_inject -- --window Prism            # 0→1→0 pressure ramp
cargo run -p prism-tablet --example pen_inject -- --window "CLIP STUDIO" --ramp 200,300,600
cargo run -p prism-tablet --example pen_inject -- --window Prism --file trace.json --origin X,Y --scale S
```

`--file` takes `ui.penTrace` output or `[x, y, pressure]` / `{x, y, pressure, time?,
tiltX?, tiltY?, rotation?}` samples. Points are mapped to the target's client area as
`origin + point * scale` (for a trace in document coordinates, use the canvas origin
and zoom). `--hover N` sets the hover lead-in before contact, and `--dry-run` prints
the frames. Injection moves the real cursor and focuses the target window, so don't
use the mouse while it runs. It is a local measuring tool, not a CI test.

The safe frame encoding and stroke validation (`synthetic_pen`) are unit-tested on
every platform. On Windows the device boundary lives in `windows.rs` with the rest
of the Windows `unsafe`.

**Verified end to end** on this machine (3840×2088 client, 150 % DPI) against the
running app (`--control`, `ui.penTrace`). A 2304 px ramp produced one `down`, 576
`move` and one `up` event, all `source: windowsInk` with `pressureValid: true`.
The pressures rose 0 → 0.99 → 0 in 4/1024 steps at about 8.5 ms per packet. The
canvas showed the expected tapered line as one Brush Tool history step. Four more
ramps with 1, 2, 3 and 8 hover frames each committed one stroke.

Open observation: the very first injected stroke, sent right after Windows
brought the previously background window forward, reached the trace without its
`down` (moves and `up` only) and committed nothing. Later strokes, including those
with a single 16 ms hover frame, all committed. Whether a physical pen touching
just as the window activates loses its stroke the same way is not yet established.

## G-pen comparison with CLIP STUDIO PAINT (2026-11-14)

The same synthetic stroke drove CLIP STUDIO PAINT PRO and Prism: document x 200 → 1400 at a fixed
y, 301 samples 8 ms apart, pressure 0 → 1 → 0, no tilt. Both used a 1600×900 RGB canvas at the
same physical scale (0.899 screen px per document px) and the G-pen sub tool: CLIP STUDIO's own,
and Prism's from `ui.cspTool {"tool":"pen","subTool":"G펜"}`. Size was 25 in both (CLIP STUDIO's
user setting; Prism's imported default is 10, so it was set to 25). Stabilization was 6 and
"Adjust by speed" was on. CLIP STUDIO first used Wintab and drew the synthetic stroke at a
constant width, as from a mouse. It was switched to Tablet PC for the run and back afterwards.

Width is the coverage summed down each column (CLIP STUDIO measured at 100 % view, Prism from
the saved PNG):

| Pressure | 0.17 | 0.33 | 0.50 | 0.67 | 0.83 | 1.00 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| CLIP STUDIO (rising / falling), px | 2.6 / 2.7 | 7.0 / 7.1 | 11.3 / 11.4 | 15.4 / 15.5 | 19.5 / 19.6 | 23.4 |
| Prism, px | 4.4 | 8.9 | 13.2 | 17.2 | 21.3 | 25.2 |

- **Pressure response matches.** Both are linear with a zero minimum: about 24.9 px per unit of
  pressure in each. The peak sits at x 798 (CLIP STUDIO) and 802 (Prism), and the profile is
  symmetric in both.
- **Prism is a constant 1.6–1.9 px wider.** The gap does not depend on pressure. A CLIP STUDIO
  G-pen stroke of size 25 without pressure also measures 23.7 px, so this is how wide a dab of
  a given size is drawn (edge or anti-aliasing), not the pressure curve. The thin ends follow:
  ink above half coverage spans x 225–1376 in CLIP STUDIO and 220–1380 in Prism.
- Not measured: tilt, speed-dependent stabilization at other speeds, other sub tools, and opacity
  or density pressure.

**Cause of the width gap (same day).** The user's CLIP STUDIO G-pen and the installed default
differ only in size: 25 against 10, with hardness 100, thickness 100, angle 0 and automatic spacing
type 2 in both (read from copies of both `EditImageTool.todb`). So the gap comes from how each app
renders, not from the settings. Measurements at 100 % view, with CLIP STUDIO on Wintab (synthetic
pen = full-size mouse input):

| G-pen, size 25 | CLIP STUDIO | Prism |
| --- | ---: | ---: |
| One dab: area → equivalent diameter | 399 px² → **22.5 px** | 491 px² → **25.0 px** |
| One dab: centre column | 0.39, 0.79, 20 × 1, 0.79, 0.39 (22.4) | 0.13, 0.88, 23 × 1, 0.88, 0.13 (25.0) |
| Constant line: cross-section | 0.79, 22 × 1, 0.79 (23.6) | 0.82, 24 × 1, 0.82 (25.6) |

- **Overlapping dabs widen a line by a similar amount in both apps**: +1.1 px in CLIP STUDIO and
  +0.6 px in Prism. Both build up the partly covered edge pixels as dabs overlap (Prism:
  `CovMap::accumulate`, "over" unless wet edges).
- **The single dab is the difference.** CLIP STUDIO's medium anti-aliased round tip of size 25 is
  about 2.5 px smaller across than its nominal size. Prism draws the exact circle, and its test
  `csp_anti_alias_levels_sample_like_csp_and_parse_legacy_flags` asserts that "coverage across the
  dab stays its diameter". That assumption is the code-side cause.
- Across the pressure ramp, line width against nominal size fits `0.998·size − 1.21` in CLIP STUDIO
  and `1.006·size + 0.42` in Prism. The dab shrink therefore looks like a constant offset (about
  −1.2 px of radius), not a scale. This is fitted from line widths only; single dabs were measured
  at size 25 only.
**The rule, measured (same day).** Single full-pressure G-pen dabs in CLIP STUDIO at every
anti-aliasing level and five sizes (Wintab mouse input, 100 % view, area converted to an
equivalent diameter, px):

| AA level | 5 | 10 | 25 | 50 | 100 |
| --- | ---: | ---: | ---: | ---: | ---: |
| none | 5.05 | 10.09 | 25.03 | 50.16 | 100.12 |
| weak | 3.55 | 8.58 | 23.50 | 48.53 | 98.51 |
| medium | 2.86 | 7.65 | 22.54 | 47.53 | 97.52 |
| strong | 2.86 | 6.83 | 21.59 | 46.55 | 96.53 |

Radial profiles of the size-100 dabs show the mechanism. Coverage reaches 0 at exactly
r = size / 2 at every level. The anti-aliasing is a linear ramp laid **inside** that edge
(weak 48.5 → 50, medium 47.5 → 50, strong 46.5 → 50), not centred on it. All 20 dabs fit within
0.16 px:

    coverage = clamp((R − d) / w, 0, 1),  R = size / 2,  d = distance from the dab centre
    to the pixel centre,  w = min(W, R),  W = 0 (none), 1.5 (weak), 2.5 (medium), 3.5 (strong)

The cap w ≤ R explains why medium and strong are equal at size 5. Prism's computed round tip
centres its anti-aliasing on R (2×2 samples, plus a box for medium and strong), so it keeps the
nominal diameter at every level. That is the whole width gap: at medium, size 25 gives
22.5 px in CLIP STUDIO and 25.0 px in Prism.

**Fixed.** `prism-paint` now renders an anti-aliased hard computed CSP tip with this ramp,
evaluated once at each pixel centre. The edge distance is first order, (1 − q) / |∇q|, taken
through the linear tip map, so rotated and flattened tips follow the same rule. This replaces the
2×2 samples and box filter. The unit test
`csp_anti_aliased_round_dab_matches_measured_clip_studio_diameters` checks all 20 measured dabs
at three sub-pixel phases (within 0.25 px; 0.6 px for the binary level 0). Re-running the same
synthetic pressure stroke through the rebuilt app:

| Pressure | 0.17 | 0.33 | 0.50 | 0.67 | 0.83 | 1.00 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| CLIP STUDIO, px | 2.61 | 6.97 | 11.28 | 15.39 | 19.52 | 23.39 |
| Prism before, px | 4.42 | 8.88 | 13.18 | 17.19 | 21.35 | 25.24 |
| Prism after, px | 2.69 | 7.05 | 11.38 | 15.47 | 19.59 | 23.39 |

The mean |difference| over the stroke is 0.06 px. A single size-25 dab is 22.55 px in Prism and
22.54 px in CLIP STUDIO. Line fits: CLIP STUDIO `0.998·size − 1.21`, Prism `1.005·size − 1.33`.

**Performance.** `bench_brush` gained three CSP cases: hard computed tip, 8 % spacing, pressure
size, a 500-point stroke on a 6016×6016 RGBA8 layer. Release build on this Windows / Ryzen
(24 threads) / RTX 3070 Ti machine, three runs each, comparing `render.rs` before the fix
(`3d2be2c~1`) with after:

| Case | Before, total ms | After, total ms | Speed-up |
| --- | ---: | ---: | ---: |
| 25 px, AA medium (9 719 dabs) | 1 108 / 1 108 / 1 166 | 114 / 119 / 113 | ≈ 9.8× |
| 200 px, AA medium (1 216 dabs) | 5 439 / 5 430 / 5 713 | 597 / 591 / 591 | ≈ 9.2× |
| 200 px, AA strong (1 216 dabs) | 5 488 / 5 460 / 5 634 | 594 / 586 / 600 | ≈ 9.3× |

The old path took 2×2 samples per pixel, each averaging a 4×4 box (64 shape tests). The new path
evaluates one ramp per pixel. The worst 16-point chunk at 200 px fell from about 300 ms to
32 ms. `cargo xtask perf --quick` completed (all quick benches ran). Its baseline is from an
Apple M4 Pro machine class, so the regression comparison was skipped, and quick mode applies no
budgets. No baseline or scorecard number was changed.

Not covered: image (sampled) tips, elliptical tips (thickness < 100), and soft tips. CLIP STUDIO
forces anti-aliasing off for a soft round tip wider than 25 px of falloff (`docs/sut-analysis.md`
› Anti-aliasing levels).

## Remaining physical acceptance

No physical tablet stroke has been driven or observed by this validation run.
Long holds, actual barrel buttons and mouse right-clicks, weak starts, fast
successive strokes, releasing tails and detached windows still require a real
device check. Enable `ui.penTrace` only for the capture, then disable it. Its
device identifier is an opaque native handle, not a hardware model name.

Windows Ink is the native backend. WinTab-only input remains unsupported here.
Native history is bounded to 512 records per message and the transport to 4096;
overflow cancels an unfinished stroke with a diagnostic. Unknown device pressure
is not treated as maximum pressure. Timestamps have millisecond precision.

See [analysis evidence](pen-input-analysis.md) for verified imports, call sites,
inferences and unrecovered Photoshop algorithms, and
[control protocol](control-protocol.md#pen-input-diagnostics) for trace usage.
