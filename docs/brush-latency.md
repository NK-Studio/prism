# Brush response investigation (2026-11-03)

The local CSP material `茂み1` corresponds to Prism's saved `茂み1 2` in
`3分で茂みブラシ`. Its source settings are size 100, stabilization 10,
adjust-by-speed off, stickiness 4 and ribbon quarter-turns 1. CSP's Sub Tool
Detail UI independently confirmed stabilization 10 and adjust-by-speed off;
post correction was unchecked. Measurements below distinguish rendering cost,
input sampling and visible response. They do not establish full CSP parity.

## Rendering cost and changes

The imported brush holds 10,930,815 RGB bytes in addition to its grayscale tip.
Each live tail preview cloned the generator and brush context, including these
pixels in three settings snapshots. `DabBuilder`, `DualBuilder` and `BrushContext`
now share their immutable settings with `Arc`. Editing the caller's brush still
creates an independent snapshot; preview clones do not copy its pixels.

New strokes also rebuilt tip mipmaps and premultiplied color planes. A bounded
two-entry, 256 MiB cache reuses prepared pixels. Exact asset equality detects
edits without relying on a hash. Current scalar settings replace the cached
snapshot on a hit. Normal-sized sampled brushes are prepared when selected;
large assets remain lazy. The cache does not change brush filtering or pixels.

Same machine/profile (`fast`, opt-level 2), 6000 × 4000 transparent document,
121 input points at 120 Hz, source size 100; wall-clock milliseconds:

| Operation | Before p95 | After p95 |
|---|---:|---:|
| Raster input only | 0.454 | 0.319 |
| Tail preview | 7.729 | 1.049 |
| Engine live update | 8.263 | 2.188 |

Before cold preparation was 97.587 ms; after cache misses remain approximately
94–108 ms, while cache hits measured p50 11–14 ms / p95 12–15 ms. The final run
measured cold preparation 98.430 ms, warm preparation p95 12.207 ms and warm engine
pen-down 13.003 ms, including settings snapshots. Preparation moves to
selection for this brush; it is not claimed to be free on first use.

These are CPU processing measurements, not hardware pen-to-photon latency.
The before/after comparison uses the same `fast` profile, not a release-LTO
baseline. Raw local outputs and recordings are ignored under
`target/brush-latency/`; no SUT assets or proprietary code are committed.

The final release-LTO run on the same 24 MP scenario measured cold preparation
87.373 ms, warm preparation p95 11.290 ms, raster p95 0.291 ms, tail preview
p95 0.968 ms, warm pen-down 12.931 ms and live-engine update p95 1.603 ms.
Intermediate runs, including a slower run during concurrent compilation, are
retained separately. There is no release-LTO before baseline for a speedup ratio.

Reproduce with a local downloaded material:

```powershell
$env:PRISM_CSP_LATENCY_UUID = '<local brush UUID>'
cargo run --release -p prism-engine --example brush_latency
```

The example imports into an in-memory session, creates a 24 MP document, and
prints preparation, raster, tail-preview and live-engine timings. It writes no
presets or documents. Optional `PRISM_TEST_CSP_ROOT` selects a local CSP store.

## Original application comparison and limits

CSP and the isolated Prism test app use separate 512 × 512 white documents, brush
size 100 and a 378-document-pixel horizontal mouse gesture. OS scale is 150%:
CSP 100% and Prism 66.7% produce equal physical canvas sizes. The input source
is a constant-pressure mouse drag, not the user's physical pen.

CSP's earlier window capture achieved approximately 60 fps, but native wgpu
window-GDI capture returned Prism's stale initial surface. It is not a valid
Prism timing source. The final recordings instead capture the visible desktop
canvas region only (CSP 60 fps, Prism 58 fps); both show the full fast stroke
reaching near the release point. The animated OS cursor cannot identify actual
button-down time, so the earlier cursor-to-stroke interval is not an input
latency measurement. Display scanout and physical-pen timestamps are not instrumented.

The first Prism comparison exposed a separate input issue: a complete fast
mouse gesture between frames could be lost or treated as a click at its final
point, losing the path needed by a ribbon. A command-driven stroke with the same
loaded brush rendered correctly. A raw single-frame regression fails on the
original canvas handler (zero strokes rather than one). The handler now
preserves the actual press, moves and release, checks ownership at the press
layer, and feeds held strokes even before the drag threshold is reached.
Stationary clicks retain their existing behavior. Test batches use
`Harness.input_mut().events`: `Harness.event` processes each queued event in a
separate frame and cannot reproduce this issue.

The visual rerun then exposed the missing second release phase: with only two
input positions, stabilization 10 / stickiness 4 had no initial repeats, leaving
the endpoint around 40% of the requested distance. The final gesture drain in
the existing CSP analysis had not been implemented. Prism now drains release
pressure with bounded work, stopping at pressure zero or settling motion and
interpolating a zero-pressure endpoint. Dots keep their contact pressure. See
[the provenance and remaining release/device differences](sut-analysis.md#stabilization-and-adjust-by-speed-2026-10-16).

The final native rerun reaches the end of the same 378 px flick, whereas the
intermediate build painted only about 150 px. In the final desktop videos,
CSP's first visible paint is at 12.333 s and its full stroke at 12.350 s;
Prism's first paint and full stroke are both visible at 16.167 s. These are
independent recording timelines, not synchronized input timestamps or evidence
that Prism is faster than CSP. They establish successful visible drawing and
endpoint recovery for this quick mouse gesture. Physical-pen latency and the
progression of a held stroke require a different measurement setup.

Stabilization itself remains sample-count based. A synthetic 600 px/s probe
gives 75/37.5/18.75 ms equivalent lag at artificial 60/120/240 Hz. These are
neither measured user input rates nor CSP measurements. The filter must not be
disabled or retuned solely from those numbers. Physical-pen history, pressure,
slow/fast corners, release behavior and random ribbon appearance remain
separate acceptance work under [Spec.1.2.2](../doc/Spec.1.2.2.md).

## Validation

- Paint: 190 tests pass, including sparse flick completion, stationary dot pressure,
  idempotent release, chunking, immutable preview snapshots and cache invalidation/bounds.
- Engine: 1,062 library tests pass (18 ignored); adversarial command `panic_hunt` passes.
- IO: 206 library tests pass (9 ignored). The earlier modified-IO release corpus
  gate passed 484 tests, with 10 ignored; the final latency changes are in paint/UI.
- UI: all new input regressions pass; sequential library suite has 1,511 passes,
  17 ignored and three failures in unchanged Save UI path tests. Two compare a
  directory with/without a trailing Windows separator; one expects an overwrite warning.
  The parallel suite aborted with `STATUS_INVALID_HANDLE`; the sequential suite completed.
- Final local library rerun: 156 ready sub-tools import; all 154 brushes paint
  nonempty strokes; zero import/stroke errors. 39 materials still warn. This is
  a smoke check, not a CSP pixel oracle or complete compatibility certification.
- Paint all-target Clippy passes. Broader/scoped engine and UI Clippy remain
  blocked by existing lints in Affinity, plugins, color/fill/image commands and
  analysis/gradient/native-dialog/transform UI code (including the unchanged
  Gradient match arm in `canvas.rs`). New brush/input code adds no listed lint failure.
- Layer checks pass for all 29 crates. Targeted formatting and `git diff --check`
  pass; repository-wide formatting already reports unrelated files.
- Release `perf_scenarios --quick` completes. It is a smoke run; full-size
  budgets do not apply, and the old baseline is from a different machine class,
  so its automated performance regression comparison is skipped.

The corrected native test app is separate from the user's original running
Prism, uses an isolated settings directory, and keeps the original documents intact.
One automation limitation remains: selecting this large color-tip preset can
complete but exceed the control channel's 8 MiB response limit when returning
all brush pixels. The native UI and in-memory benchmark work; compact brush
responses require a separate follow-up. No transport limit was raised or bypassed.

## Mouse lag and duplicate preset highlight (2026-11-04)

Reported: drawing `茂み1 2` with a mouse trailed far behind the pointer, as if a separate
smoother ran; picking it also highlighted its sibling bushes.

- **Lag.** Windows delivers at most one mouse move per frame (about 60 Hz while Prism renders),
  and mouse drag points carried no time. The CSP stabilizer counts its window in samples, so
  level 10 averaged about 167 ms of mouse movement versus about 80 ms for a 125 Hz pen. Mouse
  drag points are now timed by frame (`Stylus::frame_ms`), and the stabilizer averages the same
  span of time as input at a 125 Hz reference (`REFERENCE_INTERVAL_MS`): sparse input uses
  proportionally fewer samples; input at or above the reference, untimed automation input and
  pen-up repeats are unchanged. Synthetic 600 px/s line at level 10: 36 ms trail at 125 Hz,
  within 1.35× of that at 60 Hz (`sparse_input_averages_the_same_time_span_as_reference_rate_input`).
  The reference rate is an estimate; CSP's own mouse sampling and device scaling remain untraced.
- **Highlight.** The preset list highlighted every preset that `is_current` matched, which
  ignores size and colour: the 茂み bushes share all other settings. The highlighted preset is
  now the session's picked preset (`ToolState::brush_preset`), falling back to the first match
  (`brush_panel::current_index`).

## UI frame time with a large CSP brush (2026-11-04)

Measured through the control channel on the dev build (`cargo build`), Clip Studio workspace
restored, `茂み1 2` selected, a 60 Hz `ui.pointer` drag; `perf.uiMs` is the CPU time to build a
frame, `perf.timings.spans.ui*` splits it by area (new):

| Change | UI ms per frame | fps |
|---|---:|---:|
| Before | ~50 (idle and drawing; spikes to 900) | 14–19 |
| Material list lays out only the rows in view (`show_rows`; all 444 rows were built every frame) | ~26 | 30 |
| Tip pixels shared between brush clones (`paint::shared::Shared`, copy-on-write) | ~2 | 60 |

The options bar, Brush Settings and Tool Property each cloned the brush twice and compared it
every frame; with this brush that copied and scanned about 11 MB of colour tip plus its 16-bit
shape per clone. Clones now share the buffers and equality short-cuts on shared storage, so
those per-frame checks cost microseconds. The window also presents with one queued frame instead
of two (`SurfaceConfig::LOW_LATENCY`), removing about 17 ms at 60 Hz. Physical pen-to-photon
latency remains unmeasured; the first stroke after selecting a large brush still pays its
one-time tip preparation (~100 ms cold, see above).
