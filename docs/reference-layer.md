# Reference layers

CLIP STUDIO lets a layer (usually the line art) be set as a **reference layer**. Tools with an
"Anti-overflow" option then use its lines as borders. Prism has the layer flag and the brush
option "Do not cross lines of reference layer" (`BrushUseReferLayer`, property 1250). The
fill tool's "Refer to other layers" choices are still approximated by all layers
(`csp_tool_property.rs`).

## Model

- `prism_doc::Layer::reference: bool`. Any layer kind can carry it; only layers with pixels
  (`Layer::surface()`) give lines. It is layer data: setting it is a history step that changes
  no pixels, and `layer_pixels_equal` ignores it.
- `.prism`: `LayerManifest::reference` (`#[serde(default)]`, written only when set), so older
  files load with the flag off. Test: `crates/format/tests/reference_layer.rs`.
- PSD has no block for it: the flag is not written to PSD and is lost there.

## Command

`layer.setReference {"layer": id? = active, "on": bool? = toggle}` → `{layer, reference}`.
Layers panel right-click menu: **Set as Reference Layer** / **Clear Reference Layer** (one layer).
The Layers panel does not mark reference layers yet; the menu label shows the state.

## Brush behaviour

A brush with `BrushSettings::csp_refer_layer` paints through a narrower selection
(`prism_engine::reference_layer::constrain`, called from `brush_cmds::stroke_with`):

1. The reference lines are the pixels of every visible reference layer other than the layer
   being painted whose alpha is above 0.5 (a layer without alpha: darker than 0.5). Only the
   stroke's reach is read: the path's bounds grown by the brush size, inside the canvas.
2. Each pixel the path passes (about one a pixel of travel) seeds a flood fill that stops at
   lines. The filled areas are where the stroke may paint, so crossing a line with the pen
   paints on both sides, and a brush wider than a gap does not leak through it.
3. Line pixels touching a filled area are also paintable, so colour meets the line with no
   gap. The existing selection still applies.

No reference layer, a hidden one, or no line in reach: the stroke is not constrained.

Limits (never a crash): a stroke whose reach is over 64 million pixels is not constrained;
non-finite points are skipped.

## Not done

- The live stroke is drawn unconstrained; the committed stroke is clipped at pen-up.
- CLIP STUDIO's siblings of the option are not read: Fill up to vector path, Tolerance, Area
  scaling and Scaling mode. Prism's line threshold (0.5) and the one-pixel overlap onto the
  line are its own choices, not compared with CLIP STUDIO's pixels.
- No reference-layer mark in the Layers panel; no PSD round trip.
