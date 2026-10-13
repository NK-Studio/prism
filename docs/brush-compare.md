# Brush comparison against Photoshop

`cargo xtask brush-compare` measures the brush engine against Photoshop. Both apps render the
same strokes, and the tool reports where the results differ. It is a local measuring tool, not a
CI test: Photoshop renders are observation oracles (AGENTS.md, clean-room) and stay in
`target/brush-compare/`, which is never committed.

## Workflow

```sh
cargo xtask brush-compare jsx      # writes target/brush-compare/photoshop.jsx
# Photoshop: File › Scripts › Browse… › photoshop.jsx (renders into target/brush-compare/ps/)
cargo xtask brush-compare all      # Prism renders into prism/, then the diff
```

Open `target/brush-compare/report.md`. Each case gets:

- **ink PS / ink Prism / ratio:** total coverage. The ratio is the quickest check on flow
  build-up, spacing and dab count.
- **MAE, max, differing:** per-pixel difference over the inked pixels.
- **peak, FWHM, Δcentre:** a 1-D profile along the case's probe line. FWHM is the width at
  half the peak. It separates falloff and edge width from overall darkness. Δcentre shows
  half-pixel coordinate offsets.
- `diff/<id>.png`: red where Prism lays more ink, blue where Photoshop does (×4).
- `profiles/<id>.csv`: the two profiles, for plotting.
- Photoshop's tool options read back after the brush was set. Any switched-on flag (`on: …`)
  means a dynamics section or option was active. That render is not comparable until it is
  turned off.

Options: `--out DIR`, `--cases FILE`, `--only id1,id2`. After changing the engine, run
`cargo xtask brush-compare all` again. The Photoshop renders don't need regenerating unless
the cases change.

## Cases

The cases are in `crates/engine/examples/brush_compare/cases.json`. They use Photoshop's panel
units: whole pixels, degrees and whole percentages. Fields: `size`, `hardness`, `spacing`,
`angle`, `roundness`, `opacity`, `flow`, `tool` (`brush` | `pencil`), `points` (path anchors in
pixels, at least two), `canvas` and `probe` (`{"row": y}` or `{"column": x}`). Values Photoshop
can't represent are rejected.

Photoshop renders each case with **Stroke Path** (no simulated pressure) on a new 8-bit sRGB
document at 72 ppi, in black on white. Prism uses `paint.stroke` / `paint.pencil` with
pressure 1 and no smoothing. Pen pressure, tilt and other per-point input can't be scripted in
Photoshop, so the cases cover deterministic settings only. Jittered dynamics need statistical
cases, which the tool doesn't support yet.

## Limits

- The script was verified with Photoshop 2026. Other versions may name tool options
  differently. The report lists any case whose read-back doesn't match (a section left on,
  or a value that didn't apply). Re-run the script before trusting those numbers.
- Photoshop keeps Brush Settings sections in the tool state, and Reset Tool doesn't clear
  them. The script turns every section off explicitly. Afterwards it restores the Brush and
  Pencil options and the selected tool.
