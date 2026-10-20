# Clip Studio Paint SUT investigation

Investigation: 2026-10-15, on `codex/feature-sut-brush`. This continues the initial
importer described in [sut-brushes.md](sut-brushes.md). Successful import is not
evidence of matching Clip Studio rendering.

## Scope and provenance

The maintainer requested local REA analysis of Clip Studio Paint for SUT support.
Only static, read-only inspection of the installed application, its directly
related resources and two local SUT files was performed. Proprietary code,
decompiler output, executables and brush assets are not copied into Prism source.
Prism's Rust reader is implemented independently from observed format facts and
the public [SQLite file format](https://www.sqlite.org/fileformat.html).

| Target | Identity |
| --- | --- |
| Executable | `C:\Program Files\CELSYS\CLIP STUDIO 1.5\CLIP STUDIO PAINT\CLIPStudioPaint.exe` |
| File version | `5.0.4.0` |
| Size | 92,879,872 bytes |
| SHA-256 | `2e08e084c7f2256d4bba1d6a5bfcdd3498b8cd026667e1952e116c56c9ac9a2a` |
| Tools | REA `6.4.0`, Ghidra `12.1.4 PUBLIC`, Node `24.21.0`, Temurin JDK `21.0.12.1+1` |

The existing isolated tools are under ignored `target/rea-tools`. This run's
requests, complete evidence, scripts and identical executable copy are under
`target/rea-tools/csp` (helper launch scripts are alongside it). No global REA
registration, installed application, or application settings were changed.

### REA coverage and direct Ghidra fallback

REA's initial sandboxed target open failed at Windows path canonicalization.
An authorized execution outside that sandbox reached target validation, which
rejected this executable's overlapping DOS/PE headers: `e_lfanew` is `0x10`,
with `PE\0\0` at that offset; REA 6.4.0 requires an offset of at least 64.
The original bytes were preserved. Neither a successful native REA session nor
full-program REA analysis is claimed.

REA's independent PE resource operation **did** succeed on the original file:

- Evidence ID: `ev_b7218f56d145aa7dad019da09a7ff245516b2cf544aa9494dd342fc010e0ae12`.
- Provider: `rea-pe-resources`, version `1`.
- Retained response: `evidence/03-resources.json`.
- Scope: static resource-tree inventory. Payloads other than icon-group metadata
  remain opaque; this does not establish brush behavior.

Direct Ghidra imports the identical executable copy using `-noanalysis`,
`-readOnly` and `-deleteProject`. Its PE loader reports an overlapping-header
data insertion warning, then successfully imports the image with zero additional
files loaded. Focused decompilation is recorded separately, **not** as REA
Evidence. Raw byte scans are candidate discovery, not validated instructions.
Recovered signatures and types are provisional; the large brush settings
function reports unsettled type propagation. Findings use concrete field
offsets, keys and call relationships rather than treating inferred types as
original declarations. `evidence/manifest.json` records retained output hashes.

## Findings

### Material references: observed framing, implemented

Independent SQLite inspection of the two local files found the same framing in
`BrushPatternImageArray` and `TextureImage`:

| Offset | Encoding | Meaning |
| --- | --- | --- |
| 0 | big-endian u32 | Header size, observed value 8 |
| 4 | big-endian u32 | Number of reference records |
| Each record + 0 | big-endian u32 | Total record byte length, including this field |
| Each record + 4 | big-endian u32 | Path byte length |
| Each record + 8 | UTF-16LE | Exact `MaterialFile.OriginalPath` |
| Remaining record bytes | opaque | Names and other material metadata; not needed to resolve the path |

The two-tip array is 520 bytes: 8-byte header plus records of 258 and 254
bytes. The sampled texture array is 310 bytes: header plus a 302-byte record.
Evidence is retained in `evidence/sut-0.json` and `evidence/sut-1.json`, each
including original-file SHA-256. These are independent SQLite observations,
not REA Evidence. The initial synthetic fixtures had omitted record lengths;
they now use this observed framing and original synthetic paths/pixels.

The reader now resolves full paths, preserves reference order and repetitions,
and rejects malformed framing, invalid UTF-16, missing paths and ambiguous
duplicate material rows. It no longer searches for path substrings anywhere in
the blob. Unknown metadata is skipped within the validated record length.
Coverage is the observed layout; other material formats remain unsupported.

### Identifier precision and texture recovery: source-confirmed fixes

The original reader converted SQLite integers to floating point, then compared
Variant IDs after parameter clamping to ±1,000,000. Different variants could
therefore alias. SQLite integer values now retain their signed 64-bit identity;
node/variant selection and schema page IDs use an exact integer accessor.
Integral REAL values remain accepted within the signed 64-bit range.

A non-BLOB `TextureImage` formerly returned an error before entering the
recoverable texture-import boundary. It now produces a warning and keeps the
brush, matching the behavior already used for unreadable texture materials.

### Texture mode mapping: initial label inventory

The installed English resource contains texture labels with resource paths
`[1,14,1,26]` through `[1,14,1,35]`: Normal, Multiply, Subtract, Compare,
Outline, Overlay, Color dodge, Color burn, Hard mix, Height. This is a resource
inventory, **not** a mapping from `TextureCompositeMode` values. The parser and
file hash are retained in `evidence/resource-labels.json`.

The [official Texture documentation](https://help.clip-studio.com/en-us/manual_en/810_subtools/T.htm)
describes mode choices and per-tip versus per-stroke application, but does not
specify serialized enum values. Its displayed list also differs from the
installed resource inventory. Unused resource labels may exist; neither source
proves that every label is an active menu entry in this build.

Focused Ghidra confirmed property-name initialization at `0x1401108b0`
(`TextureCompositeMode`, global `0x1454de8c0`), `0x14010ba80`
(`BrushSizeEffector`, global `0x1454deb30`), and `0x14010b690`
(`BrushPatternImageArray`, global `0x1454def80`). These initialize named keys;
they do not establish rendering formulas. See `evidence/ghidra-focused.txt`.

The settings reader `0x142b5c0f0` and writer `0x142b5ee30` connect the
texture-mode key to internal property ID `0x66`. The larger brush readers/writers
`0x142b79920` / `0x142b8fff0` connect `BrushSizeEffector` to `0x3e9` and
`TextureDensityEffector` to `0x70`. Both use the same effector serialization
helpers. Evidence: `evidence/ghidra-globals.txt`. This narrows the next trace;
an internal property ID is not a serialized mode enum.

### Follow-up: verified texture dispatch and independent float implementation

The property-to-renderer chain is now established: settings builder `0x142b5e030`
copies property `0x66` to configuration `+0x14`; renderer setup `0x14266e450`
copies it to texture object `+0x48`. Sampling dispatch at `0x14266e089` uses ten
image-relative branch targets from `0x14266e320`. Each target was independently
disassembled/decompiled after automatic switch recovery failed. Successful direct
Ghidra outputs are `texture-pass1/2/5/6/7/8/9.txt` under `target/rea-tools/csp`.
Passes 3/4 had script errors and are **not** evidence despite process exit status 0.

The ABR conversion bridge `0x142f0a5c0`, reached from `0x142f0b9d0`, independently
links named tokens to codes 1,2,3,5,6,7,8,9. Code 2 accepts `linearBurn`, and
subtraction tokens additionally toggle inversion; code 9 merges Height and
linearHeight. Thus reusing Photoshop's similarly named modes is insufficient.
Names Normal/Outline for codes 0/4 are an inference from the remaining resource
labels and the eight independently linked modes; their **numeric behavior** is
directly verified by renderer branches `0x14266e145` and `0x14266e1cb`.

For normalized coverage `v`, adjusted darkness `q`, density `d`, `x=q*d`, and
coverage ceiling `h`, the stress-disabled equations are:

| Code | Prism CSP mode | Result before final range clamp |
| --- | --- | --- |
| 0 | Normal | `v*(1.5-x)` |
| 1 | Multiply | `v*(1-x)` |
| 2 | Subtract | `v-x` |
| 3 | Compare | `min(v,1-x)` |
| 4 | Outline | `v*(1+d)-x` |
| 5 | Overlay | Standard overlay of `v` with `1-x` |
| 6 | Color dodge | `min(1,v/x)`, with the zero divisor handled explicitly |
| 7 | Color burn | `max(0,1-(1-v)/(1-x))`, with endpoint cases handled explicitly |
| 8 | Hard mix | `2*v-x` (continuous, unlike the existing binary mask) |
| 9 | Height | `min(h,10*d*v-(1-q)*h)` |

Density setter `0x14266dab0` confirms Height's gain of 10 and separate coverage
ceiling. Caller `0x142650000` scales sampled coverage by the same state field
passed as `h` by `0x142669c60`. Prism applies per-tip CSP modes to flow-scaled
coverage and passes that scale to Height. Applying nonlinear modes before flow
would give a different result. Stroke-level textures use Prism's normalized
stroke buffer. Full CSP stroke accumulation/opacity equivalence remains unverified.

The raw texture luminance becomes darkness before the clipped brightness/contrast
curve; inversion changes both that conversion and the brightness sign. Tone
adjustment happens after bilinear sampling. Setup `0x14266e450` and independently
decoded floating constants establish these transforms. The UI swatch shares the
same tone function, and CSP brightness/contrast controls use a ±100 range.

The model stores `texture.cspMode` separately from existing `texture.mode`; old
presets and ABR imports preserve their previous mask behavior. Known SUT values
0–9 select the source-specific equations. Unknown values retain a warning and
fall back to CSP Multiply. The existing brush command, replay, preview cache and
disk preset store preserve the mode and independent pressure curves.

Native calculations use 15-bit fixed-point coverage and density quantized to
1/1024; Prism uses bounded floating-point equations at all supported document
depths. This is mathematical mode support, **not** a claim of bit-identical CSP
pixels. Emphasize density uses a separate image-reference calculation and remains
an explicitly warned contrast approximation; rotation and matched resampling
also remain outside this implementation.

### Effector flags: native corroboration, activation fix implemented

Local default effector blobs have a nonzero word at bytes 4–7 (`0x1f0` or
`0xf0`) while bytes 8–11 are zero. Other effectors have nonzero words at
bytes 8–11 and appended curve data.

Direct Ghidra corroborates the distinction:

- Reader `0x142cbf0d0` reads the header length, then assigns serialized bytes
  4–7 to object offset `+0xc` and bytes 8–11 to object offset `+8`.
- Writer `0x142cc0240` emits these two object fields in that same order.
- Constructor `0x142cbeb80` initializes the field at `+0xc` to `0xf0` and
  the field at `+8` to zero.
- Consumers `0x142cbf8b0` and `0x142cbf9d0` require bit `0x10` in **both**
  fields before configuring the corresponding curve/parameter. They similarly
  gate other controls using matching bits in both fields.

Evidence: `evidence/ghidra-effector.txt` and `evidence/ghidra-flags.txt`.
The field roles are inferred as supported controls and active controls from
initialization, serialization and consumption together. The first control's
pen-pressure interpretation is corroborated by the subsequent runtime consumer
trace below. Other input sources remain incompletely mapped. No runtime execution was observed.

The importer now requires both `0x10` bits, with a bounded header of at least
40 bytes (the native reader's unconditional fields). Previously the supported
mask alone enabled pressure even when the active mask was zero. The fix applies
to both brush size and per-tip texture density. That initial fix did not add custom
curves; the follow-up below adds curves, minimums and opacity/flow mappings.

### Follow-up: independent pressure responses

The same executable identity was reverified before continuing. Direct Ghidra
outputs `ghidra-curves-1.txt` through `ghidra-curves-5.txt` are retained under
`target/rea-tools/csp`; these are not REA Evidence objects.

The effector reader/writer (`0x142cbf0d0`, `0x142cc0240`) establish a big-endian
header with a percentage minimum at byte 12, pressure-curve byte length at 32,
second curve length at 36, and third length at 44 for headers of at least 48 bytes.
Observed headers are 40, 44 and 48 bytes. Curve blocks follow the header in order.
The minimum consumers `0x142cbf8b0` / `0x142cbf9d0` multiply by the independently
decoded constant 0.01. The parameter consumer `0x1425717f0` applies
`minimum + (1 - minimum) * response(pressure)` before other enabled controls.

Curve reader `0x1423f1fc0` and writer `0x1423f2680` establish three big-endian u32
fields: header size (12), point count (at most 16), and point-record stride (16).
Each point stores two big-endian f64 coordinates. The default constructor
`0x1423f1c50` uses endpoints (0,0) and (1,1). The importer checks lengths and
bounded counts before allocating, validates finite normalized coordinates and
strictly increasing input coordinates, and rejects invalid responses with a
parameter-specific warning while retaining the brush.

Evaluator `0x1423f0ef0` dispatches to `0x143209150`. Two points define a straight
line; with three or more, interior points are quadratic Bezier controls, joined
at adjacent controls' midpoints. First and last points are endpoints. This is
not straight interpolation through every stored point. Boundary fallback uses
the segment's endpoint chord (`0x141a943e0`). Prism independently evaluates the
standard quadratic equations and clamps the resulting normalized response.
Synthetic analytic checks include the three-point curve (0,0), (0.5,1), (1,1),
whose response is `2*x - x*x`, and a four-point midpoint-join example.

Size, opacity, flow and per-tip texture density each retain their own response
and minimum. Curves are immutable shared data in `prism-paint`, serialized as
bounded point arrays, applied after pressure-sample interpolation, and preserved
by the existing brush commands and preset store. Legacy presets omit the optional
`pressureCurve` field and retain their previous linear behavior. Unsupported
enabled effector controls produce warnings instead of silently becoming pressure.
The engine and isolated paint tests enable serde_json's accurate `float_roundtrip` parser: default
JSON parsing changed 935 of 10,006 independently generated fractional f64 values
by an ULP in a local check. A regression test round-trips adjacent input coordinates
exactly, preventing persisted curves from drifting or collapsing two input points.

The two local files contain 3-, 4- and 16-point responses; their private values
are retained only in `evidence/curve-inventory.json`. Synthetic tests use original
invented points. Neither the binary nor proprietary curves are runtime inputs to
Prism's implementation.

## Initial importer validation (2026-10-15)

- `cargo test -p prism-io -j2`: 368 passed, 2 ignored across 44 suites,
  including all 13 SUT unit tests. Synthetic cases cover malformed framing,
  exact signed 64-bit identifiers, texture recovery and independent pressure flags.
- Both available local SUT files import, paint nonblank strokes and survive
  preset-store save/load. The final report explicitly records zero failed imports,
  blank strokes, persistence failures or panics. An earlier sandboxed run could
  not persist either preset; its passing test status alone was not acceptance.
  Final report: `target/rea-tools/csp/evidence/sut-import-final-report.md`.
- Engine `panic_hunt` passes with adversarial command parameters.
- `cargo xtask layers`: 29 crates, no violations. Changed-file Rust 2024
  formatting and `git diff --check` pass.
- `cargo clippy -p prism-io --all-targets --no-deps -j2 -- -D warnings`
  passes. The same command without `--no-deps` stops at the existing
  `clippy::nonminimal_bool` in `crates/text/src/cjk.rs:71`; a fully clean
  dependency-inclusive Clippy run is not claimed.
- `cargo xtask test-corpus -p prism-io` verifies the pinned corpora and runs
  all library/integration tests with `corpus,heif`. The default thin-LTO build
  was stopped during test linking; the final run keeps release optimization
  with `CARGO_PROFILE_RELEASE_LTO=false`, `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`
  and two build jobs. Final result: **389 passed, 0 failed, 2 ignored across
  43 suites**, including `psd_tools_mutations_never_panic`; command exit status 0.
  Log: `target/sut-corpus-no-lto.log`. A long tool/host interruption occurred
  during the mutation test, so its wall time is not a performance measurement.

These checks establish importer robustness and local round trips, not matched
Clip Studio rendering. No pixel-parity or physical-pen comparison was performed.

## Curve and texture follow-up validation (2026-10-15)

- Library tests: paint **101 passed**, IO **115 passed / 1 ignored**, engine
  **993 passed / 13 ignored**. Coverage includes all ten texture equations,
  tone-after-sampling and alpha-before-mode ordering, independent pressure
  responses, exact f64 persistence, malformed command atomicity and 8/16/32-bit
  incremental-stroke equivalence. Log: `target/sut-verified-libs.log`.
- The two new CSP UI tests pass, covering dropdown selection, command patch
  replay, preset selection and preview-cache invalidation. The final full UI
  library run is **1374 passed, 3 failed, 13 ignored**. Its failures are the
  existing Windows save-path tests: `a_full_path_in_the_name_is_taken_as_the_location`,
  `a_picked_file_sets_the_folder_name_and_format` and
  `saving_over_a_file_open_in_another_tab_warns`. Logs:
  `target/sut-final-ui-focused.log`, `target/sut-final-ui-all.log`.
- Pinned IO corpus: **392 passed, 0 failed, 2 ignored across 43 suites**;
  `cargo xtask test-corpus -p prism-io` exited successfully with release LTO
  disabled, 16 codegen units and two build jobs. Log: `target/sut-final-corpus.log`.
- Both available local SUTs import, paint nonblank strokes and preserve their
  settings exactly through preset-store save/load: **2/2**, zero failed imports,
  blank strokes, persistence failures or panics. Report:
  `target/rea-tools/csp/evidence/sut-curves-textures-report.md`.
- Paint/IO scoped Clippy passes with `--all-targets --no-deps -D warnings`.
  Strict engine/UI Clippy encounters existing `nonminimal_bool`,
  `manual_range_contains`, `collapsible_match` and `type_complexity` findings in
  unrelated files (the last is `engine/examples/brush_compare/main.rs:964`);
  a completely clean workspace lint run is not claimed.
  Engine/UI all-target scoped Clippy passes with only those four pre-existing
  lint categories allowed on the command line; no source-level allowances were
  added. Log: `target/sut-final-clippy-known-baseline.log`.
- `cargo xtask layers`: **29 crates, zero violations**.
- Engine `panic_hunt` passes with adversarial command parameters (1 test,
  11.94 s); log: `target/sut-final-panic-hunt.log`. Changed-file Rust 2024
  formatting and `git diff --check` pass. Binary SHA-256 is unchanged.
- Native offscreen UI snapshots were rendered and visually inspected at
  1440x900: `target/sut-csp-texture-ui.png` and
  `target/sut-csp-texture-menu.png`. Korean Outline selection, all ten menu
  choices, brightness 37, contrast -33 and the pressure-shaped stroke preview
  are present. The script uses an original synthetic stroke and procedural
  texture, not private SUT assets. Snapshot logs identify the Microsoft Basic
  Render Driver; their frame timings are not a hardware-GPU performance claim.
- Release pressure benchmark: 6000x6000 sparse canvas, 40 px brush, 500 input
  points, 3310 dabs, 85 touched tiles, warmup then three-run medians: baseline
  **44.14 ms**, two-point identity **43.59 ms**, sixteen-point identity
  **43.33 ms**. Pixel and dab equivalence is checked outside timed regions.
  These nearby timings do not establish a speedup. One million response
  evaluations take 2.34 / 9.25 / 21.74 ms respectively. This is a sparse brush
  workload, not a dense 36 MP scan. Log: `target/sut-pressure-perf.log`.
- `cargo xtask perf --quick` completed successfully. Its baseline comparison
  was skipped because the stored baseline is macOS/M4 Pro and this host is
  Windows/AMD with RTX 3070 Ti; tracked over-budget scenarios remain. No budget
  or checked-in baseline was changed. Log: `target/sut-final-perf-quick.log`;
  measurements: `target/perf/results.json`.

These results validate the independently implemented equations and persistence,
not matching CSP pixels or physical pen behavior. Native stress/emphasize-density
and stroke accumulation remain outside the verified model.

## Remaining research

1. Validate the static texture equations against matched rendered strokes, including
   fixed-point rounding, stroke-level opacity/flow, emphasize density and resampling.
2. Resolve the remaining effector controls (tilt, speed, random, etc.); verify curves
   with matched settings and pressure traces before claiming rendering fidelity.
3. Verify tip-order enums, dual-brush behavior, texture rotation and wet effects.
4. Re-run the prior 237-file collection on a machine where it is available.
   This Windows checkout has two local samples; the earlier count is historical.
5. Compare matched strokes in Clip Studio and Prism. Static decompilation and
   successful imports do not replace pixel or physical-pen comparison.

## Grainy Pen mouse-stroke follow-up (2026-10-15)

The maintainer supplied a mouse-stroke comparison and a CSP UI capture showing
15 px, 90% opacity and 322.4% zoom. The file's stored brush size is 10 px; the
comparison harness overrides that size and uses constant pressure 1. The
user's screenshots are visual references, not identical-path pixel oracles.

### Observations and independent implementation

- The material's last PNG is a **thumbnail**, not the original raster. The local
  texture thumbnail is 300×300 while its linked original is 501×501; the tip is
  52×72 in both. A bounded native Rust reader now validates ustar checksums,
  C2F framing/CRC, complete SQLite schema records and the linked original raster,
  then decodes bounded 256×256 zlib blocks. It supports the observed one-channel
  8-bit black/alpha layout and (2026-10-16, the Gouache tip) the gray layout: pixel
  kind 17, two planes per tile (Parameter `[17, 1, 1, 2, 65536, 1, 256, 1, 256, 65536,
  256, 256, 8, 8]`), the alpha plane first and the gray plane second, matched against
  the material thumbnail value for value. Each block header gives the expanded tile size
  (65536 per plane). Monochrome materials (리얼 G 펜) use the black/alpha layout with
  32-byte rows and trailing word 1: one alpha bit per pixel, most significant bit first
  (8192-byte tiles), matched bit for bit against the thumbnail. That material's SQLite
  database also uses 4096-byte pages: the encoded 5120-byte prefix then hides pages 1–2,
  and recovery tries 1024- and 4096-byte pages, accepting exactly one validated reading.
  RGBA materials (kind 33, five planes) remain unsupported and warn before thumbnail
  fallback.
- This is limited recovery, not a complete C2F decoder. Its encoded prefix is
  unavailable. Only complete schema leaf records in the first 512 plaintext
  pages are candidates; linked roots, text encoding, ownership and raster layout
  must all validate. The normal SUT database parser retains its strict header
  checks. Synthetic tests cover UTF-8/UTF-16, old trailing NULL columns, cropped
  tiles, CRC/length/checksum failures, cycles and decompression output limits.
- The spray property reader and per-step loop establish independent area size,
  particle size and count. `BrushSprayDensity=8` is eight particles per step;
  `BrushSpraySize=2` is the particle diameter, not the 15 px brush-area diameter.
  `BrushSprayBias=-100` places particle centres on the circumference. Prism
  preserves these separately in `cspSpray`, using its own deterministic random
  streams and a 256-particle work cap. Particle size/count retain independent
  pressure curves. Positive distribution-bias branches were cross-checked
  against instructions before deriving the normalized mathematical behavior.
- Particle rotation is separate from the area rotation. The local file's
  `BrushRotationEffectorInSpray=3` does not enable its stored random amount 100;
  the effective particle angle is zero. Unsupported active rotation controls,
  synchronized/physical particle sizes and fixed-point placement warn.
- The texture factory builds at most two reduced levels when both original
  dimensions exceed three. Each step discards odd trailing edges and uses a
  2×2 integer average with downward rounding: 501→250→125. Nominal level scales
  are 100/50/25%, independent of those rounded dimensions. At 25% the selected
  125×125 level has effective scale 1. Below 25% that same level is sampled at
  the remaining scale. Sampling uses integer destination coordinates without
  a half-pixel offset: nearest at requested scale ≤100%, bilinear above it.
- No evidence supports bypassing a parameter's pressure curve for mouse input.
  Texture density's consumer uses the ordinary pressure path. Constant mouse
  pressure therefore remains 1 and each parameter evaluates its own curve.

Direct Ghidra outputs (not REA Evidence records) and private assets remain under
ignored `target/rea-tools/csp`: `texture-mouse-pass1..5.txt`, `spray-pass3..6*`,
`spray-evidence-manifest.json`, `texture-findings.md`, `spray-findings.md` and
`evidence/grainy-raster-ledger.json`. The executable identity above is unchanged.

### Limits

General RGB and continuous mask paths confirm coverage scaling before texture,
but the scaling member has not been proven to be the UI Flow control. An earlier
binary auxiliary-mask path alone cannot establish general brush alpha semantics.
Random streams, exact accumulation, wet edge, automatic spacing and remaining
input controls do not yet have matched CSP pixel validation. The implementation
must not be described as complete CSP rendering parity or behavior-only research.

### Small-particle coverage and texture polarity

The initial spray implementation exposed a second problem: treating CSP's
disabled anti-aliasing as Prism's Pencil mode thresholded small, partially
covered particles away. These are different sampling rules. Direct read-only
Ghidra traces in `spray-aa3.txt` through `spray-aa5.txt` establish:

- `0x1425260a0` selects reduced tip levels only for anti-alias settings above 1.
- `0x14264a900` dispatches setting 0 to the unfiltered scan path.
- `0x14264b4b0` maps destination pixel centres into the original tip raster;
  `0x142641900` reads nearest source alpha and scales coverage by alpha/255.
  There is no 50% alpha cutoff. The auxiliary binary-mask branch is not the
  ordinary brush-alpha path.

Prism now stores `cspAntiAlias` independently of its existing Pencil `aliased`
flag. Setting false samples the original tip with nearest sampling, preserves
fractional alpha, and does not snap the dab centre to the Pencil grid. Synthetic
tests preserve a 0.4-alpha, 2 px particle at 8/16/32f while checking that legacy
Pencil thresholding is unchanged. The enabled levels are mapped too; see
"Anti-aliasing levels" below.

The texture sampler also required an explicit polarity conversion: Prism's
public texture tile stores luminance, whereas the native CSP sampler consumes
the black/alpha plane. Convert `g = 1 - luminance` before the source-specific
tone equations. Reduced levels average integer alpha, rounding down, rather
than averaging floating luminance. Synthetic tests cover polarity, rounding,
odd dimensions, sample phase and unchanged legacy texture behavior.

### Matched Prism comparison and runnable build

The local comparison uses the supplied SUT, red paint, 15 px, opacity 0.9,
constant pressure 1, no smoothing, and the same 321-point sinusoidal path over
a 0.6-gray background. The before/after images are
`target/rea-tools/csp/grainy-matched-before.png` and
`target/rea-tools/csp/grainy-matched-after.png`. The old smooth band becomes a
granular stroke with visible small particles and gaps. Compared visually with
the supplied CSP screenshot, **Prism still has more gaps**. The reference uses
a different path and screen zoom; it is not a registered pixel oracle. Do not
equate fixing particle loss with complete CSP density or accumulation parity.

The native app was built with `cargo build -p prism --profile fast -j2` to
`target/fast/prism.exe` (2026-10-15 20:32 KST, 67,682,816 bytes). Reimport the
SUT before testing: an existing saved preset retains its previously decoded
thumbnail and settings. Set brush size to 15 px and opacity to 90% explicitly.

A 6000x6000 sparse-canvas smoke benchmark used 500 inputs, one warmup and three
timed runs: single-tip configuration 16,260 dabs / 134.720 ms median; CSP spray
130,080 dabs / 90.729 ms. The configurations have different footprints and
outputs, so these numbers are not evidence of an equivalent-work speedup or a
dense 36 MP processing budget. Log: `target/grainy-spray-perf.log`.

### Follow-up validation

- Paint unit tests: **108 passed**. IO unit tests: **127 passed, 2 ignored**;
  explicitly running the private Grainy original-raster oracle also passes.
- Engine: **994 passed, 13 ignored**. UI: **1374 passed, 3 failed, 13 ignored**.
  The three failures are the unchanged Windows save-path tests listed in the
  preceding validation section. New spray command persistence/validation and
  preview cache checks pass. Log: `target/grainy-engine-ui-final.log`.
- IO release corpus: **404 passed, 0 failed, 3 ignored across 43 suites** with
  two build jobs, LTO disabled and 16 codegen units. Log:
  `target/grainy-corpus.log`.
- Engine adversarial-input `panic_hunt`: **1 passed**, 12.76 s. Layers:
  **29 crates, zero violations**. Relevant four-crate all-target no-deps Clippy
  passes with only the previously documented four baseline categories allowed
  on the command line. No source lint allowances were added. Logs:
  `target/grainy-panic-hunt.log`, `target/grainy-layers.log` and
  `target/grainy-clippy-final.log`. `git diff --check` passes.
- Native offscreen UI capture `target/grainy-final-ui.png` was inspected at
  1440x900. It displays the actual after-render image, 15 px / 90% controls,
  and a nonblank synthetic spray preview using the changed preview scaling.
  This UI check uses a synthetic tip, not an assertion that the current UI
  preset is the imported Grainy preset. The capture uses Microsoft Basic
  Render Driver; its timings are not hardware-GPU performance measurements.
- `cargo xtask perf --quick` completed with exit 0; measurements are in
  `target/perf/results.json` and `target/grainy-perf-quick.log`. Existing
  over-budget UI scenarios remain. No tracked budget or baseline was changed;
  the cross-platform stored baseline is not a same-machine regression oracle.

## 240% tip thickness and live CSP comparison (2026-10-15)

The previous importer incorrectly constrained `BrushThickness=240` to legacy
roundness 1.0. Native property 0x41a is converted by 0.01 into runtime +0x208
in `0x142b80a20`; property 0x41c selects the thickness axis through flag 0x40.
`0x1422e0db0` evaluates thickness, `0x142618d40` forwards it, and
`0x14264b860` applies the selected local-axis scale before rotation. The
minimum for positive radius r is `max(t, min(1, 1/r))`, with no upper-one
clamp. For this SUT the local Y scale is 2.4: the 52x72 sampled tip at 2px
particle diameter has support approximately 1.444x4.8px. This does not change
the spray-area diameter, count or spacing. Raw findings and constants are in
ignored `target/rea-tools/csp/thickness-findings.md` and associated JSON.

Prism preserves this as optional `cspTipScale: [1, 2.4]`, independently of
legacy roundness. Rendering applies it in tip coordinates and expands damage
bounds to avoid clipping rotated tips. Presets and preview cache preserve the
field. Commands reject invalid scales and scaled diameters above 5000px;
direct rendering bounds them defensively. The 1000% scale cap is Prism's
work limit, not a verified CSP UI maximum. Thickness pressure/random controls
remain outside this static-scale implementation.

Live CSP was inspected using Computer Use after the maintainer resumed control.
Its Grainy Pen settings visibly confirm 15px, 90%, thickness 240% with vertical
direction and 2px particles. A separate transparent 512x512 document was
created, a horizontal mouse drag drawn, and the original PNG saved as
`target/rea-tools/csp/csp-live-15px-90.png`. The user's original illustration
was retained. Prism renders use the same nominal dimensions/settings, a
horizontal path x64..444 at y254 and pressure1. The actual mouse sample
stream and random seed are not synchronized, so this is a controlled visual
and statistical comparison, not a registered pixel-equality oracle.

`thickness-live-comparison.png` shows CSP / before / after on identical 0.6-gray
backgrounds at 3x nearest enlargement. No generated artwork is substituted.
Metrics in `thickness-live-metrics.json` use x100..409, y247..261:

| Measurement | CSP | Prism before | Prism after |
|---|---:|---:|---:|
| Mean core alpha | 0.7169 | 0.3817 | 0.6481 |
| Nonzero core fraction | 1.0000 | 0.5305 | 0.8286 |
| Core fraction above alpha 0.5 | 0.6761 | 0.4585 | 0.7604 |
| Maximum alpha | 1.0000 | 0.9020 | 0.9020 |
| Full nonzero vertical extent | 26px | 16px | 20px |

The thickness correction substantially changes coverage, but **the results
are not identical**. CSP has faint edge pixels and stronger peak accumulation;
Prism retains more empty pixels and a different distribution of middle alpha.
The SUT enables unsupported `BrushUseWaterEdge`; that is a concrete remaining
lead, not proof it alone explains every difference. Exact accumulation,
input sampling and random-stream differences also remain unverified.

Current native test build: `target/fast/prism.exe`, 2026-10-15 23:16 KST,
67,658,752 bytes, SHA256
`c47aa579f911fbff153cbf97ed552a020ab46110791103acc8627c464fb3bb2c`.
Reimport the SUT before testing. Paint109/IO128 unit tests and the explicit
local 501px material oracle pass; engine995 pass. UI1375 pass with the same
three pre-existing Windows save-path failures (13 ignored in engine and UI).
The sandboxed engine run failed filesystem operations; the same current test
binary passed outside the sandbox. See `target/thickness-engine-verified.log`
and `target/thickness-ui-verified.log`.

Final IO corpus:405 pass, zero failures, three ignored across43 suites.
Current panic_hunt and layers29/0 pass. Scoped all-target Clippy passes with
only the four previously documented baseline categories allowed on the command
line. Current offscreen screenshot `target/thickness-final-ui.png` was visually
inspected:15px/90% controls, actual Prism after PNG and a separate synthetic
preview with the new scale. The snapshot uses Microsoft Basic Render Driver.

The final `cargo xtask perf --quick` completed and wrote
`target/perf/results.json` (log `target/thickness-perf.log`). Existing UI
scenarios remain over budget; no performance baseline or budget was changed.
All scheduled validation is complete; the three known UI path failures and
CSP visual differences above remain open.

## Imported tool-property presentation (2026-10-16)

The local CSP 5.0.4.0 executable retains SHA256
`2e08e084c7f2256d4bba1d6a5bfcdd3498b8cd026667e1952e116c56c9ac9a2a`.
This investigation used authorized read-only native analysis and direct UI
experiments on a duplicate sub-tool; it is not strictly behavior-only research.
Raw Ghidra output, private SUTs and SHA256 inventories remain in ignored
`target/rea-tools/csp/display-*` artifacts. Prism's implementation is independent.

Confirmed structure:

- `NodeVariantID` selects current display and brush settings; `NodeInitVariantID`
  selects initial settings. Grainy and another local pen have two corresponding
  rows. `Manager.CommonVariantID=44` has no matching exported row in these files;
  Prism does not fabricate or merge that absent row.
- `VariantShowParam` is a big-endian u32 array frame: header byte size 12, item
  count, element byte size 4, followed by the visible property IDs. Native writer
  `0x1429e9590` reserves/backfills this header. Reader `0x1429e8e90` applies the
  IDs as visibility flags on available property records and retains unknown IDs.
  It does not reorder known controls; writer emits their canonical order.
- `VariantShowSeparator` is the per-variant **Show Category** option. Key reader
  `0x1429e91c0` and writer `0x1429e98d0` bind it independently of the array.
- On a duplicate named Prism Display Research, turning only Brush Size's eye
  off removed ID 1000 and changed count 12 to 11. Turning Show Category on changed
  only the category scalar from 0 to 1. Export counters and archive timestamps
  also changed; those are not display controls.
- Changing Opacity from Show Slider to Show Indicator changed no Node/Variant
  fields in the exported SUT. Selecting original Grainy retained its own category
  and visibility but shared the indicator setting. Indicator style therefore
  belongs to app display preferences, not imported preset metadata. The original
  slider was restored after observation.
- Texture Density's plus button expands the Scale ratio child (ID 120); the
  collapsed view hides it even when it is present in ShowParam. The UI experiment
  restored the collapsed state. Arbitrary user reordering remains unverified.

Verified display properties include Brush Size 1000, Opacity 10, Composite Mode
30, Anti-aliasing 20, Thickness 1050, Amount of paint 1081, Density of paint 1083,
Color stretch 1085, Texture 100, Texture Density 110, Texture Scale 120,
Watercolor edge radius 1201, Stabilization 40, Adjust by speed 41 and vector
magnet power 1241. Property 1201 contains radius plus units; its UI checkbox also
needs the separate `BrushUseWaterEdge` flag. Grainy's mixing values 80/64/42 are
stored despite both watercolor-enable flags being zero, explaining disabled rows.
These mappings are supported by the existing native property-reader/writer traces,
not inferred only from UI order.

Prism stores explicit optional SUT provenance on the preset, outside BrushSettings:
original display membership/category, optional user override, input/output operator
IDs and an allowlist of source scalar values. Legacy and ABR presets have no SUT
metadata. Valid empty visibility means hide all; malformed/missing display framing
warns and falls back to size/opacity without rejecting an otherwise usable brush.
The 512-item display limit is Prism defensive policy, not a claimed CSP maximum.
Unknown IDs remain bounded and preserved. `PrismTextureScaleFactor` is Prism-owned
presentation metadata recording material downsampling, allowing the UI to show the
source scale percentage while retaining the renderer's bounded tile scale.

The original texture's display name is inside an encoded C2F material catalogue
not decoded by the current importer. `texture_name` stays empty for these files;
the UI must use a generic imported-texture label rather than invent a name. This
work does not establish every tool family's conditional control layout or implement
unsupported watercolor/magnet/stabilization rendering.

Control availability is not inferred from scalar-column presence: SUT Variant rows
can retain values for disabled or inapplicable controls. Import preserves operator
IDs exactly (including unrecognized families); missing IDs stay unknown rather than
being guessed as the known brush family. The initial presentation catalog groups
Ink mixing before Anti-aliasing and Brush Tip controls as observed in CSP. The two
original local visibility lists do not by themselves establish the relative order
of every hidden category, so no universal ordering claim is made for other operator
families. Unknown visible parameter IDs produce an explicit import warning while
remaining preserved in the original metadata.

For operator families not yet mapped, the panel still binds verified visible
property IDs to the imported runtime BrushSettings (size, opacity and other
implemented values). Showing stale source values here would disagree with actual
painting after a value edit. Only additional hidden-control availability is gated
on the verified operator family. The other local pen's seven-row layout was also
rendered separately from Grainy's twelve-ID layout; unknown extra settings remain
explicit rather than inferred from a name or the presence of a texture.

The local Grainy panel comparison uses the imported material at 15 px / 90%:

| CSP reference | Prism presentation |
| --- | --- |
| Brush Size 15, Opacity 90 | Actual brush values, editable with px / % units |
| Amount / density / stretch 80 / 64 / 42, disabled | Imported values with an inactive-CSP-settings explanation |
| Texture density 100, scale 25 under the expander | Actual texture values with percentage units and expandable scale |
| Watercolor edge enabled, radius 2.0 | Read-only source value and unsupported-effect explanation |
| Stabilization 10, adjust by speed enabled | Read-only source values and unsupported-effect explanation |
| Material texture title | Generic imported-texture label until its catalogue title is decoded |

Local offscreen evidence is `target/rea-tools/csp/prism-sut-settings-ko.png`,
`prism-sut-details-narrow-ko.png`, `prism-sut-settings-scrolled-ko.png`,
`prism-sut-categories-narrow-ko.png` and `prism-sut-other-layout-ko.png`.
The Korean narrow viewport is 460x680; the ordinary comparison is 1200x900.
These are real native UI renders. This display work does not establish pixel-identical
CSP strokes or implement the remaining unsupported rendering effects.

## Watercolor edge (2026-10-16)

Same CSP 5.0.4.0 executable (SHA256 above). Authorized read-only Ghidra analysis;
outputs `water-edge-pass1..5.txt` and `we6..we10.txt`, the private findings file
`water-edge-findings.md` and launch logs stay under ignored `target/rea-tools/csp`.
They are direct Ghidra observations, not REA Evidence records. Prism's
implementation (`crates/paint/src/csp_water_edge.rs`) is independent.

**Settings.** The property builder `0x142b80a20` reads enable 1200, radius 1201
(with unit), opacity influence 1202 and darkness influence 1203 (both /100),
process-after-drag 1204, and blur 1205 only inside the after-drag branch. Config
transfer `0x1422e5020` and the stroke setup `0x142566fc0` pass them to the edge
processor `0x1425c7f80`, which derives `R = r + 1`, window half-size `n = ceil(r)`,
`a = clamp(opacity, 0.01, 1)`, near amount `3a`, slope `10a/R`, darkness
`round(200 * darkness)` 8-bit levels, and enables blur only above `1e-8`.

**Rendering.** About sixty composite writers (`0x142650000`..`0x14266c...`), after
`mask * opacity` and the stroke texture and before the selection, call the
per-pixel edge functions (`0x1425c5e80` gray, `0x1425c6100` RGB, `0x1425c5d30`
alpha-only). For a covered stroke-mask pixel, `0x1425c7450`/`0x1425c7b50` find the
squared Euclidean distance `d2` to the nearest zero mask pixel in the
`(2n+1)²` window (initial value `(n+1)²`) and the window's maximum mask `M`. When
`0 < d2 <= R²`, with `d = sqrt(d2)`:

    f = (d > 1) ? (R - d) * 10a / R : 3a
    w = f * M * (0.95 + 0.0004 * bayer16[y mod 16][x mod 16])
    alpha = min(1, alpha + w)

The dither table at `0x1444ff8e0` (read by `0x142578ef0`) equals the standard
16×16 Bayer matrix; Prism generates it from the 2×2 base pattern (verified to
match all 256 entries). Darkness uses `share = w / (alpha_before + w)`: RGB scales
all channels by `(max - levels*share) / max` (zero when not positive); gray subtracts.

**Process after drag.** Without it the edge is live while drawing. With it,
`0x1425c5a40` runs once after the drag: `0x1425c6490` writes an 8-bit amount
`min(255, round(255 w))` for covered pixels; if blur is enabled, `0x1424b5740` →
`0x1424b3220` applies a separable integer Gaussian (horizontal then vertical, in
64-pixel aligned blocks). Kernel builder `0x1424b57d0`: `b = min(blur, 500)`,
`σ² = -b² / (2 ln(1/255))`, half-width `trunc(b)`, centre weight 65535 and
`trunc(65535 exp(-i²/2σ²))` taps; the 8-bit pass `0x1424b3c00` outputs
`floor(Σwv / Σw)` over taps inside the line. `0x1425c7250` then marks zero mask
pixels with a nonzero blurred amount (within `ceil(blur)` of the stroke) so they
composite, producing a faint halo; `0x1425c59b0` makes the writers add
`((E*257+1) >> 1) / 32768` instead of the live amount.

**Prism.** `BrushSettings.cspWaterEdge` is optional (old presets unchanged) and
bounded (radius ≤ 100 px is Prism's work policy; blur ≤ 500 px is the source
clamp). Live edges are recomputed for the dirty tiles plus neighbours within the
radius, so chunked painting equals the committed stroke at 8/16/32-bit (tested).
After-drag edges, blurred or not, apply only to the committed composite. The blur
field is limited to 64 Mi pixels (Prism policy; larger strokes commit the unblurred
edge). Coverage below one 15-bit step counts as empty. On a sparse 36 MP canvas
(diagonal 500-point 40 px stroke, release, medians of three) the live edge costs
53 ms against 40 ms without it, and the after-drag blur commit 64 ms against 9 ms
(`cargo run --release -p prism-paint --example water_edge_bench`).

**Known differences / not established.** Edge style, strength and vividness
columns have no identified consumer in this path and warn when nonzero style is
set. The per-pixel colour variant (`0x1425c6830`, used when both stroke colour
buffers exist) is not mapped; Prism darkens the stroke colour. The radius/blur
scale factor (`+0x110` of the stroke object) is assumed to be 1 for canvas-pixel
units, and non-pixel units warn. CSP excludes pixels outside the image from the
"empty" search; Prism's stroke buffer has no canvas rectangle, so it treats them
as empty.

**Live comparison.** Using the existing CSP capture `csp-live-15px-90.png` (same
nominal settings, unsynchronized input/random stream, so statistics only) and
Prism renders of the same horizontal path through `prism-cli`, alpha metrics over
x 100..409 (core rows 247..261 for the means):

| Measurement | CSP | Prism, no edge | Prism, edge | Prism, edge + 50% size |
|---|---:|---:|---:|---:|
| Maximum alpha | 1.000 | 0.902 | 1.000 | 1.000 |
| Core mean alpha | 0.717 | 0.653 | 0.867 | 0.710 |
| Core nonzero fraction | 1.000 | 0.833 | 1.000 | 1.000 |
| Core fraction above 0.95 | 0.607 | 0.000 | 0.752 | 0.596 |
| Vertical extent (px) | 26 | 20 | 34 | 26 |
| Rows with mean alpha > 0.4 | 248..259 | — | 245..262 | 248..259 |
| Faint pixels (0 < a < 0.1) | 4319 | 105 | 4157 | 4329 |

The halo decay outside the band matches closely. With the edge alone the band is
too wide. Grainy's `BrushSizeEffector` has active controls `0x30`: pressure plus
`0x20`, with a stored 50% field and a second curve `(0,1) (0.5,1) (0.8,0) (1,0)`.
If `0x20` is the tilt response whose input is 1 for a vertical pen or mouse (the
interpretation used by the separate in-progress `Dynamic.csp_tilt` work), mouse
strokes paint the spray area at 50% size. Rendering at 7.5 px reproduces every
measurement above, including the identical solid rows, which also supports the
earlier 240% particle thickness. This is a statistical match on one brush and one
path, not pixel identity; tilt mapping itself is not part of this change.
Images: `target/rea-tools/csp/water-edge-live-comparison.png` and the three
`prism-*water-edge*.png` renders.

## Effector tilt response (2026-10-16)

Same executable. Existing direct Ghidra outputs (`evidence/ghidra-effector.txt`,
`ghidra-flags.txt`, `ghidra-curves-3.txt`, `accumulation-pass2.txt`) settle the
second effector control. Reader `0x142cbf0d0` assigns header words in order: 4
supported, 8 active, 12 pressure minimum, 16 → `+0x18`, 20 → `+0x20`, 24 → `+0x24`,
then curve lengths at 32/36 and, for headers of at least 44 bytes, word 40 → `+0x1c`
(otherwise 100); 44 is the third curve length. Consumer `0x142cbf9d0` passes
`+0x18`/`+0x1c` (×0.01) and the second curve to setter `0x142572c80` when bit `0x20`
is supported and active (`0x40` uses `+0x20` and the third curve, `0x80` uses `+0x24`).
Evaluator `0x1425717f0` multiplies the pressure result by
`A + (B − A) · curve(input)` with `A` = word 16 and `B` = word 40, so the two
values need not be ordered and the response is independent of pen pressure.

Grainy's `BrushSizeEffector` has `A = 50`, `B = 100` and the curve
`(0,1) (0.5,1) (0.8,0) (1,0)`. The importer now maps active `0x20` onto the
existing `Dynamic.cspTilt` (input 1 = vertical pen or mouse, 0 = horizontal);
`CspTilt` no longer requires `minimum <= maximum`. Speed (`0x40`) and random
(`0x80`) still warn. The exact native altitude input for physical pens is not
traced here; the mouse value is supported by the comparison below.

Rendering the imported SUT unchanged (15 px, 90%, same path) now gives maximum
alpha 1.000 (CSP 1.000), core mean 0.710 (0.717), core fraction above 0.95 0.596
(0.607), vertical extent 26 (26), solid rows 248..259 (248..259) and 4329 faint
pixels (4319): byte-identical to the 7.5 px prediction above. Image:
`target/rea-tools/csp/tilt-live-comparison.png`. This remains a statistical match
on one brush and one unsynchronized path, not pixel identity.

## Stabilization and Adjust by speed (2026-10-16)

Same executable; direct Ghidra outputs `stab1.txt`..`stab12.txt` under ignored
`target/rea-tools/csp`, not REA Evidence. Prism's `crates/paint/src/csp_stabilizer.rs`
is independent.

**Settings.** Key reader/writer bind `FlickerReduction` to property `0x28` (integer),
`FlickerReductionBySpeed` to `0x29` (flag), `FlickerReductionBySpeedType` to `0x2a` and
`Stickness` to `0x32` (display IDs 40/41 are the first two). Reader `0x142b5b1b0`
(called by `0x1429a7f30`) returns the level, "speed type 0", "other speed type", the
stickiness and `0x413`; `0x1429a7f30` passes them to two input filters through setter
`0x141afbd40` (for device type 4 they are first scaled by 1.3 and 0.4). The setter
stores level `S`, stickiness `P`, start window `S` if `S <= 2` else
`clamp(min(S, P) >> 1, 2, 15)`, and for speed type 0 with `S > 2` the speed scale
`K = 2S + 100`.

**Filter.** `0x141afb2e0` appends each input (x, y, pressure, time in ms) to a
1000-entry ring and calls `0x141afb5c0`, which outputs the average of the newest `W`
samples with a fractional weight for the next older one; positions and pressure are
averaged, and missing or >1000 ms old samples repeat the oldest sample still used.
`W` is a double:

- at pen-down (state 0): the start window;
- without speed adjustment: `W += min(0.5, (S − W + 1)/S)` per input, up to `S`;
- with speed type 0 (from the second sample): `c = (K − d)/K` with `d` the distance from
  the previous raw sample, halved when positive; `W += c`, minimum 2, maximum `S`
  (start window while not yet drawing). A gap over 1000 ms passes the raw sample.

- with another speed type and `S <= 30` (setter flag `+0x3c`): over the newest
  `min(n, 30)` samples, the mean non-zero step length `v` (screen pixels) and the
  straightness `q` = bounding-box diagonal / path length; target
  `int(q·(2.5 − v)·20)` when `v < 2.5`, else `S`; at least `S` while drawing (the start
  window before); `W` moves a quarter sample per input towards it (snapping within a
  quarter) and stays within `min(n, 30)`, `n − 1` and 100. The 2.5 and 20 are the filter
  constructor's defaults (`0x141afade0`: `+0x44` = 25, `+0x40` = 200, both /10). Above
  level 30 the plain ramp applies. A second flag (`param_5`) that would use half-sample
  steps is not mapped.

**Pen-up** (`0x141afc580`, events 6/9) feeds the release sample again
`floor(W) − P` times, so with stickiness 0 the line reaches the release point.

**Not established / not mapped.** The state machine `0x141afafe0` can end the stroke
early while decelerating below a threshold (2 px, or a stickiness-dependent value)
during catch-up; Prism always completes the repeats. The pressure of the release
sample and device-type scaling are not modelled; Prism repeats the last input.
Distances use document pixels × zoom as screen pixels. No matched CSP capture of a
stabilized stroke has been made.

## Start and end (2026-10-16)

Same executable; direct Ghidra outputs `inout1.txt`..`inout8.txt` (private, not REA
Evidence). Prism's `crates/paint/src/csp_in_out.rs` is independent.

**Settings.** `BrushInOutType` 0x488 (0 length, 1 percentage, 2 fade),
`BrushInOutBySpeed` 0x489, `BrushInOutTarget` 0x48a, `BrushUseIn`/`InLength`/`InRatio`
0x492–0x494 and `BrushUseOut`/`OutLength`/`OutRatio` 0x49c–0x49e. Reader `0x142b6f9a0`
returns the lengths (unit-converted pixels, or ratio × 0.01). The target array is a
big-endian frame `[12, count, 12]` followed by `(effector property, enabled, value %)`
entries; applier `0x142cc1260` turns an enabled entry into evaluator control `0x100` with
minimum `value × 0.01`, and evaluator `0x1425717f0` multiplies the parameter by
`minimum + (1 − minimum) · factor`. Effector IDs in the observed arrays: 0x70 texture
density, 0x3e9 size, 0x3fd flow, 0x411 spacing, 0x41b thickness, 0x43a/0x43c colour
mixing, 0x445 blur, 0x44c sub colour, 0x450/0x452/0x454 hue/saturation/value,
0x46c/0x46e spray size/density, and six vector-related IDs.

**Taper.** The whole path is tapered after the fact (`0x1430cd8d0`, `0x142b8d050`):
percentages multiply the path length; with "by speed" each length is limited to the path
covered by its first (or last) `trunc(length · 0.04)` input segments (at least one); if
start + end exceed the path both are scaled to fit. `0x142495e50` inserts path points at
the boundaries and sets each point's factor to `sin(90° · s/start)` or
`sin(90° · d/end)` (a one-degree sine table with linear interpolation).

**Prism.** `BrushSettings.cspInOut` (mode, start/end, by speed, targets size, flow,
texture density, spray size and spray density). The taper is evaluated per dab from the
path distance with an exact sine. A committed stroke (`render_stroke`, symmetry commit,
`dabs`) resolves the lengths from the walked path after stabilization and smoothing;
a stroke being drawn knows only a pixel-length start, so the end appears on release.
Fade (type 2), non-pixel units and other targets (spacing, thickness, colour, blur)
warn. Both local SUTs have start and end disabled, so this is checked only with
synthetic settings, not against a CSP render.

## Color mixing (2026-10-16)

Same executable; direct Ghidra outputs `mix1.txt`..`mix7.txt` (private, not REA
Evidence). Prism's `crates/paint/src/csp_mixing.rs` is independent.

**Settings.** `BrushUseWaterColor` 0x43e, `BrushWaterColor` 0x438 (mixing type),
`BrushMixColor` 0x439 (amount of paint, display 1081), `BrushMixAlpha` 0x43b (density,
1083), `BrushMixColorExtension` 0x43d (color stretch, 1085), `BrushColorMixingMode`
0x43f, effectors 0x43a/0x43c. The builder `0x142b80a20` stores enable `+0x358`, type
`+0x35c`, amount/100 `+0x368`, density/100 `+0x3c8` and stretch/100 `+0x428`.
`0x1422e09a0` evaluates amount and density per dab through their effectors (capped at
1); with mixing off they are 1 and stretch 0.

**Pick-up.** While a dab is rasterized (`0x14264cb30` → `0x14261e9c0`), every footprint
pixel adds the layer pixel with the stroke coverage composited over it: alpha sum,
alpha-weighted colour sums and a pixel count (transparent pixels count). Before the next
dab, `0x14261e4f0` forms the mean alpha and alpha-weighted mean colour (white when no
alpha) and blends them into the picked colour with `k = 0.05 / (stretch + 0.05)`: the
first pick is taken directly; with no picked alpha the new colour replaces the old one
at alpha × k; an empty sample only fades the alpha by `1 − k`.

**Dab colour.** `0x142562f50` (type 0): dab opacity `(1 − d)·A + d`; for amount
`a < 1` the colour is `(brush·a² + picked·(1 − a²)·A) / (a² + (1 − a²)·A)` (the picked
colour itself at `a = 0`). The first dab has no pick and paints the brush colour.

**Compositing.** A mixing dab moves the pixels it covers towards its colour *and* its
alpha (`(1 − d)·A + d` times the dab opacity), weighted by the tip value, so paint can
lower the alpha of what it covers. Each dab lays over the result of the previous ones:
a later dab's colour and alpha replace earlier ones even where coverage is already full.
The first dab picks up what is under its own footprint (pen-down pick).

**Prism.** `BrushSettings.cspMixing` with per-dab `paintAmount`/`paintDensity`
(effector dynamics), a picked-colour reservoir in `StrokeRenderer`, and the layer's
pre-stroke pixels as the sampling source (commit, live and symmetry paths); without a
source only the stroke itself is picked up. The coverage map keeps each pixel's mixed
colour plus target alpha and, for mixing brushes, lets every dab paint over the stored
values (`paint_over`); compositing lerps colour and alpha towards them
(`csp_mixing::blend_towards`). Samples are taken after each dab is accumulated, over
the dab's whole tip square (`in_tip_quad`). Mixing types other than 0 (separate "running"
samplers) warn and use this blend.

**Pick-up footprint.** `0x14264cb30` walks every pixel of the dab polygon's scan spans and
passes each to `0x14261e9c0` with stroke coverage 0, so the pick-up counts the whole tip
square (the quad `0x142640510` builds from the dab centre and radius), transparent pixels
included, not only pixels the tip covers. `0x14261e9c0` adds the layer alpha and the
alpha-weighted colour (converted to the perceptual space first in that mode).

**Perceptual mode** (`BrushColorMixingMode` 1 → dab `+0x24`; property `0x440` → `+0x28`,
absent from both local SUTs, so 0). Pick-up sums (`0x142577960`), the reservoir blend
(`0x142577700`, `0x142577910`) and the amount-of-paint blend in `0x142562f50` all average
in one space built by `0x142577b00` from constants in the executable: RGB through a
per-channel curve, a 3×3 matrix to cone responses, then a PQ curve per channel; the
inverse uses the inverse matrix and curves. The matrix is the sRGB primaries' RGB→XYZ
(0.4124/0.3576/0.1805, …) followed by Hunt–Pointer–Estevez (0.38971, 0.68898, −0.07868 /
−0.22981, 1.18340, 0.04641 / 0, 0, 1), each cone normalised by the D65 white
(0.95047, 1, 1.08883). The PQ constants are m1 0.1593017578125, 1/m2 0.0126833…,
c1 0.8359375, c2 18.875, c3 18.6875, with white at 80 of 10000 cd/m² (`0x1425779b0`,
`0x142577a30`). The RGB curve is one of five tables filled by `0x1425772d0` with blend
parameters 0, 0.25, 0.5, 0.75, 1 of an sRGB-style curve (`0x142578ab0`: exponent
`1 + 1.4p`, offset `0.055p`, linear toe below 0.03928); index 0 (the default) is the
identity, so the RGB values are taken as linear. Prism: `csp_mixing::to_perceptual`.

**WaterColor2.** The writer `0x142b8fff0` stores property `0x43e` under both
`BrushUseWaterColor` and `BrushUseWaterColor2`; the second key is a mirror, not a
separate setting. Prism enables mixing from either.

**Live comparison.** A CSP-exported round mixing brush (size 50, amount 80, density 50,
stretch 100, automatic spacing type 3, perceptual mode on) drawn in CSP 5.0.4 across red
and yellow stripes on a transparent 512² canvas (private `csp-mix-live.png`) and
rendered by Prism from the same SUT: the alpha ramp along the stroke (131→237 in CSP)
is within ±2 at every sampled x; over the stroke core the mean |Δalpha| is 1.09 (p95 2)
and the mean |ΔRGB| ≈ 1. The match needed both the paint-over compositing and the
automatic spacing below (with the 10% fixed interval Prism plateaued at 225 instead
of 237). Private comparison `mix-live-comparison.png`.

With CSP's anti-aliasing (this SUT is weak, level 1), the whole-square pick-up and
perceptual mixing, the ramp matches to x = 140 (132/164/186/207/219 against
131/164/187/207/220) and the plateau is 231–235 against 237. Colours are closer (R 26
against CSP 26 at x = 260, RGB mode gave 30). The stroke edge differs: CSP's top and
bottom rows are 0.72 of the core and its coverage width is 49.4 px; Prism's hard 2×2
sampled tip gives 50.0 px with no partial rows. A pick-up square of 0.96·r would match
the plateau exactly, but nothing in the evidence gives that factor, so it is not used.
Neither gap can be settled without a controlled capture of a non-mixing round brush.

## Automatic spacing (2026-10-16)

`BrushAutoIntervalType` (sub-object `+0x200`) selects spacing: the equality check
`0x1422e5380` only compares `IntervalBase` (`BrushInterval`/100) when the type is 0, and
the dab-step function `0x1422e0db0` uses `IntervalBase` (with its effector) only for
type 0. Types 1–3 call `0x1422e4830(type, hardness, thickness)`: base 0.12 (type 1),
0.08 (type 2), 0.04 (type 3), times `3.5 − 2.5·max(hardness, 0.2)` and, when the
thickness is below 1, times `max(thickness, 0.01)`. Hardness counts as 1 for image tips
and at ≥ 0.99999999. The step is `2·max(radius, 0.1)·ratio`, i.e. the ratio is a fraction
of the diameter. Types 4 and 5 use a separate distance rule and warn in Prism (mapped
to type 2). A cap of 0.25 under an unidentified flag (`+0x78 & 0x8000`) and the minimum
step for soft tips are not reproduced. Prism: `prism-io` `sut::spacing`.

## Anti-aliasing levels (2026-10-16)

Direct Ghidra outputs `aa6`–`aa8` and `spray-aa3`–`aa5` (private). The dab renderer
`0x14264b860` stores the AntiAlias level (0 none, 1 weak, 2 medium, 3 strong; dab `+0x2c`,
forced to 0 by `0x1422e0db0` for a soft round tip wider than 25 px of falloff) and asks
`0x142526160` → `0x1425260a0` for the tip bitmap. Levels above 1 pick a reduced level from
the tip's mip chain (`0x1424b7aa0`: the smallest level whose scale is at least
`2r·100/N`, halved for strong, N the larger tip side); weak and none use the original.
The per-pixel sampler `0x142641180` then reads, for a level other than 0, four nearest
texels from two scanlines half a pixel apart, each stepping half a pixel (`0x142644ac0`
sets the step to 16384/32768), and divides their sum by 4·255; level 0 reads one nearest
texel at the pixel centre. There is no bilinear filtering at any level.

Prism (`render.rs`, `csp_aa`): level 0 samples once at the pixel centre; other levels
average 2×2 nearest samples at ±¼ pixel around the centre (the source starts its rows at
the pixel corner, but CSP's own stroke edges are symmetric, so the offsets are taken as
centred); sampled tips use mip level 0 / `level_for(d)` / `level_for(d/2)`. A computed
round tip is sampled as its exact shape (hard) or Prism's falloff (soft); medium and
strong average a 1 px or 2 px box around each sample to stand in for the reduced levels
of CSP's generated tip bitmap, whose resolution is not known. `cspAntiAlias` is now a
level 0..3; older presets stored a boolean (false = 0, true = 1). Grainy Pen (level 0)
renders unchanged.

## Tip rotation controls (2026-10-16)

The builder `0x142b80a20` stores `BrushRotation` (0x424) at `+0x268`, the
`BrushRotationEffector` flags (0x426) at `+0x270` and `BrushRotationRandomScale` (0x427) / 100
at `+0x278`. The dab step `0x1422e0db0` computes the angle only when flag 0x01 is set: the
base angle, plus an input angle at sample `+0x28` when 0x10/0x20 is set (else one at `+0x48`
when 0x40 is set), plus for 0x80 a random turn `(r·2⁻¹⁵ − 0.5)·scale·360°` from the stroke's
LCG (`×0x41c64e6d + 0x12d687`, bits 16..30), wrapped to 0..360°. Flag 0x02 does not enter
the angle. Observed values: 3 (base only), 19 (Gouache: base + direction), 131 (the Grainy
pens: base + random). Prism had mapped 3 to the line direction without evidence and ignored
131, so 거친 펜 (thickness 60%, random 100%) drew a smooth ellipse instead of CSP's rough
edge. Prism now maps 0x10/0x20 to the direction control and 0x80 to angle jitter
`scale / 2` (Prism's jitter spreads ±360°·j); 0x40 warns. The `+0x28` input is taken to be
the line direction from the 0x13 Gouache setting; it is not otherwise traced. *Superseded on
2026-10-19 (see Tip order, rotation input and colour tips): 0x10/0x20 are the pen's direction
and axis rotation, 0x40 the line direction.* Prism's
random stream is its own, so the individual dab angles differ from CSP's.


## Sub-tool operation IDs (2026-10-17)

A `.sut` holds one CSP *sub-tool*, and sub-tools are not only brushes: fill, selection,
gradient, text, figure, ruler and other tools export the same way. The shipped tool database
(`Settings/PAINT/Tool/<lang>/EditImageTool.todb`, a SQLite tree tool → group → sub-tool) gives
every default sub-tool's `Node.NodeInputOp`, `NodeOutputOp` and `NodeIcon`, which is what the
table below is built from (observed, not traced in code). The output operation decides the
family; the input operation only separates variants (31/51 enclose-and-fill vs 50/50 bucket).

| Output op | Family | Prism |
|---|---|---|
| 10 | brush (brush, airbrush, decoration, blend, sketch) | brush preset |
| 11 | pen / pencil | brush preset |
| 12 | eraser | brush preset (Eraser tool) |
| 13 | copy stamp | brush preset |
| 14 | dot pen | brush preset, 1 px, AA off |
| 20 | selection | Clip Studio workspace |
| 31, 50, 51, 53, 54 | fill (refer layers, enclose, lasso fill, close gaps) | Clip Studio workspace; 50/50 runs on Paint Bucket, 31/51 on the Lasso as enclose and fill |
| 52 | gradient | Clip Studio workspace (Gradient) |
| 30, 32, 33 | figure | Clip Studio workspace |
| 70 | text | Clip Studio workspace (Type) |
| 40, 42, 45 | operation / move | Clip Studio workspace |
| 41 | eyedropper | Clip Studio workspace |
| 60–65, 80–85, 90–93, 1–3, 100/110/120 | comic frame, correct line, ruler, view, other | kept, not runnable |

`0/0` (old or hand-made files without the columns) is treated as an unverified brush, as before.

`NodeIcon` tells apart sub-tools that share an operation: pen 101/103, pencil 102/113, brush
104/110/112, airbrush 105, eraser 106, blend 109/114, decoration 120–130, selection 200–205,
figure 210–218 (216 = lasso fill), fill 400/404–407, auto select 401, gradient 403,
eyedropper 300, operation 301/302/350, text 500. Values ≥ 10000 are user icons; placement then
falls back to the operation family.

The dot pen sub-tools carry no `BrushSize` even in CSP's own defaults, so Prism imports them
at 1 px with anti-aliasing off (inferred from the tool's purpose; the size source in CSP is
not traced).

Imported function sub-tools map their tool-property columns onto Prism's tool options
(inferred from the column names and the defaults, not traced): `FillReferMultiLayer` ≠ 0 →
sample all layers; `FillColorMargin` (percent) × 2.55 → tolerance; `FillContinuousPixel` →
contiguous; `FillAntiAlias` → anti-alias; `GradationShapeKind` 0 → linear, otherwise radial;
`GradationDither`, `GradationReverseSaved` → dither, reverse; `TextFontSize` → type size;
`TextVertical` → vertical type.

## Login redirects and an installed-app SUT bridge (2026-10-18)

The maintainer requested read-only REA investigation of CLIP STUDIO login,
account restriction risk, and using the installed app to obtain SUTs for Prism.
This extends the earlier SUT investigation to the launcher and its directly
related authentication/download paths. No account was signed into, no download
or purchase was requested, no session/cookie/login database was read, and no URL
handler or installed application was changed. This is static reverse engineering,
not a behavior-only investigation. No proprietary implementation is copied into
Prism. Product code was not changed.

### Evidence and limits

- Target: `C:\Program Files\CELSYS\CLIP STUDIO 1.5\CLIP STUDIO\CLIPStudio.exe`,
  version 5.0.4.0, 31,790,688 bytes; SHA-256
  `ddc382724fcc6b336911298b572b9d08da40e92635286136e339c4fa6624b4b8`.
  The independent local copy has the same hash.
- REA 6.4.0 `open_binary` succeeded for this launcher (unlike the earlier PAINT
  executable). `inspect_pe_resources` succeeded with evidence ID
  `ev_fd0b1d00c2bf9dc811916cfbbd87fd0b80c962aca14727e2063e98ad3dfe1d25`.
  Resource evidence proves inventory/identity, not authentication behavior.
- REA `xrefs` for the `materialdownload` literal timed out at its 90-second
  provider budget. Do not describe this as successful REA call-graph analysis.
- Direct Ghidra 12.1.4 focused decompilation of five candidate functions completed
  on the identical copy, with `-noanalysis -readOnly -deleteProject`; the import
  reported zero additional files loaded. This is separate from REA Evidence.
  Types and recovered control flow remain provisional; no live server behavior
  was verified. The first launcher attempt could not save Ghidra's global JDK
  preference; the successful invocation used the bundled Java directly and
  workspace-local settings/cache directories.
- Private evidence: ignored `target/rea-tools/csp-login/` contains REA requests and
  responses, `protocol-registration.json`, `static-strings.json`,
  `auth-strings.json`, `lea-candidates.json`, `focused.txt`, and logs. Raw binaries
  and decompiler output must remain ignored.

### What is established locally

Windows registers `clipstudio:` to the installed `CLIPStudio.exe -url "%1"`, and
`clipstudiopaint:` to `CLIPStudioPaint.exe -url "%1"`. The registration was read,
not invoked. This supports handing official links to their official owner; it
does not establish an authenticated callback to Prism.

Static inspection finds `materialdownload` at `0x1415e1c68`. Focused Ghidra
confirms references in `0x1400a80c0` (which also checks `clipstudio:`) and
`0x1400aa970`. This supports a native material-download dispatch path, but does
not establish a stable third-party URI parameter contract or a SUT output path.

The launcher also contains `/login/sessionhandover`, `/login/openurl`,
`/login/canceled`, `/login/closeview`, `login_sharing_code`, `auth_token`,
`DeviceTrustedToken`, and `X-CLIP-AUTH-TOKEN` literals. Ghidra confirms that
`0x140a934d0` looks up `login_sharing_code` among parameter entries and
`0x140a0f0a0` adds that named field to a request parameter structure before a
request-helper call. The header literal is initialized by `0x140036960`.
These are evidence of app-specific session sharing, not evidence that arbitrary
apps may reuse it. The code lifetime, server validation, accepted redirect
addresses, and anti-abuse rules are not established.

The inspected Prism engine/UI sources provide `brush.presets.importSut`, not
CLIP STUDIO authentication or ASSETS downloading. The importer supports local
SUT files/data with documented limits and fidelity warnings; it does not accept
an arbitrary downloaded material package as a SUT. Capability-restricted
MCP/control callers use bounded base64 `data`, not ambient filesystem `path`
reads (see [sut-brushes.md](sut-brushes.md)).

### Direct Prism sign-in and account risk

Opening the official site in the system browser is feasible. Returning an
account-authorized session to Prism is a different integration: a supported
third-party client registration/API and permission to download assets would
need to be established with CELSYS. No public registration/download API was
identified in the official sources reviewed. This is a search limitation, not
proof that no partner API exists. A website login redirect alone neither grants
Prism a token nor grants asset-download rights. For a future authorized OAuth
integration, [RFC 8252 section 8.4](https://www.rfc-editor.org/rfc/rfc8252#section-8.4)
requires registered native clients/redirects; follow external-browser and PKCE
practice. That standard is not evidence of CELSYS's particular implementation.

No defensible account-lock probability or failed-login threshold can be derived
from local binaries. The [CELSYS Terms of Service](https://www.celsys.com/en/information/celsys_terms/)
require the specified service usage methods, prohibit unauthorized access and
service disruption, and permit account suspension/termination for violations
or investigation of suspected unauthorized use. Accordingly, copying an app
session, impersonating its client, taking over its URI handler, or replaying
private requests should not be represented as a safe supported integration.
The reviewed sources do not say that simply opening an official page from Prism
causes a ban. Rate limits, authentication rejection, temporary security locks,
account suspension, and PAINT license activation restrictions are distinct.

### Feasible installed-app bridge

Proposed workflow (not implemented or verified end to end in this investigation):

1. Prism opens an official ASSETS page in the default browser.
2. The user signs in and chooses/downloads the material through the official
   website and CLIP STUDIO. Purchase confirmation stays in the official UI.
3. In PAINT, install the downloaded tool and export it as an individual `.sut`
   into a user-selected transfer folder. Download completion is not SUT export.
4. A native Rust Prism bridge can watch that folder, wait for a complete/stable
   file, apply existing size/format validation, deduplicate it, and call the
   existing importer. Import warnings and failures must be visible. Preserve
   the exported source and avoid silent library replacement on name collisions.

The [official ASSETS help](https://assets.clip-studio.com/en-us/help) requires
CLIP STUDIO for its download buttons. Downloaded materials appear in the app's
material library, as explained in the [save-location FAQ](https://support.clip-studio.com/en-us/faq/articles/20190005).
The [official tool manual](https://help.clip-studio.com/en-us/manual_en/150_tools/Importing_and_exporting_tools.htm)
documents exporting individual `.sut` files and separate `.sutg` groups; choose
individual SUTs for Prism's current importer. A usable PAINT installation with
export available is needed for this route; launcher installation alone is not
proof of that capability.

A semi-automatic bridge is technically credible and keeps credentials/download
requests inside the official client. Fully automatic download-to-export has no
verified public CLI/IPC contract here. UI automation could be researched, but
would require separate version-specific testing. Reading the internal material
store is another unverified format project; the app's library cannot simply be
assumed to be a folder of ready-to-import SUTs. The Windows evidence does not
establish macOS behavior, and this bridge requires a supported official app on
the host (no FreeBSD support was established).

### Material permission is separate from authentication

The [official usage FAQ](https://support.clip-studio.com/en-us/faq/articles/20190003)
allows exporting downloaded materials and prohibits giving them to other users.
However, the [ASSETS terms](https://assets.clip-studio.com/ko-kr/information/terms/detail)
(reviewed 2026-10-18; displayed revision August 19, 2026) limit import/use to
"specified software". The [Japanese terms](https://assets.clip-studio.com/ja-jp/information/terms/detail)
likewise say `指定のソフトウェア`. Export permission is therefore not sufficient
proof of permission to use all ASSETS brushes in Prism. Limited-license materials
have additional conditions. Official-app downloading does not remove that issue.

For product release, obtain CELSYS clarification/permission for this exact
third-party workflow and asset scope. A local import bridge can initially serve
self-authored brushes or independently supplied SUTs with rights permitting
Prism use; do not bundle or redistribute downloaded ASSETS materials. This is a
technical feasibility assessment with identified contractual uncertainty, not a
claim that CELSYS has approved Prism or that account restrictions are impossible.

Validation: installed/copy hash equality, successful REA resource/open responses,
Ghidra completion for five functions, registry reads, existing importer inspection,
and official documentation/terms cross-check. No runtime authentication test,
SUT export test, or Cargo tests were run because this change only records research.
The sibling `../craftrules/README.md` was unavailable in this checkout.

## Direct import from downloaded local materials (2026-10-18 follow-up)

The maintainer asked whether manual PAINT SUT export can be removed. **For the
observed local cache, yes: all ten downloaded brushes were reconstructed from
local databases and material files and accepted by the current Rust SUT parser.**
This is a private feasibility proof, not a shipped browser/import command or a
complete decoder for every downloaded `.tool` file. It supersedes the earlier
uncertainty about whether the local library has a usable export-free route.

### Scope and evidence

Only the local material catalog, tool settings/cache databases, referenced brush
materials, and installed PAINT executable were inspected. SQLite connections to
original databases used read-only mode and query-only operation. No login store,
cookie, token, download API, account request, application automation or export
command was used. Original files were not modified. Research outputs, including
asset-bearing reconstructed inputs, remain ignored under
`target/rea-tools/csp-local-materials/`; do not distribute or commit them.

The PAINT executable is still 5.0.4.0, SHA-256
`2e08e084c7f2256d4bba1d6a5bfcdd3498b8cd026667e1952e116c56c9ac9a2a`.
REA `inspect_pe_resources` succeeded again (the same deterministic evidence ID
`ev_b7218f56d145aa7dad019da09a7ff245516b2cf544aa9494dd342fc010e0ae12`).
`open_binary` still rejects this executable's overlapping PE header;
`inspect_binary_layout` reports its initial provider is Linux-x64-only. These
failures were retained, not counted as code-analysis success.

Direct Ghidra, separately from REA Evidence, completed five focused functions
using the unchanged installed binary, read-only/no-analysis/delete-project
options and workspace-local settings. The loader warned about overlapping
headers but successfully imported the program and reported zero additional
files loaded. Observations:

- `0x1402eec90` constructs a path using `EditImageToolDownloadTool.dtdb`.
- `0x142a49150`, `0x142a4a550`, and `0x142a4b7d0` access
  `DownloadToolVariantManager` and `ToolVariantNode` through database helper
  calls. The latter two also use the companion download-node records.
- `0x140110be0` initializes the `MaterialFile` table-name constant.

This corroborates that the observed download cache is an app-owned persisted
structure, not a guessed sidecar. It does not establish the complete cache
population/invalidation lifecycle or an export RPC. Raw decompilation is in
`focused.txt`; raw string hits and candidate references are identified separately
in `paint-strings.json` and `lea-candidates.json`.

### Local layout and identity mapping

The observed user data root is `%APPDATA%/CELSYSUserData/CELSYS` (Windows).
This is an observation, not a path to hard-code for every installation.

| Relative path | Observed role |
|---|---|
| `CLIPStudioCommon/MaterialDB/CatalogMaterial.cmdb` | SQLite material catalog; `MaterialModifier` joins `CatalogMetaData` through `_PW_ID`/`ModifierID` |
| `CLIPStudioCommon/Material/<bucket>/<bucket>/<catalog UUID>/<material UUID>/data/material_0.tool` | Downloaded brush payload, C2F-framed, not a plain SUT |
| `CLIPStudioPaintVer1_5_0/Tool/EditImageToolDownloadTool.dtdb` | Plain SQLite download tool cache, `ToolVariantNode` and `ToolVariant` |
| `CLIPStudioPaintVer1_5_0/Tool/EditImageTool.todb` | Plain SQLite current tool library, `Node` and `Variant` |
| `CLIPStudioCommon/Material/<bucket>/<bucket>/<tip UUID>/data/material_0.layer` | Referenced local tip image, accompanied by a thumbnail and material metadata |

The catalog has 422 metadata rows: ten `MaterialKind=brush, Downloaded=1` rows
and two downloaded catalog rows. All twelve have `DownloadNotYet=0` and
`NewDownloading=0`. A colon-separated catalog `Path` such as
`.:<bucket>:<bucket>:<catalog UUID>` resolves below `Material`; a brush's
`MaterialUuid` selects its child folder. The ten payloads all use C2F chunks
`HEAD`, `dATA` (5,130-byte encoded prefix), `dATA` (plaintext remainder), `TAIL`.
Finding an embedded SQLite signature does not establish a standalone SUT.
No complete C2F tool-prefix decoder was implemented.

The download cache holds eight matching `ToolVariantNode` rows and sixteen
`ToolVariant` rows (190 columns). The current tool library holds the other two
matching nodes and their variants (523 columns). `NodeVariantID` selects the
active row, avoiding a different/default variant. Join the catalog to those
nodes by `MaterialUuid == NodeMaterialUuid`, not by name or assumed equality of
content IDs: the observed catalog child `MaterialContentId` values differ from
the nodes' `NodeMaterialContentID` (which corresponds to their collection).
The ten recovered UUIDs exactly match the catalog's ten downloaded brush UUIDs.

`BrushPatternImageArray` and `TextureImage` use the already-observed bounded
reference framing. Their original paths resolve to separate local material
folders. Seventeen references across the ten brushes were resolved, including
one brush with six tips. Therefore, this route does not require decoding the
outer downloaded `.tool` container when complete usable cache records exist.

### Private proof and results

A temporary analysis script read each selected node and active variant, resolved
its material references, and created an independent SQLite SUT-shaped input
with `Node`, `Variant`, and `MaterialFile` tables. The material rows retain the
exact referenced path and contain a standard ustar package of the local original
`.layer` and its thumbnail. No proprietary implementation was copied. No
previous SUT export was used to build these ten inputs.

The temporary input is an implementation aid for this proof; a product adapter
could supply the equivalent data directly to a refactored Rust importer without
requiring a user-visible SUT export or permanent transfer file. Cache-based input
represents the cached/current tool settings, which may include user edits; it
must not be presented as necessarily identical to the original ASSETS download.

The current `prism-io` library was rebuilt with
`cargo build -p prism-io --profile fast --offline` (passed, 43.40 seconds; an
incremental-cache hard-link warning fell back to copying). A separate ignored
Rust harness linked that newly built library and called `sut::read_sut`:

| Check | Result |
|---|---|
| Reconstructed SQLite integrity checks | 10/10 `ok` |
| Actual current Prism parser acceptance | 10/10, one correctly named preset per input |
| Brushes without original-tip fallback | 6/10 |
| Brushes with explicit thumbnail fallback | 4/10; original raster layout unsupported by existing parser |
| Six-tip brush | All six tips recovered; ordering remains cyclic approximation |
| Previously exported `세모네모펜` reference | Full parsed `brush` value, including tip data, exactly equals local-cache result |
| Catalog and both source tool database hashes before/after | All three unchanged |

The comparison reference was the prior investigation's private `sut-0.sqlite`
(offset-zero copy of the old SUT export), SHA-256
`49f3e2c44e930baf1fb144b34d6f73893111d714d89b2a45ecef7b4db0447b45`.
It was used only after reconstruction to compare results. There was no new
manual or automated export. Equality here is of Prism's parsed brush data for
one existing reference, not a CSP pixel-render parity claim. Unmapped ribbon,
color-change, input control and display settings still produce existing warnings.

Evidence: `schema.json`, `downloaded-catalog.json`, `tool-files.json`,
`caches-and-chunks.json`, `proof-manifest.json`, `parse-results.json`,
`reference-parse.json`, and `verification.json`. Asset-bearing proof inputs and
serialized presets remain private in `proof/`. The parser harness does not
attach a preset store or change a running Prism session. No product source was
edited and no GUI feature was added. A full Cargo test suite was not run for
this research-only documentation update.

### Product recommendation and remaining gaps

Implement an explicit local-material import command: locate the selected CELSYS
material root, list completed downloaded brushes from a consistent read-only
catalog snapshot, resolve nodes/variants and all dependencies, then import the
selected brushes through the existing mappings. Retain origin UUID and source
revision/hash, deduplicate, show fidelity warnings and distinguish missing data
from unsupported material kinds. All application code should be Rust/native.
Read-only snapshot handling must account for SQLite WAL and concurrent app
writes; metadata flags alone do not guarantee a stable multi-file snapshot.
Canonicalize/junction-check references inside the permitted root and enforce
existing file, allocation, recursion and image limits. Never write to the CSP
catalog/tool databases.

The current evidence supports this PC's cached brushes. It does not prove that
a brand-new download has its tool cache and separate tips populated before
PAINT has processed it. Cache absence, stale cache, deleted/external references,
relocated user data, future schema versions and non-brush materials need explicit
handling. The initial feature should report missing cache data and request an
official-app refresh/import step where needed; do not claim that raw `.tool`
files alone are universally supported. A fully general cold-cache importer needs
additional C2F tool-container and dependency work. macOS was not tested.

The earlier material-permission caveat still applies: local technical access and
avoiding server requests do not establish permission to use every ASSETS material
in Prism. This proof needs no credentials, private API replay, URI interception,
or request impersonation; that removes those integration mechanisms from the
proposed local importer, but is not a guarantee about all account or policy risks.

### Product implementation (2026-10-18)

Implemented as `prism_io::sut::library` (list/import), engine commands
`brush.cspLibrary.list` / `brush.cspLibrary.import` and the Window › Extensions › CLIP STUDIO Materials panel.
It follows the recommendation above: one in-memory snapshot per call of the catalog and both
sub-tool databases read as bytes by the existing bounded Rust SQLite reader (extended for
UTF-16LE text, `INTEGER PRIMARY KEY` rowid aliases and rows written before columns were
added, which take their declared defaults); join by `MaterialUuid == NodeMaterialUuid`, the
download cache before the current library; material references resolved inside the canonical
material root (traversal and links out are rejected), packaged with their thumbnail and passed
to the unchanged SUT mapping. Missing caches, pending downloads, non-local materials and a
non-empty `-wal`/`-journal` are reported, not guessed; the `.tool` container is not decoded.
On this PC (2026-10-18): 10/10 listed as ready and imported; 6 original tips, 4 thumbnail
fallbacks (as in the proof); the three database files byte-identical afterwards (opt-in tests
`local_library_lists_and_imports_every_ready_brush`, `local_library_imports_ready_brushes_into_the_library`).
Still open: the cold-cache lifecycle for brand-new downloads, macOS paths, and the
material-permission question above.

Follow-up (2026-10-18, four new downloads): the catalog's `CatalogMetaData.MaterialKind` labels
each material (`brush`, `other_tool` for a fill sub-tool, `auto_action` for a `.laf`, `image`
for ruler/grid layers). A single-material download's `Path` is its own folder (no collection
child folder or collection row). Immediately after downloading through CLIP STUDIO, with PAINT
not started, neither tool database changed (mtimes Oct 16 / Jun 7), so the new brush and fill
tool have no cache and list as Not prepared; only `data/material_0.tool` exists. Whether PAINT
fills the download cache on start or only when the sub-tool is added is still to be observed.

## Direct `.tool` and `.laf` containers (2026-10-18)

The maintainer supplied three exact downloaded material paths. Read-only analysis used the
same installed PAINT 5.0.4 executable (SHA-256
`2e08e084c7f2256d4bba1d6a5bfcdd3498b8cd026667e1952e116c56c9ac9a2a`).
REA's PE-opening limitation documented above still applies. This follow-up used direct
Ghidra 12.1.4 and independent PE/RTTI/Capstone inspection; those observations are not
REA-generated Evidence records. Raw disassembly/decompilation, decoded files and temporary
analysis programs remain ignored in `target/rea-tools/csp-raw-import/`.

| Supplied material | Bytes | SHA-256 | Decoded contents |
|---|---:|---|---|
| NANAGOM PEN (`.tool`, internal name NANAGOM) | 114756 | `cdb5e090efaf32729e591000aa7c61aad6b5abdb039bbaf2d376f7ca12c9af7c` | 114688-byte material SQLite; 40960-byte nested tool SQLite |
| Enclose-and-fill tool (`.tool`) | 64580 | `487edb0f05dde6f426e8498d52a262c205ad166fecdc02be26e2afa1d45a88ae` | 64512-byte material SQLite; 14336-byte nested tool SQLite |
| Boundary cleanup (`.laf`) | 1034 | `0ef588e491a2201ebe905e601cdfd1cf66f74be09b7caa1c6dd370749ede639f` | 980-byte typed action-set value stream |

### Evidence and independent implementation

RTTI identifies `BlowfishWithCbcAndCtsDecrypting@C2F` and the related Crypto implementation.
The C2F decryption vtable is at `0x1447667c0`; direct Ghidra recovered its entry points,
including `0x14401f630`. Subsequent independent disassembly traced the material reader at
`0x143a111d0`, its C2F converter call at `0x14401dd10`, and the Crypto wrapper at
`0x144021e10`. The reader supplies a fixed material-format key, independent of the account.
The cipher implementation was not copied. Prism uses the MIT/Apache RustCrypto `blowfish`
crate, with independently written framing/CBC-CTS glue and bounds checks.

Observed framing: signature `89 43 32 46 0d 0a 1a 0a`; LE u32 length, four-byte tag,
payload, LE CRC32 of tag+payload. `HEAD`/`TAIL` are empty. `dATA` begins with LE u16
encoding: 0 is plain, 1 is an eight-byte IV followed by Blowfish CBC ciphertext with
ciphertext stealing. Blowfish block words use little-endian order (ordinary big-endian
Blowfish fails). The two tools encode their first 5120 bytes and leave the remainder plain;
the short LAF encodes its entire payload. Independent PyCryptodome verification recovered
valid SQLite headers and all four outer/inner SQLite integrity checks returned `ok`.
Synthetic vectors cover lengths 8, 9, 15, 16, 17 and 31, including CTS tails.

The outer tool database has `Material`, `ElemScheme`, `ParamScheme`, and `sqlite_sequence`.
Exactly one Material row supplies `MaterialToolVariantManager`, a complete nested SQLite
with `Node` and `Variant`. The importer follows that field rather than treating a raw
occurrence of the SQLite signature inside an overflow page as a standalone database.
The sampled tools have no external brush-tip or texture dependencies. NANAGOM uses
input/output 10/10 and brush size 25; the fill tool uses 31/51 and is not a paint brush.

The LAF value stream has a typed header and named fields. Observed values are UTF-8 strings,
unsigned integers with a byte-length prefix, and length-delimited arrays/objects (tags
0x48/0x49, BE u32 payload lengths). The reader bounds bytes, strings, entries and nesting,
checks exact frame consumption and rejects unknown types. The supplied set contains one
action with three enabled, noninteractive items:

1. `layeractionselectfromlayerstatusnew`, `referselected=0`: active-layer transparency.
2. `editactionclearoutofselectarea`: inverse selection, Clear, restore selection.
3. `selectactiondeselect`: deselect.

`actions.importLaf` compiles only this verified subset into Prism's existing commands,
installs the complete set atomically, and does not execute it. Unknown enabled steps,
parameters and interactive settings reject the whole import. Playback retains normal
command authorization and error handling. Disabled source steps are explicitly omitted.
This is command-level compatibility, not a CSP raster-parity claim.

**2026-10-18 Actions follow-up:** the initial adapter behavior above is superseded
by the common Photoshop-style action model. Set names, enabled flags, dialog flags
and source-step identities are now retained. Clear Outside Selection is one atomic
Prism command, so the three source steps remain three steps rather than five. Unknown
commands/parameters still reject the complete import. See [Actions](actions.md) and
the separate [Photoshop investigation](actions-analysis.md).

### Product behavior and limits

File Open and drag/drop accept `.tool` and `.laf`. The SUT command also accepts downloaded
C2F tool bytes. Local catalog import falls back to the original material file when no PAINT
cache exists, supporting collection-child and single-material folder layouts. Direct raw
tool import does not yet resolve external dependencies through the catalog; missing required
tips fail, and unsupported optional texture handling retains existing warnings.

(Superseded by [Enclose and fill](#enclose-and-fill-2026-10-18): output 51 now runs.) The fill tool's mapped metadata is preserved with an explicit enclose-and-fill warning. Output 51
remains disabled by the existing Sub Tool capability map: parsing it is not implementation
of CSP's enclosure/close-gap/expand/color-reference fill behavior. The brush and boundary
cleanup action have executable mappings. No credentials, login/session databases, URI
registrations or server requests are used. No source material is redistributed.

### Local validation

The supplied two tools and LAF pass the opt-in readers directly from their original paths;
the original bytes are unchanged. The local catalog now lists 12 tools with readable sources:
the existing 10 cache-backed tools plus these two `materialFile` entries. All 12 import,
with existing fidelity warnings, and all three source databases remain byte-identical.
This counts successful parsing, not 12 fully implemented tool behaviors.

The synthetic IO suite passes 157 tests (6 private-oracle tests ignored by default).
Engine regression coverage includes cleanup at 8/16/32-bit depths, atomic rejection of
unsupported action steps, and preserving mapped non-brush metadata through the preset store.
`local_materials_import_paint_execute_and_persist` accepts `PRISM_TEST_CSP_MATERIALS`, a JSON
array of the two tool paths and the LAF path, to check actual imports, pen drawing, action
playback, distinct default groups, persistence and unchanged original bytes.

That full local engine test passes. The engine library passes 1010 tests (15 ignored),
and the adversarial command `panic_hunt` passes. Native offscreen snapshots verify the
imported NANAGOM strokes and the Japanese-named LAF entry in the Actions panel; captures
remain in the ignored analysis folder. The desktop `fast` build succeeds. Layers remain
valid (29 crates) and generated menu wiring remains 628/628; neither figure claims tool
behavior parity.

Across 98 compiled integration executables, the two GPU preview suites and MCP suite
initially failed under sandbox GPU/socket restrictions, then passed when rerun outside
those restrictions (GPU tests sequential). The application and automation library socket
tests also pass there. The UI library has 1435 passes, 13 ignored tests, and the three
previously documented Windows `save_ui` path failures. Strict workspace Clippy stops at
the pre-existing `prism-text/src/cjk.rs` nonminimal boolean expression. These are separate
from direct material decoding and the passing original-file integration test.
All-target `--no-deps` Clippy completes with existing engine/UI/example warnings and no
diagnostics in the material-reader/import changes. The original-file tests and IO corpus
checks use the optimized `fast` profile; they do not claim release-LTO validation.
The complete IO `corpus,heif` library/integration run passes, including the PSD mutation,
smart-object, TIFF and vector corpora. Final SHA-256 comparison again confirms all three
supplied material files are unchanged.

## Enclose and fill (2026-10-18)

Output operation 51 (input 31, for example 隙間無く囲って塗るツール) now runs in the Clip
Studio workspace. Evidence was limited to read-only inspection of the installed CLIP STUDIO
PAINT: the shipped tool database `Settings/PAINT/Tool/english/UXEditImageTool.todb` (SQLite,
`Node` joined with `Variant` on `NodeVariantID`) and the column-name string table in
`CLIPStudioPaint.exe`, plus the supplied enclose-and-fill `.tool` decoded earlier. No code was
disassembled or decompiled for this; nothing from CSP is copied.

### Observed settings

| Column | Default `Enclose and fill` (todb) | Supplied tool | Prism |
|---|---|---|---|
| `FillReferMultiLayer` | 1 | 1 | sample all layers |
| `FillColorMargin` | 30 | 20 | colour margin, percent × 2.55 → tolerance |
| `FillAntiAlias` | 1 | 1 | anti-alias |
| `FillUseCloseGap`, `FillCloseGap`, `FillCloseGapUnit` | 0, 50.0, 0 | 1, 5.0, 0 | close gap in pixels when enabled |
| `FillUseExpand`, `FillExpandLength`, `FillExpandLengthUnit`, `FillExpandType` | 1, 10.0, 0, 2 | 1, −1.0, 0, 2 | area scaling in pixels when enabled; only a negative value changes the fill (see below) |
| `FillSoakIntoNarrowArea` | absent | 1 | always on (soak back after close gap) |
| `CloseFillReferColorType` | 7 | 9 | target colour (list index, see below); not mapped |
| `FillNotRefer*`, `FillReferFrame`, `IncludeFill*` | various | various | not mapped |

Inferred, not traced: the length unit 0 is pixels (CSP's brush sizes in the same table read as
pixels). Observed in the CSP 1.5 Tool Property UI (2026-10-19): `CloseFillReferColorType` is
the index of the *Target color* list, not a bit set: 0 all colours, 1 only transparent, 2 area
surrounded by transparent, 3 only black, 4 area surrounded by black, 5 only white and
transparent, 6 area surrounded by white and transparent, 7 treat semi-transparent as
transparent (the default), 8 all enclosed areas except transparency, 9 all enclosed areas
including transparency. Prism implements the behaviour measured for 7 and 9 (below) whatever
the value. Paper, draft, text and locked-layer exclusions have no Prism counterpart
(Prism has no paper layer); the reference is the active layer or the visible composite.

### Prism behaviour (independent implementation)

`prism_algo::enclose_fill` and the command `paint.encloseFill` define it as follows:

1. Line art is any pixel whose ink, `alpha × (1 − luminance)` in 0–255, exceeds the colour
   margin; everything else (transparent, white, light colours) is open.
2. With close gap *g*, open pixels whose centre lies within (*g* + 1) / 2 of line art don't
   connect areas.
3. The outer space is every open, 4-connected area inside the lasso that touches a pixel
   outside it, soaked back into the pixels step 2 held back next to it (no further than that
   radius, paths never crossing line art), so it stops inside a sealed gap. The canvas edge
   inside the lasso closes areas (the lasso may extend past it).
4. Everything else inside the lasso is filled: the enclosed areas *and the line art*, so the
   colour runs under the lines. A line only partly inside the lasso is filled up to the lasso;
   a stroke that encloses nothing is filled too.
5. A negative area scaling shrinks the fill away from the outer space (not from the lasso or the
   canvas edge). A positive one changes nothing: there is no line art left to grow into, and the
   fill never enters the outer space. Nothing leaves the lasso.
6. The result is filled with the foreground colour like the Paint Bucket: one history step,
   written only to the target, clipped by the selection; no step when nothing is filled.

The lasso is the Lasso tool's outline (freehand, Alt for straight segments). In the Clip
Studio workspace the sub-tool selects the Lasso; closing the outline runs the fill instead of
selecting, and a press inside an existing selection draws rather than moving it.

Known differences: CSP's other target colours (0–6, 8), vector-centre reference, paper
exclusion and exact anti-aliasing are not reproduced.

### A/B against CLIP STUDIO PAINT (2026-10-19)

Up to this date Prism filled only the enclosed open areas (and grew into the lines by the area
scaling). A user's 隙間無く囲って塗るツール (gap 5, area scaling −1) then left a transparent ring
between the fill and the lines. Measured by driving CSP 1.5 on the same synthetic line art (two
18 px anti-aliased loops, one with a 4 px gap; an open C and a loop cut by the lasso) and
exporting the fill layer, then running `paint.encloseFill` with the same lasso and settings:

| Case | Prism before (IoU, alpha ≥ 128) | Prism after |
|---|---:|---:|
| 隙間無く囲って塗るツール (target 9, gap 5, −1) on the loops | 0.723 | 0.994 |
| Same tool, open C plus a loop cut by the lasso | — | 0.987 |
| Default 囲って塗る (target 7, margin 30 %, +10) on the open C and loop | — | 0.9999 |

CSP filled the lines under the colour in every case, including the open C, which encloses
nothing; with −1 it stopped one pixel short of the lines' outer edge and stayed flush with the
lasso cut; +10 never reached open pixels outside the lines. The remaining differences are edge
anti-aliasing (CSP leaves the default tool's edge hard) and the hand-drawn lasso position. The
test images and exports stay local (`target/ab-enclose/`).

## Every downloaded material, end to end (2026-10-19)

The maintainer asked for every CLIP STUDIO material in the local library to work. The opt-in
test `crates/engine/tests/csp_library_local.rs` (`every_local_csp_material_imports_and_paints`)
imports each of the 153 materials (151 brushes, 2 sub-tools) on its own, paints a pressure
stroke with every brush and writes contact sheets and a JSON report to `target/csp-check`.
`crates/io/src/sut/material_tests.rs` › `local_tool_materials_decode` decodes every tip and
texture package of every downloaded `.tool`. Evidence: read-only Python inspection of the
local material files (decoded copies under the scratch folder, nothing written to the CLIP
STUDIO folders); no executable was disassembled for this section.

Before: 134 of 153 failed to import (`SUT table MaterialFile is missing`), 107 tips fell back
to their thumbnails and 15 brushes painted nothing. After: all 153 import, all 302 tip packages
decode from their original rasters, every brush paints.

### Observed and fixed

| Finding | Evidence | Prism |
|---|---|---|
| Downloaded `.tool` containers keep tip and texture packages in the outer database's `ToolPatternInMaterial` (`RelativePath`, `FileData` ustar package), not in a `MaterialFile` table of the sub-tool database | 4 tips of Evergreens; references in `BrushPatternImageArray` equal `RelativePath` | `read_sut` falls back to that table, mapped to `MaterialFile` rows |
| The C2F cipher swaps the last two ciphertext blocks also when the encoded length is a whole number of blocks (CBC-CS3) | 45 of 302 `.layer` databases had a garbled tail on page 5 (`sqlite_sequence` schema) with plain CBC; 302 of 302 open with the swap | `csp_container::decrypt` undoes the swap; the 16-byte test vector is now in CS3 order |
| `.layer` files decode completely with the container cipher | as above | the material reader decodes the whole layer database first, the partial plaintext recovery stays as fallback |
| Colour layers: tile layout `[33, 1, 4, 5, 65536, 4, 1024, …]`, an alpha plane then four bytes a pixel | 28 tips (`LayerColorTypeIndex` 0); blue-green-red byte order inferred (Windows DIB order), not traced | read as Rec. 601 luminance and flagged as colour |
| Tips up to 14418 × 1244, 5703² and 706 × 12001 pixels | sizes from the tile headers | sources up to 16384 px a side and 64 M pixels, area-averaged while decoding to 4 M pixels a tip (less when a sequence would exceed 16 M) |
| Layer opacity below 256 | one tip at 225 | alpha × opacity / 256 |
| Tiles may cover only the stored extent (`width − 1`): a material stores 2048 with 8 tile columns | Rock Edge / Layer / Crack Painter | the uncovered last column is transparent |
| Effector control `0x80` is random with its minimum in word 24 (already traced, see Effector tilt response) | 57 size, 53 flow, 50 spray-size effectors | `Dynamic.cspRandom`: a per-dab factor between the minimum and 1, multiplied with pressure and tilt (uniform distribution inferred) |
| `BrushSpraySizeSyncBrushSize`: the particle size keeps its ratio to the brush size | Sparkle-Lei 500 → 100, crystal sparkles 150 → 47.407…, an odd value consistent with proportional resizing | `CspSpray.sizeRatio`, resolved against the current brush size before rendering |

### Colour brush tips (inferred)

The 15 CHERU hair-highlight tips are colour layers drawn in pure white with
`BrushChangePatternColor` = 0. Read as darkness they are empty, yet the brushes are sold to
paint highlights, so CLIP STUDIO must draw colour tips in their own colours unless *Change brush
tip colour* is on (inference, not traced). Prism's tips are density masks painted in the brush
colour, so a colour tip now uses its alpha as the shape, and the import warns
(`COLOUR_TIP`) when CLIP STUDIO would keep the tip's colours. Gray tips keep
`alpha × (1 − gray)`. With *Change brush tip colour* on, Prism already does what CLIP STUDIO does,
so that setting no longer warns.

### Still open (measured on the local library)

- ~~`BrushRibbon` (51 brushes: highlights, bushes, braids, chains)~~: implemented on 2026-10-19,
  see Ribbon.
- ~~Rotation flag `0x40` (39 brushes)~~, ~~tip order types 1–3 (33 brushes)~~ and ~~colour tips
  (28 brushes)~~: implemented on 2026-10-19, see the next section.
- Smaller items: particle angle input controls (6), colour mixing type 2 (5), dual brush (3),
  spray size unit 2 (3), speed control `0x40` (1), fade start/end (1).

## Tip order, rotation input and colour tips (2026-10-19)

Same CLIP STUDIO PAINT 5.0.4.0 executable (SHA-256 above). Authorized read-only direct Ghidra
decompilation (`-readOnly -noanalysis`, a copy of the `Persist` project in the session's scratch
folder) plus string inspection of the installed language resources
(`resource/<language>/742DEA58-ED6B-4402-BC11-20DFC6D08040`) and the shipped default sub-tools
(`Settings/PAINT/Tool/english/EditImageTool.todb`, read through a copy). These are not REA
Evidence records; no decompiled code is copied into Prism.

### Tip order (`BrushPatternOrderType`, property 0x42f)

Evidence:

- Reader `0x142b79920` stores `BrushPatternOrderType2` into property 0x42f when that key is
  present, else `BrushPatternOrderType`.
- Builder `0x142b80a20` reads 0x42f with the tip array (0x42e) and the flip settings (0x432,
  0x433) and passes them to `0x142e64720` → `0x1424aafa0` → `0x142573f90`, which stores the
  order at tip-source `+0x78` and the flip bits at `+0x7c`.
- The per-dab selector `0x1425744a0` (called from the dab step `0x1422e0db0` through tip source
  `+0x298`) takes the stroke's LCG state and a dab counter, with `c` tips: order 0 → `n mod c`;
  1 → `k = n mod (2c − 2)`, mirrored to `2c − 2 − k` from `k ≥ c − 1` (it also reports the
  return half for flip mode "on reversed strokes"); 2 → `min(n, c − 1)`; 3 → `⌊r·c + 10⁻⁸⌋` with
  `r` = LCG bits 16..30 × 2⁻¹⁵ (constants `0x144349010` = 1e-8, `0x1444ea8a8` = 2⁻¹⁵);
  4 → `n` while `n < c`, then no tip (the dab is skipped); 5 → an explicit per-stroke index list.
  The counter advances once per dab. A single tip ignores the order (except 4/5 after the
  first dab).
- The English resource labels the choice *Set how brush tip shapes repeat*: Repeat, Reverse,
  Do not repeat, Random, matching codes 0–3. Codes 4 and 5 have no label and are not stored by
  any local brush.

Prism (`CspTipOrder`, `BrushSettings.cspTipOrder`): codes 0–3 as above; other codes repeat with a
warning. Prism draws the random index from its own counter-based stream (`stream::TIP`), so
individual random picks differ from CSP's, the distribution does not.

### Tip rotation input (`BrushRotationEffector` 0x10 / 0x20 / 0x40)

Evidence:

- The dab step `0x1422e0db0` adds sample `+0x28` when 0x10 or 0x20 is set, else sample `+0x48`
  when 0x40 is set (see Tip rotation controls). The pen sample layout follows from the effector
  evaluator `0x1425717f0`: `+0x10` pressure (control 0x10), `+0x18` tilt (0x20), `+0x20` speed
  (0x40), `+0x30` the input of control 0x100.
- The vector-stroke input request `0x1422e9ff0` (read by `0x142e85450`) asks for an extra per-point
  input when rotation flags `& 0x30` are set (or the spray's), next to the pressure, tilt, speed
  and fade inputs the effectors need. It asks for nothing for 0x40: that angle can be derived
  from the path.
- The resources list the direction choices in this order: None, *Direction of pen* (Japanese
  ペンの向き, "faces the direction the tablet pen is tilted, when it can be detected"), *Pen axis
  rotation* (ペン軸の回転, "follows the twist of the pen around its axis, when detected"),
  *Direction of line* (線の方向), *Direction of whole spray*, *Spray toward center*. The spray
  particle angle code (`0x1422e8d70`, and the particle step in `spray-pass4.txt`) tests 0x100 (an angle passed in for the
  whole spray), 0x40 (sample `+0x48`) and 0x200 (`0x141a94240`: the angle from the spray centre
  to the particle, i.e. toward the centre), so the choices map to 0x10, 0x20, 0x40, 0x100 and
  0x200 in order.
- Shipped defaults agree: Pencil, Mechanical pencil, Charcoal, Chalk, Gouache, Oil paint store 19
  or 147 (0x10); Flat marker, Flat watercolor brush, Thick oil paint, Brush pen store 35 (0x20);
  Crayon, Kneaded eraser, Cherry, Bloodstain store 67 or 195 (0x40).

Inference: 0x10 is the pen's tilt direction, 0x20 its barrel rotation, 0x40 the line direction.
Both pen angles read the same sample field; which one CSP writes there when both bits are set is
not traced (no brush stores 0x30).

Prism: 0x10 → `Control::PenTilt` (the tilt azimuth), 0x20 → `Control::Rotation` (barrel
rotation), 0x40 → `Control::Direction`, with the random turn unchanged. This replaces the earlier
mapping of 0x10/0x20 to the line direction: Gouache and the other 0x10 brushes no longer turn
with a mouse, as in CSP, where a mouse has no tilt direction. Spray particle angle controls are
unchanged (still warned).

### Colour tips

Evidence: property 0x44e is `BrushChangePatternColor` (reader `0x142b79920`); the builder sets dab
flag 0x20000 and reads 0x44f/0x450 with it. That a colour tip without it keeps its own colours
remains an inference from the 15 white CHERU highlight tips (see Colour brush tips).

Prism: the material decoder keeps the colour layer's straight RGB (blue-green-red bytes,
alpha-weighted when the material is reduced); `BrushSettings.cspTipColors` holds it per tip
(8-bit RGB, base64 in JSON). The renderer samples premultiplied colour planes exactly like the
tip's shape (same level, transform, anti-aliasing) and lays the colour per pixel into the stroke
buffer, so Normal and other blend modes, opacity, flow and textures work as for a coloured dab.
Ribbon slices sample the planes trimmed like their tip. Colour mixing holds one colour a dab, so
there a colour tip mixes its alpha-weighted mean colour; `COLOUR_TIP` warns only when a tip has
several colours (none of the local brushes: the 15 CHERU highlight brushes, which mix and lay
ribbons, have pure white tips). The preset store writes the planes as 8-bit tip files
(multi-tip sequences now go to tip files too, so large CSP brushes stay under the 16 MB group
file limit). Not reproduced: erasing ignores colours, Dual Brush tips are alpha only, a Pencil
(aliased) ribbon lays the mean colour, and the tip thumbnails stay grayscale.

### Measured on the local library

`csp_library_local` (153 materials, 151 brushes): 0 import errors, 0 empty strokes; brushes with
warnings 33 → 18. The tip order (33), rotation by pen input (39) and colour tip (28) warnings are
gone; what remains is listed under Still open (particle angle input 6, colour mixing type 2 5,
dual brush 3, spray size unit 3, one each of water edge unit, emphasize density, speed control,
fade start/end). Sheets: 茂み1–3 and 茂み下 paint their greens, the 木目 grain brushes their
grays; the highlight brushes paint white (checked on a black layer). This is a check that the
settings are carried through, not a pixel comparison with CLIP STUDIO.

## Ribbon (2026-10-19)

Same CLIP STUDIO PAINT 5.0.4.0 executable (SHA-256 above). Authorized read-only direct Ghidra
decompilation of the `Persist` project (`-readOnly -noanalysis`, script `PrismFocused.java`) plus a
raw byte scan for `test byte [reg+0x78], 0x20`. Outputs under `target/rea-tools/csp`:
`builder-ribbon.c` (the settings builder, cut from `texture-pass5.txt`), `rb1.txt`…`rb5.txt`
(SHA-256 prefixes f35eec17, f25175ca, 035af6cd, c9e1d191, 9d793d76), `ribbon-scan.py`,
`fstart.py` (function bounds from `.pdata`), `rdd.py` (constants). Not REA Evidence records;
no decompiled code is copied into Prism.

### Evidence

- `BrushRibbon` is property 0x456 (id 1110, `display-properties.json`). The builder
  `0x142b80a20` reads it only when spraying (0x46a) is off and a tip image was loaded (0x431 with
  a non-empty 0x42e), and then sets configuration flag 0x20 at `+0x78`. With that flag it resets
  colour mixing type 1 (`+0x35c`) to 0.
- Seven instructions test the flag. `0x142566130` sends a stroke to `0x142562240` with it (the
  dab path `0x142567790` without it); `0x1422e48a0` skips the spacing-based quality estimate;
  `0x1425ad530`/`0x1425adb00` skip the per-dab tile loops.
- `0x142562240` runs once per step and keeps the previous step's state (`+0x180`: position, size,
  angle, …). Its geometry step `0x1422e1410` takes the radius `r` (pressure-scaled size, at least
  0.1) and the thickness: without vertical thickness (`+0x78` flag 0x40) the width is `r·t`,
  with it the along-axis scale is `t`. It snaps the tip rotation `+0x268` (`BrushRotation`,
  read whatever the rotation effector flags; +180° on one tip-selector output) to a quadrant:
  < 45° → 0, < 135° → 1, < 225° → 2, < 315° → 3, else 0 (constants 45/135/225/315/180/360).
  For quadrants 0 and 2 the tip's height (`+0x64`) runs along the stroke and its width (`+0x60`)
  across, for 1 and 3 the reverse.
- It advances the tip position by `4·cross / (2r·t_along)` texels for a step of 4 document pixels
  (constant `0x144360e48` = 4.0). Where that passes the tip's end it shortens the step to the
  rest of the tip and flags the next step to fetch the next tip from the tip selector
  `0x1425744a0` (the same one the dab path uses), starting at its first row. Without a tip the
  step is 10⁶.
- The quad job `0x142618d40` → `0x142617380` → `0x142640510` builds the slice from the previous
  step's centre and half-width (corners `P₀ ∓ n₀·w₀`) to this step's (`P₁ ± n₁·w₁`), resamples the
  tip so its cross axis spans the width (`0x1425261e0` picks the axis by quadrant) and maps the
  tip rows `[u₀, u₁]` (×0x8000 fixed point) onto it.

Inference: the cross directions are the stroke normals at each step; quadrant 1 shows the tip
upright on a stroke drawn to the right. Checked against the 50 local ribbon brushes: every
`BrushRotation` ≈ 90° brush (bushes 2709 × 1345, rocks 2049 × 440, wood grain, runes, the CHERU
highlights at 100°) has a wide tip and every 0° one (lightning 1036 × 3008, braids 496 × 8443) a
tall tip, so the long side always runs along the stroke. The sign of each axis (which side the
tip's first column lies on) is not traced; Prism rotates the tip by quarter turns, never mirroring
it.

### Prism (independent implementation)

`BrushSettings.cspRibbon` (`CspRibbon { quarterTurns }`, `crates/paint/src/csp_ribbon.rs`):

- Import: `BrushRibbon` with tip images and without spraying, quarter turns from the raw
  `BrushRotation` (read whatever the rotation flags, though no local ribbon brush depends on it);
  ribbon brushes no longer warn and colour
  mixing type 1 is treated as 0. 50 of the 51 flagged local brushes qualify (RushLines# 3 sprays).
- Dynamics: in ribbon mode `DabBuilder` steps every 4 px (shorter at a tip's end), ignoring
  spacing, scatter and angle; each step emits one dab carrying a `RibbonSlice` (both
  cross-sections, the tip rows, the tip index) with the step's size, flow, opacity, colour and
  texture depth. A sequence moves to its next tip (cyclically) when one runs out.
- Render: `BrushContext` locates each pixel centre in the slice by inverting its bilinear map
  (half-open along the path, so neighbouring slices share their edge without overlap or gap),
  samples the tip with bilinear mip filtering (nearest for anti-aliasing off) and applies noise,
  per-tip texture and flow like a dab. The along axis clamps to the tip's end texels so copies
  join; one fully transparent last line along the ribbon (the column CSP materials leave
  uncovered when their tiles stop short of the stored width) is trimmed, since an odd mip chain
  would weight it like content and print a seam at every join.
- Not reproduced: CSP's tip order types for the next tip (cyclic), dual brush on the ribbon, the +180° selector case, fixed-point sampling; no
  matched CSP stroke was compared, so this is behavioural, not pixel, parity. Colour tips on
  ribbons: see Tip order, rotation input and colour tips.

