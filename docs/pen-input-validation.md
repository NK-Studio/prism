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
