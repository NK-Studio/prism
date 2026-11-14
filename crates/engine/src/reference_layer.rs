//! CLIP STUDIO reference layers: `layer.setReference` marks a layer as one, and a brush imported
//! with "Do not cross lines of reference layer" (`BrushSettings::csp_refer_layer`) paints only
//! in the areas its path enters, bounded by the lines of the reference layers. The flag is layer
//! data, so setting it is a history step that changes no pixels. See `docs/reference-layer.md`.

use prism_doc::{Document, LayerId};
use prism_geom::Rect;
use prism_paint::{BrushSettings, StrokePoint};
use prism_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const SET: &str = "layer.setReference";
/// A reference pixel at least this opaque (or, without alpha, this dark) is a line.
const LINE: f32 = 0.5;
/// The stroke area the constraint is computed for; a larger stroke is left unconstrained.
const MAX_AREA: i64 = 64_000_000;
/// Path samples walked for seeds.
const MAX_SEEDS: usize = 2_000_000;

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: SET.into(), msg: msg.into() }
}

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    st.active_layer.filter(|id| st.doc.layer(*id).is_some()).map(|_| ()).ok_or_else(|| "no active layer".into())
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = match p.get("layer") {
        None | Some(Value::Null) => st.active_layer.ok_or_else(|| bad("no active layer"))?,
        Some(v) => LayerId(v.as_u64().ok_or_else(|| bad("`layer` must be a layer id"))?),
    };
    let now = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?.reference;
    let on = match p.get("on") {
        None | Some(Value::Null) => !now,
        Some(v) => v.as_bool().ok_or_else(|| bad("`on` must be true or false"))?,
    };
    if on != now {
        s.edit(if on { "Set as Reference Layer" } else { "Clear Reference Layer" }, |doc, _| {
            doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.reference = on;
            Ok(())
        })?;
    }
    Ok(json!({ "layer": id.0, "reference": on }))
}

/// Reference-layer command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: SET,
        label: "Set as Reference Layer",
        menu: &[],
        shortcut: None,
        params: r#"{"layer":id?=active,"on":bool?=toggle} → {layer, reference}. Brushes with "Do not cross lines of reference layer" stop at the lines of reference layers"#,
        enabled: has_layer,
        run: set,
        journal: true,
    }]
}

/// The selection a stroke of `brush` along `pts` on `target` paints through: `selection`
/// itself unless the brush stops at reference-layer lines and the document has a visible
/// reference layer other than the target. Then it is `selection` limited to the areas the path
/// enters (flood-filled from every path sample, stopping at reference lines) plus the line
/// pixels touching them, so paint meets the line without leaving a gap.
pub fn constrain(doc: &Document, target: Option<LayerId>, brush: &BrushSettings, pts: &[StrokePoint], selection: Option<Surface>) -> Option<Surface> {
    if !brush.csp_refer_layer {
        return selection;
    }
    let refs: Vec<&Surface> =
        doc.walk().into_iter().filter(|(_, _, l)| l.reference && l.visible && Some(l.id) != target).filter_map(|(_, _, l)| l.surface()).collect();
    let Some(area) = stroke_area(doc, brush, pts).filter(|_| !refs.is_empty()) else { return selection };
    let (w, h) = (area.width() as usize, area.height() as usize);
    let Some(n) = w.checked_mul(h) else { return selection };
    // 1 = a reference line.
    let mut cell = vec![0u8; n];
    for surf in refs {
        let (c, alpha) = (surf.channels(), surf.format().alpha);
        if c == 0 || surf.content_bounds().intersect(&area).is_empty() {
            continue;
        }
        let px = surf.read_region(area);
        for (i, v) in px.chunks_exact(c).enumerate() {
            let line = if alpha { v.last().is_some_and(|a| *a > LINE) } else { v.first().is_some_and(|d| *d < LINE) };
            if let (true, Some(m)) = (line, cell.get_mut(i)) {
                *m = 1;
            }
        }
    }
    if !cell.contains(&1) {
        return selection;
    }
    // 2 = an area the path enters.
    let mut stack = Vec::new();
    for (x, y) in seeds(pts) {
        if !area.contains(x, y) {
            continue;
        }
        stack.push(((x - area.x0) as usize, (y - area.y0) as usize));
        while let Some((x, y)) = stack.pop() {
            let row = y * w;
            if cell.get(row + x) != Some(&0) {
                continue;
            }
            let (mut l, mut r) = (x, x);
            while l > 0 && cell.get(row + l - 1) == Some(&0) {
                l -= 1;
            }
            while r + 1 < w && cell.get(row + r + 1) == Some(&0) {
                r += 1;
            }
            if let Some(run) = cell.get_mut(row + l..=row + r) {
                run.fill(2);
            }
            for ny in [y.checked_sub(1), (y + 1 < h).then_some(y + 1)].into_iter().flatten() {
                let mut open = false;
                for nx in l..=r {
                    let free = cell.get(ny * w + nx) == Some(&0);
                    if free && !open {
                        stack.push((nx, ny));
                    }
                    open = free;
                }
            }
        }
    }
    // No selection paints everywhere (`mask_from_surface` reads it as nothing selected).
    let base = match &selection {
        Some(sel) => prism_algo::selection::mask_from_surface(Some(sel), area),
        None => vec![1.0; n],
    };
    let mut mask = vec![0.0f32; n];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let inside = match cell.get(i) {
                Some(2) => true,
                // A line pixel beside an entered area.
                Some(1) => (y.saturating_sub(1)..=(y + 1).min(h - 1)).any(|ny| (x.saturating_sub(1)..=(x + 1).min(w - 1)).any(|nx| cell.get(ny * w + nx) == Some(&2))),
                _ => false,
            };
            if let (true, Some(m), Some(b)) = (inside, mask.get_mut(i), base.get(i)) {
                *m = *b;
            }
        }
    }
    Some(prism_algo::selection::mask_to_surface(&mask, area))
}

/// The pixels a stroke can reach: the path's bounds grown by the brush size, inside the canvas.
/// `None` when there is nothing to constrain or the area is too large to scan.
fn stroke_area(doc: &Document, brush: &BrushSettings, pts: &[StrokePoint]) -> Option<Rect> {
    let finite = || pts.iter().filter(|p| p.x.is_finite() && p.y.is_finite());
    let (x0, x1) = finite().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.x), hi.max(p.x)));
    let (y0, y1) = finite().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
    if !(x0 <= x1 && y0 <= y1) {
        return None;
    }
    // Scatter, sprays and tip scale can reach further than the radius: a whole diameter.
    let grow = if brush.size.is_finite() { f64::from(brush.size.clamp(0.5, prism_paint::MAX_BRUSH_SIZE)) } else { 0.5 } + 2.0;
    let clamp = |v: f64| v.clamp(-1e9, 1e9) as i32;
    let area = Rect::new(clamp((x0 - grow).floor()), clamp((y0 - grow).floor()), clamp((x1 + grow).ceil()), clamp((y1 + grow).ceil())).intersect(&doc.bounds());
    let pixels = i64::from(area.width()) * i64::from(area.height());
    (!area.is_empty() && pixels <= MAX_AREA).then_some(area)
}

/// Pixels along the path, about one a pixel of travel (so a line between two samples is seen).
fn seeds(pts: &[StrokePoint]) -> impl Iterator<Item = (i32, i32)> + '_ {
    let ok = |p: &&StrokePoint| p.x.is_finite() && p.y.is_finite() && p.x.abs() < 1e9 && p.y.abs() < 1e9;
    let first = pts.iter().find(ok).map(|p| (p.x.floor() as i32, p.y.floor() as i32));
    let between = pts.windows(2).filter_map(move |w| match w {
        [a, b] if ok(&a) && ok(&b) => Some((*a, *b)),
        _ => None,
    });
    first.into_iter().chain(
        between
            .flat_map(|(a, b)| {
                let steps = ((b.x - a.x).abs().max((b.y - a.y).abs()).ceil() as usize).clamp(1, 100_000);
                (1..=steps).map(move |i| {
                    let t = i as f64 / steps as f64;
                    ((a.x + (b.x - a.x) * t).floor() as i32, (a.y + (b.y - a.y) * t).floor() as i32)
                })
            })
            .take(MAX_SEEDS),
    )
}

#[cfg(test)]
mod tests;
