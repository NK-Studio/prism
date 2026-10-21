//! CLIP STUDIO special rulers on layers: add, edit, delete and show parallel line and
//! perspective rulers, and snap a polyline to them (`specialRuler.snapPoints`, the geometry the
//! canvas applies to brush strokes). Rulers are layer data, so edits are history steps that
//! change no pixels. See `docs/csp-special-rulers.md`.

use prism_doc::special_ruler::{MAX_GUIDES, MAX_VANISHING_POINTS, PerspectiveRuler, RulerKind, RulerRange, SpecialRuler, SpecialRulers, VanishTarget, VanishingPoint, normalize_angle};
use prism_doc::{Document, LayerId};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::special_ruler_snap::{StrokeSnapper, candidates};
use crate::{EngineError, Result, Session};

const LIST: &str = "layer.specialRuler.list";
const ADD_PARALLEL: &str = "layer.specialRuler.addParallel";
const ADD_PERSPECTIVE: &str = "layer.specialRuler.addPerspective";
const EDIT: &str = "layer.specialRuler.edit";
const DELETE: &str = "layer.specialRuler.delete";
const SET_SHOW: &str = "layer.specialRuler.setShow";
const SNAP_POINTS: &str = "specialRuler.snapPoints";

/// Points `specialRuler.snapPoints` accepts in one call.
const MAX_SNAP_POINTS: usize = 100_000;
/// Screen points a stroke moves before its ruler line locks (as ⇧ strokes).
pub const LOCK_SCREEN_DISTANCE: f64 = 3.0;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    st.active_layer.filter(|id| st.doc.layer(*id).is_some()).map(|_| ()).ok_or_else(|| "no active layer".into())
}

/// The `layer` param, or the active layer; it must exist.
fn target(s: &Session, cmd: &str, p: &Value) -> Result<LayerId> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = match p.get("layer") {
        None | Some(Value::Null) => st.active_layer.ok_or_else(|| bad(cmd, "no active layer"))?,
        Some(v) => LayerId(v.as_u64().ok_or_else(|| bad(cmd, "`layer` must be a layer id"))?),
    };
    st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    Ok(id)
}

fn number(cmd: &str, p: &Value, key: &str) -> Result<Option<f64>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().filter(|f| f.is_finite()).map(Some).ok_or_else(|| bad(cmd, format!("`{key}` must be a finite number"))),
    }
}

fn boolean(cmd: &str, p: &Value, key: &str) -> Result<Option<bool>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_bool().map(Some).ok_or_else(|| bad(cmd, format!("`{key}` must be true or false"))),
    }
}

fn point(cmd: &str, v: &Value, what: &str) -> Result<[f64; 2]> {
    let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad(cmd, format!("`{what}` must be [x, y]")))?;
    let c = |i: usize| a.get(i).and_then(Value::as_f64).filter(|f| f.is_finite()).ok_or_else(|| bad(cmd, format!("`{what}` must hold finite numbers")));
    Ok([c(0)?, c(1)?])
}

fn opt_point(cmd: &str, p: &Value, key: &str) -> Result<Option<[f64; 2]>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => point(cmd, v, key).map(Some),
    }
}

fn centre(doc: &Document) -> [f64; 2] {
    [f64::from(doc.size.width) / 2.0, f64::from(doc.size.height) / 2.0]
}

/// One vanishing point from `{x, y}` (converging) or `{parallel: angle}`, with `snap?` and
/// `handle?`.
fn vanishing_point(cmd: &str, v: &Value, doc_centre: [f64; 2]) -> Result<VanishingPoint> {
    if !v.is_object() {
        return Err(bad(cmd, "each point must be {x, y} or {parallel: angle}"));
    }
    let snap = boolean(cmd, v, "snap")?.unwrap_or(true);
    let handle = opt_point(cmd, v, "handle")?;
    let target = match (number(cmd, v, "parallel")?, number(cmd, v, "x")?, number(cmd, v, "y")?) {
        (Some(a), None, None) => VanishTarget::Parallel { angle_deg: normalize_angle(a) },
        (None, Some(x), Some(y)) => VanishTarget::Point([x, y]),
        _ => return Err(bad(cmd, "each point must be {x, y} or {parallel: angle}")),
    };
    let handle = handle.unwrap_or(match target {
        VanishTarget::Point(q) => q,
        VanishTarget::Parallel { .. } => doc_centre,
    });
    Ok(VanishingPoint { snap, target, handle, guides: Vec::new() })
}

/// Apply `f` to the layer's ruler set (created when missing), validate, and record one step.
fn edit_rulers<R>(s: &mut Session, cmd: &str, label: &str, layer: LayerId, f: impl FnOnce(&mut SpecialRulers) -> Result<R>) -> Result<R> {
    let current = s.active().and_then(|st| st.doc.layer(layer)).and_then(|l| l.special_rulers.clone()).unwrap_or_default();
    let mut next = current.clone();
    let out = f(&mut next)?;
    next.validate().map_err(|e| bad(cmd, e))?;
    if next != current {
        s.edit(label, |doc, _| {
            let l = doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?;
            l.special_rulers = (!next.is_empty() || next.range != RulerRange::AllLayers || !next.shown).then_some(next);
            Ok(())
        })?;
        if let Some(st) = s.active_mut() {
            // Layer data only: no pixels change.
            st.last_damage = Some(prism_geom::Rect::EMPTY);
        }
    }
    Ok(out)
}

fn rulers_json(layer: LayerId, r: &SpecialRulers) -> Value {
    json!({ "layer": layer.0, "shown": r.shown, "range": r.range.name(), "rulers": serde_json::to_value(&r.rulers).unwrap_or(Value::Null) })
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let layer = target(s, LIST, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let own = st.doc.layer(layer).and_then(|l| l.special_rulers.clone()).unwrap_or_default();
    let applies: Vec<Value> = crate::special_ruler_snap::rulers_for(&st.doc, layer).into_iter().map(|(id, r)| rulers_json(id, r)).collect();
    Ok(json!({ "own": rulers_json(layer, &own), "applies": applies }))
}

fn add_parallel(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = ADD_PARALLEL;
    let layer = target(s, cmd, p)?;
    let angle = number(cmd, p, "angle")?.ok_or_else(|| bad(cmd, "`angle` (degrees, counter-clockwise) is required"))?;
    let doc_centre = s.active().map(|st| centre(&st.doc)).unwrap_or_default();
    let center = opt_point(cmd, p, "center")?.unwrap_or(doc_centre);
    let snap = boolean(cmd, p, "snap")?.unwrap_or(true);
    let ruler = SpecialRuler { snap, kind: RulerKind::Parallel { angle_deg: normalize_angle(angle), center } };
    let index = edit_rulers(s, cmd, "Parallel Line Ruler", layer, |r| {
        r.rulers.push(ruler);
        Ok(r.rulers.len() - 1)
    })?;
    Ok(json!({ "layer": layer.0, "index": index }))
}

fn add_perspective(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = ADD_PERSPECTIVE;
    let layer = target(s, cmd, p)?;
    let doc_centre = s.active().map(|st| centre(&st.doc)).unwrap_or_default();
    let raw = p.get("points").and_then(Value::as_array).ok_or_else(|| bad(cmd, "`points` must be an array of {x, y} or {parallel: angle}"))?;
    if raw.is_empty() || raw.len() > MAX_VANISHING_POINTS {
        return Err(bad(cmd, format!("`points` needs 1 to {MAX_VANISHING_POINTS} vanishing points")));
    }
    let points = raw.iter().map(|v| vanishing_point(cmd, v, doc_centre)).collect::<Result<Vec<_>>>()?;
    let eye_level = opt_point(cmd, p, "eyeLevel")?;
    let snap = boolean(cmd, p, "snap")?.unwrap_or(true);
    let ruler = SpecialRuler { snap, kind: RulerKind::Perspective(PerspectiveRuler { points, eye_level }) };
    let index = edit_rulers(s, cmd, "Perspective Ruler", layer, |r| {
        r.rulers.push(ruler);
        Ok(r.rulers.len() - 1)
    })?;
    Ok(json!({ "layer": layer.0, "index": index }))
}

fn index(cmd: &str, p: &Value, key: &str) -> Result<Option<usize>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().and_then(|i| usize::try_from(i).ok()).map(Some).ok_or_else(|| bad(cmd, format!("`{key}` must be an index"))),
    }
}

fn edit(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = EDIT;
    let layer = target(s, cmd, p)?;
    let i = index(cmd, p, "index")?.ok_or_else(|| bad(cmd, "`index` is required"))?;
    let snap = boolean(cmd, p, "snap")?;
    let angle = number(cmd, p, "angle")?;
    let center = opt_point(cmd, p, "center")?;
    let eye_level = opt_point(cmd, p, "eyeLevel")?;
    let vp = index(cmd, p, "point")?;
    let (x, y, parallel) = (number(cmd, p, "x")?, number(cmd, p, "y")?, number(cmd, p, "parallel")?);
    let handle = opt_point(cmd, p, "handle")?;
    let point_snap = boolean(cmd, p, "pointSnap")?;
    let guides = match p.get("guides") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let a = v.as_array().filter(|a| a.len() <= MAX_GUIDES).ok_or_else(|| bad(cmd, format!("`guides` must be an array of at most {MAX_GUIDES} [x, y] points")))?;
            Some(a.iter().map(|g| point(cmd, g, "guides")).collect::<Result<Vec<_>>>()?)
        }
    };
    edit_rulers(s, cmd, "Edit Special Ruler", layer, |r| {
        let ruler = r.rulers.get_mut(i).ok_or_else(|| bad(cmd, format!("layer has no ruler {i}")))?;
        if let Some(v) = snap {
            ruler.snap = v;
        }
        match &mut ruler.kind {
            RulerKind::Parallel { angle_deg, center: c } => {
                if eye_level.is_some() || vp.is_some() || x.is_some() || y.is_some() || parallel.is_some() || handle.is_some() || point_snap.is_some() || guides.is_some() {
                    return Err(bad(cmd, "a parallel ruler takes `angle`, `center` and `snap`"));
                }
                if let Some(a) = angle {
                    *angle_deg = normalize_angle(a);
                }
                if let Some(v) = center {
                    *c = v;
                }
            }
            RulerKind::Perspective(per) => {
                if angle.is_some() || center.is_some() {
                    return Err(bad(cmd, "a perspective ruler takes `eyeLevel`, `snap` and `point` edits"));
                }
                if let Some(e) = eye_level {
                    per.eye_level = Some(e);
                }
                let point_edit = x.is_some() || y.is_some() || parallel.is_some() || handle.is_some() || point_snap.is_some() || guides.is_some();
                match (vp, point_edit) {
                    (None, true) => return Err(bad(cmd, "`point` (vanishing point index) is required for point edits")),
                    (Some(j), _) => {
                        let v = per.points.get_mut(j).ok_or_else(|| bad(cmd, format!("ruler {i} has no vanishing point {j}")))?;
                        match (x, y, parallel) {
                            (Some(x), Some(y), None) => v.target = VanishTarget::Point([x, y]),
                            (None, None, Some(a)) => v.target = VanishTarget::Parallel { angle_deg: normalize_angle(a) },
                            (None, None, None) => {}
                            _ => return Err(bad(cmd, "move a point with `x` and `y`, or make it parallel with `parallel`")),
                        }
                        if let Some(h) = handle {
                            v.handle = h;
                        } else if let (Some(x), Some(y)) = (x, y) {
                            v.handle = [x, y];
                        }
                        if let Some(b) = point_snap {
                            v.snap = b;
                        }
                        if let Some(g) = guides {
                            v.guides = g;
                        }
                    }
                    (None, false) => {}
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({ "layer": layer.0, "index": i }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = DELETE;
    let layer = target(s, cmd, p)?;
    let i = index(cmd, p, "index")?;
    let removed = edit_rulers(s, cmd, "Delete Special Ruler", layer, |r| match i {
        Some(i) if i < r.rulers.len() => {
            r.rulers.remove(i);
            Ok(1)
        }
        Some(i) => Err(bad(cmd, format!("layer has no ruler {i}"))),
        None => Ok(std::mem::take(&mut r.rulers).len()),
    })?;
    Ok(json!({ "layer": layer.0, "removed": removed }))
}

fn set_show(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = SET_SHOW;
    let layer = target(s, cmd, p)?;
    let shown = boolean(cmd, p, "shown")?;
    let range = match p.get("range") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().and_then(RulerRange::from_name).ok_or_else(|| bad(cmd, "`range` must be all, folder or editing"))?),
    };
    if shown.is_none() && range.is_none() {
        return Err(bad(cmd, "give `shown` and/or `range`"));
    }
    let (shown, range) = edit_rulers(s, cmd, "Show Special Ruler", layer, |r| {
        if let Some(v) = shown {
            r.shown = v;
        }
        if let Some(v) = range {
            r.range = v;
        }
        Ok((r.shown, r.range))
    })?;
    Ok(json!({ "layer": layer.0, "shown": shown, "range": range.name() }))
}

/// Snap a polyline as a stroke drawn on `layer` would be: `[x, y, …rest]` points keep their
/// other values (pressure, tilt).
fn snap_points(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = SNAP_POINTS;
    let layer = target(s, cmd, p)?;
    let raw = p.get("points").and_then(Value::as_array).ok_or_else(|| bad(cmd, "`points` must be an array of [x, y, …]"))?;
    if raw.len() > MAX_SNAP_POINTS {
        return Err(bad(cmd, format!("at most {MAX_SNAP_POINTS} points")));
    }
    let zoom = number(cmd, p, "zoom")?.unwrap_or(1.0);
    if zoom <= 0.0 {
        return Err(bad(cmd, "`zoom` must be positive"));
    }
    let mut pts = Vec::with_capacity(raw.len());
    for v in raw {
        let a = v.as_array().filter(|a| a.len() >= 2).ok_or_else(|| bad(cmd, "each point must be [x, y, …]"))?;
        let c = |i: usize| a.get(i).and_then(Value::as_f64).filter(|f| f.is_finite()).ok_or_else(|| bad(cmd, "point coordinates must be finite numbers"));
        pts.push(([c(0)?, c(1)?], a.get(2..).unwrap_or_default().to_vec()));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let Some((start, _)) = pts.first() else {
        return Ok(json!({ "points": [], "line": null }));
    };
    let mut snapper = StrokeSnapper::new(candidates(&st.doc, layer, *start), *start, LOCK_SCREEN_DISTANCE / zoom);
    let out: Vec<Value> = pts
        .iter()
        .map(|(xy, rest)| {
            let [x, y] = snapper.push(*xy);
            let mut v = vec![json!(x), json!(y)];
            v.extend(rest.iter().cloned());
            Value::Array(v)
        })
        .collect();
    let line = snapper.locked().map(|l| json!({ "origin": l.origin, "dir": l.dir, "layer": l.layer.0, "ruler": l.ruler, "point": l.point }));
    Ok(json!({ "points": out, "line": line }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: LIST,
            label: "List Special Rulers",
            menu: &[],
            shortcut: None,
            params: r#"{"layer":id?=active} → {own:{layer,shown,range,rulers}, applies:[{layer,shown,range,rulers}] (every shown ruler that snaps strokes on that layer)}"#,
            enabled: has_layer,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: ADD_PARALLEL,
            label: "Parallel Line Ruler",
            menu: &[],
            shortcut: None,
            params: r#"{"layer":id?=active,"angle":degrees counter-clockwise (y up),"center":[x,y]?=document centre,"snap":bool=true} → {layer,index}"#,
            enabled: has_layer,
            run: add_parallel,
            journal: true,
        },
        CommandSpec {
            id: ADD_PERSPECTIVE,
            label: "Perspective Ruler",
            menu: &[],
            shortcut: None,
            params: r#"{"layer":id?=active,"points":[{"x","y"} converging | {"parallel":degrees} parallel; each with "snap":bool=true,"handle":[x,y]?] (1-16),"eyeLevel":[x,y]?,"snap":bool=true} → {layer,index}"#,
            enabled: has_layer,
            run: add_perspective,
            journal: true,
        },
        CommandSpec {
            id: EDIT,
            label: "Edit Special Ruler",
            menu: &[],
            shortcut: None,
            params: r#"{"layer":id?=active,"index":ruler,"snap":bool?; parallel: "angle"?,"center":[x,y]?; perspective: "eyeLevel":[x,y]?,"point":vanishing point index?,"x","y"? (move) | "parallel":degrees? (make parallel),"handle":[x,y]?,"guides":[[x,y]] (replaces the point's guide handles, ≤16)?,"pointSnap":bool?} (accepts "coalesce")"#,
            enabled: has_layer,
            run: edit,
            journal: true,
        },
        CommandSpec {
            id: DELETE,
            label: "Delete Special Ruler",
            menu: &[],
            shortcut: None,
            params: r#"{"layer":id?=active,"index":ruler? (omit: every ruler on the layer)} → {layer,removed}"#,
            enabled: has_layer,
            run: delete,
            journal: true,
        },
        CommandSpec {
            id: SET_SHOW,
            label: "Show Special Ruler",
            menu: &[],
            shortcut: None,
            params: r#"{"layer":id?=active,"shown":bool?,"range":"all|folder|editing"?} → {layer,shown,range}"#,
            enabled: has_layer,
            run: set_show,
            journal: true,
        },
        CommandSpec {
            id: SNAP_POINTS,
            label: "Snap Points to Special Rulers",
            menu: &[],
            shortcut: None,
            params: r#"{"points":[[x,y,…]] (a stroke, first point = start),"layer":id?=active,"zoom":number=1 (lock distance is 3 screen px)} → {points (extra values kept), line:{origin,dir,layer,ruler,point}?}"#,
            enabled: has_layer,
            run: snap_points,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;
