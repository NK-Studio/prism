//! Enclose and Fill (`paint.encloseFill`): CLIP STUDIO PAINT's enclose-and-fill sub-tool. A lasso
//! outline is drawn and everything inside it that line art closes off, the lines included, is
//! filled with the foreground colour; open space that continues outside the lasso stays empty. The algorithm is
//! `prism_algo::enclose_fill`; this module validates params and writes the fill as one history
//! step, like the Paint Bucket (the boundary may come from all visible layers while only the
//! target receives the colour).

use prism_algo::enclose_fill::{EncloseOptions, MAX_DISTANCE, enclose_region};
use prism_algo::paint::bucket_fill_region_src;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const CMD: &str = "paint.encloseFill";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// An optional number param, which must be finite and within `range`.
fn num(p: &Value, key: &str, default: f32, range: std::ops::RangeInclusive<f32>) -> Result<f32> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => {
            let n = v.as_f64().map(|n| n as f32).filter(|n| n.is_finite()).ok_or_else(|| bad(format!("{key} must be a number")))?;
            if range.contains(&n) { Ok(n) } else { Err(bad(format!("{key} must be in {}..{}", range.start(), range.end()))) }
        }
    }
}

fn flag(p: &Value, key: &str, default: bool) -> Result<bool> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_bool().ok_or_else(|| bad(format!("{key} must be true or false"))),
    }
}

/// The lasso outline: at least three finite `[x, y]` points.
fn points(p: &Value) -> Result<Vec<(f32, f32)>> {
    let list = p.get("points").and_then(Value::as_array).ok_or_else(|| bad("points must be a list of [x, y] points"))?;
    let pts = list
        .iter()
        .map(|q| {
            let xy = |i: usize| q.get(i).and_then(Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite() && v.abs() <= 1.0e7);
            xy(0).zip(xy(1))
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| bad("every point must be [x, y] with finite coordinates"))?;
    if pts.len() < 3 {
        return Err(bad("the lasso needs at least 3 points"));
    }
    Ok(pts)
}

/// `#rrggbb[aa]` or `[r, g, b(, a)]` in 0..1; absent → the foreground colour.
fn color(p: &Value, fg: [f32; 4]) -> Result<[f32; 4]> {
    match p.get("color") {
        None | Some(Value::Null) => Ok(fg),
        Some(Value::String(h)) => {
            let h = h.trim_start_matches('#');
            let c = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|v| f32::from(v) / 255.0);
            match (h.len(), c(0), c(2), c(4)) {
                (6 | 8, Some(r), Some(g), Some(b)) => Ok([r, g, b, if h.len() == 8 { c(6).ok_or_else(|| bad("color is not #rrggbb"))? } else { 1.0 }]),
                _ => Err(bad("color must be #rrggbb")),
            }
        }
        Some(Value::Array(a)) if (3..=4).contains(&a.len()) => {
            let v: Option<Vec<f32>> = a.iter().map(|x| x.as_f64().map(|x| x as f32).filter(|x| (0.0..=1.0).contains(x))).collect();
            let v = v.ok_or_else(|| bad("color components must be numbers in 0..1"))?;
            Ok([v[0], v[1], v[2], v.get(3).copied().unwrap_or(1.0)])
        }
        Some(_) => Err(bad("color must be #rrggbb or [r, g, b]")),
    }
}

fn enclose_fill(s: &mut Session, p: &Value) -> Result<Value> {
    let pts = points(p)?;
    let options = EncloseOptions {
        tolerance: num(p, "tolerance", 51.0, 0.0..=255.0)?,
        close_gap: num(p, "closeGap", 0.0, 0.0..=MAX_DISTANCE)?,
        expand: num(p, "expand", 0.0, -MAX_DISTANCE..=MAX_DISTANCE)?,
        anti_alias: flag(p, "antiAlias", true)?,
    };
    let opacity = num(p, "opacity", 100.0, 0.0..=100.0)? / 100.0;
    let c = color(p, s.tools.foreground)?;
    // Clip Studio's enclose-and-fill refers to all layers by default; channel targets refer to
    // the active layer's own pixels.
    let all = flag(p, "sampleAllLayers", true)? && crate::channel_cmds::target_of(p) == crate::channel_cmds::Target::Pixels;
    let (area, img) = crate::selection_cmds::sample_rgba8(s, all)?;
    let found = enclose_region(&img, area, &pts, &options);
    drop(img);
    let Some(found) = found else { return Ok(json!({"filled": false, "areas": 0})) };
    s.edit("Enclose and Fill", |doc, active| {
        let sel = doc.selection.clone();
        let (surf, lock) = crate::channel_cmds::target_surface(doc, *active, p)?;
        bucket_fill_region_src(surf, &found.region, opacity, sel.as_ref(), lock, |_, _| c);
        surf.prune();
        Ok(())
    })?;
    Ok(json!({"filled": true, "areas": found.areas}))
}

/// Enclose and Fill command spec.
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Enclose and Fill",
        menu: &[],
        shortcut: None,
        params: r##"{"points":[[x,y],…] (lasso outline, ≥3),"tolerance":0..255=51 (ink at or below it is open, not line art),"closeGap":px 0..256=0 (passages narrower than this don't connect areas),"expand":px -256..256=0 (negative shrinks the fill away from the open space outside; the fill already covers the line art),"antiAlias":bool=true,"sampleAllLayers":bool=true,"color":"#rrggbb"=foreground,"opacity":0..100=100,"target":"pixels"|"mask"|"quickMask"|{"channel":i}=Channels panel target} → {"filled":bool,"areas":n}"##,
        enabled: crate::commands::has_paintable,
        run: enclose_fill,
        journal: true,
    }]
}

#[cfg(test)]
#[path = "enclose_fill_cmds_tests.rs"]
mod tests;
