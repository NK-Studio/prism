//! Point-level path edits, what Photoshop's Direct Selection (A) and Convert Point tools do on
//! the canvas: move anchors, drag direction handles, bend segments and convert points. The
//! geometry is `prism_vector::edit`; the canvas previews a drag with it and commits the
//! whole drag as one command (one history step).
//!
//! Every command targets a path like `path.transform`: `"name":"work"` (default), a saved path's
//! name, or `"layer"` with `"layer":id?` for a shape layer's path (re-rendered; it stops being a
//! live shape) or a layer's vector mask. Knots are addressed by `subpath` and `knot` index, as
//! listed by `path.info`, and the result is `{name, path}` in the compact JSON form.

use prism_doc::{LayerContent, Path};
use prism_geom::Point;
use prism_vector::edit::{self, Handle};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::vector_cmds::{bad, has_doc, layer_id, nums, path_json, path_mut, pt, with_shape};
use crate::{Result, Session};

/// Coordinates and offsets beyond this are mistakes (the same bound as `path.transform`).
const MAX: f64 = 1_000_000.0;

fn finite(v: [f64; 2]) -> bool {
    v.iter().all(|c| c.is_finite() && c.abs() <= MAX)
}

fn offset(cmd: &str, p: &Value) -> Result<[f64; 2]> {
    nums::<2>(p, "move").filter(|d| finite(*d)).ok_or_else(|| bad(cmd, "`move` must be [dx, dy] with finite numbers of magnitude at most 1000000"))
}

fn point(cmd: &str, p: &Value, key: &str) -> Result<Option<Point>> {
    p.get(key)
        .map(|v| {
            pt(v).filter(|q| finite([q.x, q.y])).ok_or_else(|| bad(cmd, format!("`{key}` must be [x, y] with finite numbers of magnitude at most 1000000")))
        })
        .transpose()
}

fn index(cmd: &str, p: &Value, key: &str) -> Result<usize> {
    p.get(key).and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(cmd, format!("missing `{key}` index")))
}

fn knot(cmd: &str, p: &Value) -> Result<[usize; 2]> {
    Ok([index(cmd, p, "subpath")?, index(cmd, p, "knot")?])
}

/// Applies `f` to the targeted path in one undoable step labelled `label`.
fn edit_path(s: &mut Session, cmd: &str, p: &Value, label: &str, f: impl FnOnce(&mut Path) -> std::result::Result<(), String>) -> Result<Value> {
    let name = p.get("name").and_then(Value::as_str).unwrap_or("work").to_owned();
    let layer = if name == "layer" { Some(layer_id(s, p)?) } else { None };
    let shape = layer.filter(|id| s.active().and_then(|d| d.doc.layer(*id)).is_some_and(|l| matches!(l.content, LayerContent::Shape(_))));
    let path = match shape {
        Some(id) => with_shape(s, id, label, |sh, _| {
            f(&mut sh.path).map_err(|e| bad(cmd, e))?;
            sh.live = None;
            sh.psd_raw = None;
            Ok(path_json(&sh.path))
        })?,
        None => s.edit(label, |doc, _| {
            let path = path_mut(doc, &name, layer)?;
            f(path).map_err(|e| bad(cmd, e))?;
            Ok(path_json(path))
        })?,
    };
    Ok(json!({ "name": name, "path": path }))
}

fn move_anchors(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.moveAnchors";
    let anchors: Vec<[usize; 2]> = p
        .get("anchors")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| bad(CMD, "`anchors` must list [subpath, knot] pairs"))?
        .iter()
        .map(|a| {
            let n = |i: usize| a.get(i).and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok());
            n(0).zip(n(1)).map(|(s, k)| [s, k]).ok_or_else(|| bad(CMD, "each anchor is [subpath, knot]"))
        })
        .collect::<Result<_>>()?;
    let d = offset(CMD, p)?;
    edit_path(s, CMD, p, "Drag Anchor Points", |path| edit::move_anchors(path, &anchors, d))
}

fn move_handle(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.moveHandle";
    let k = knot(CMD, p)?;
    let h = p.get("handle").and_then(Value::as_str).and_then(Handle::parse).ok_or_else(|| bad(CMD, "`handle` must be \"in\" or \"out\""))?;
    let to = point(CMD, p, "to")?.ok_or_else(|| bad(CMD, "missing `to`"))?;
    let independent = p.get("independent").and_then(Value::as_bool).unwrap_or(false);
    edit_path(s, CMD, p, "Drag Direction Point", |path| edit::move_handle(path, k, h, to, independent))
}

fn bend_segment(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.bendSegment";
    let k = knot(CMD, p)?;
    let t = match p.get("t") {
        Some(v) => v.as_f64().filter(|t| (0.0..=1.0).contains(t)).ok_or_else(|| bad(CMD, "`t` must be between 0 and 1"))?,
        None => 0.5,
    };
    let d = offset(CMD, p)?;
    edit_path(s, CMD, p, "Drag Segment", |path| edit::bend_segment(path, k, t, d))
}

fn convert_point(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.convertPoint";
    let k = knot(CMD, p)?;
    let out = point(CMD, p, "out")?;
    edit_path(s, CMD, p, "Convert Point", |path| edit::convert_point(path, k, out))
}

fn add_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.addAnchor";
    let k = knot(CMD, p)?;
    let t = match p.get("t") {
        Some(v) => v.as_f64().filter(|t| (0.0..=1.0).contains(t)).ok_or_else(|| bad(CMD, "`t` must be between 0 and 1"))?,
        None => 0.5,
    };
    let mut added = None;
    let mut r = edit_path(s, CMD, p, "Add Anchor Point", |path| {
        added = Some(edit::add_anchor(path, k, t)?);
        Ok(())
    })?;
    r["knot"] = json!(added);
    Ok(r)
}

fn delete_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.deleteAnchor";
    let k = knot(CMD, p)?;
    edit_path(s, CMD, p, "Delete Anchor Point", |path| edit::delete_anchor(path, k))
}

/// `"subpaths":[i,…]`, the components to act on; `None` when absent (the caller's default).
fn subpath_list(cmd: &str, p: &Value) -> Result<Option<Vec<usize>>> {
    let Some(v) = p.get("subpaths") else { return Ok(None) };
    let list = v.as_array().filter(|a| !a.is_empty()).ok_or_else(|| bad(cmd, "`subpaths` must list subpath indices"))?;
    // Duplicates and absurd lists are harmless but capped: a path has at most this many.
    if list.len() > 100_000 {
        return Err(bad(cmd, "too many `subpaths`"));
    }
    list.iter()
        .map(|i| i.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(cmd, "each of `subpaths` is an index")))
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

fn all(path: &Path) -> Vec<usize> {
    (0..path.subpaths.len()).collect()
}

fn edge(cmd: &str, p: &Value) -> Result<edit::Edge> {
    p.get("edge")
        .and_then(Value::as_str)
        .and_then(edit::Edge::parse)
        .ok_or_else(|| bad(cmd, "`edge` must be left, hcenter, right, top, vcenter or bottom"))
}

fn align_components(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.alignComponents";
    let e = edge(CMD, p)?;
    let subs = subpath_list(CMD, p)?;
    let canvas = match p.get("to").and_then(Value::as_str).unwrap_or("selection") {
        "selection" => None,
        "canvas" => {
            let b = s.active().ok_or(crate::EngineError::NoDocument)?.doc.bounds();
            Some([f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)])
        }
        _ => return Err(bad(CMD, "`to` must be \"selection\" or \"canvas\"")),
    };
    edit_path(s, CMD, p, "Align Components", |path| {
        let subs = subs.unwrap_or_else(|| all(path));
        edit::align_components(path, &subs, e, canvas)
    })
}

fn distribute_components(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.distributeComponents";
    let e = edge(CMD, p)?;
    let subs = subpath_list(CMD, p)?;
    edit_path(s, CMD, p, "Distribute Components", |path| {
        let subs = subs.unwrap_or_else(|| all(path));
        edit::distribute_components(path, &subs, e)
    })
}

fn arrange_components(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.arrangeComponents";
    let to = p
        .get("to")
        .and_then(Value::as_str)
        .and_then(edit::Arrange::parse)
        .ok_or_else(|| bad(CMD, "`to` must be front, forward, backward or back"))?;
    let subs = subpath_list(CMD, p)?.ok_or_else(|| bad(CMD, "missing `subpaths`"))?;
    let mut moved = Vec::new();
    let mut r = edit_path(s, CMD, p, "Arrange Components", |path| {
        moved = edit::arrange_components(path, &subs, to)?;
        Ok(())
    })?;
    r["subpaths"] = json!(moved);
    Ok(r)
}

fn set_operation(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "path.setOperation";
    let op = p
        .get("op")
        .and_then(Value::as_str)
        .and_then(crate::vector_cmds::op_from)
        .filter(|op| *op != prism_doc::PathOp::Join)
        .ok_or_else(|| bad(CMD, "`op` must be combine, subtract, intersect or exclude"))?;
    let subs = subpath_list(CMD, p)?;
    edit_path(s, CMD, p, "Path Operation", |path| {
        // By default every component but the first, whose operation never applies.
        let subs = subs.unwrap_or_else(|| (1..path.subpaths.len()).collect());
        edit::set_operation(path, &subs, op)
    })
}

fn merge_components(s: &mut Session, p: &Value) -> Result<Value> {
    let tol = p.get("tolerance").and_then(Value::as_f64).filter(|t| t.is_finite()).unwrap_or(0.1).clamp(0.05, 10.0);
    edit_path(s, "path.mergeComponents", p, "Merge Shape Components", |path| {
        *path = crate::vector_cmds::merged_path(path, tol)?;
        Ok(())
    })
}

pub fn specs() -> Vec<CommandSpec> {
    const TARGET: &str = r#""name":str|"work"|"layer"="work","layer":id? (with "layer": a shape layer's path or a vector mask)"#;
    let spec = |id: &'static str, label: &'static str, params: String, run: fn(&mut Session, &Value) -> Result<Value>| CommandSpec {
        id,
        label,
        menu: &[],
        shortcut: None,
        params: crate::vector_cmds::leak(format!("{{{TARGET},{params}}} → {{name,path}}")),
        enabled: has_doc,
        run,
        journal: true,
    };
    vec![
        spec("path.moveAnchors", "Drag Anchor Points", r#""anchors":[[subpath,knot],…],"move":[dx,dy] (anchors move with their handles; Direct Selection drag)"#.into(), move_anchors),
        spec(
            "path.moveHandle",
            "Drag Direction Point",
            r#""subpath":i,"knot":j,"handle":"in|out","to":[x,y],"independent":bool=false (a smooth point turns its other handle to stay collinear; independent = ⌥ / Convert Point, makes a corner)"#.into(),
            move_handle,
        ),
        spec(
            "path.bendSegment",
            "Drag Segment",
            r#""subpath":i,"knot":j (the segment leaving it),"t":0..1=0.5 (where it was grabbed),"move":[dx,dy] (a straight segment moves with its anchors; a curve reshapes through the dragged point)"#.into(),
            bend_segment,
        ),
        spec(
            "path.convertPoint",
            "Convert Point",
            r#""subpath":i,"knot":j,"out":[x,y]? (none: corner with retracted handles; given: smooth with this out handle and its mirror)"#.into(),
            convert_point,
        ),
        spec(
            "path.addAnchor",
            "Add Anchor Point",
            r#""subpath":i,"knot":j (the segment leaving it),"t":0..1=0.5 (the curve is split exactly; Pen Auto Add) → also "knot":[subpath,knot] of the new point"#.into(),
            add_anchor,
        ),
        spec(
            "path.deleteAnchor",
            "Delete Anchor Point",
            r#""subpath":i,"knot":j (its neighbours join; an emptied subpath is removed; Pen Auto Delete)"#.into(),
            delete_anchor,
        ),
        spec(
            "path.setOperation",
            "Path Operation",
            r#""op":"combine|subtract|intersect|exclude","subpaths":[i,…]? (the components holding them; default all but the first)"#.into(),
            set_operation,
        ),
        spec(
            "path.alignComponents",
            "Align Components",
            r#""edge":"left|hcenter|right|top|vcenter|bottom","to":"selection|canvas"="selection","subpaths":[i,…]? (the components holding them; default all) (Pen options bar › Path Alignment)"#.into(),
            align_components,
        ),
        spec(
            "path.distributeComponents",
            "Distribute Components",
            r#""edge":"left|hcenter|right|top|vcenter|bottom","subpaths":[i,…]? (three or more components; default all)"#.into(),
            distribute_components,
        ),
        spec(
            "path.arrangeComponents",
            "Arrange Components",
            r#""to":"front|forward|backward|back","subpaths":[i,…] (the components holding them) → also "subpaths": their new indices (Pen options bar › Path Arrangement)"#.into(),
            arrange_components,
        ),
        spec(
            "path.mergeComponents",
            "Merge Shape Components",
            r#""tolerance":0.05..10=0.1 (the components become one outline of the area they fill)"#.into(),
            merge_components,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use prism_doc::{LayerId, ShapeLayer};
    use serde_json::json;

    use crate::Session;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 120, "height": 100, "background": "white"})).unwrap();
        s
    }

    fn square() -> serde_json::Value {
        json!({"subpaths": [{"closed": true, "knots": [[10, 10], [60, 10], [60, 60], [10, 60]]}]})
    }

    fn anchor(v: &serde_json::Value, k: usize) -> [f64; 2] {
        serde_json::from_value(v["path"]["subpaths"][0]["knots"][k]["anchor"].clone()).unwrap()
    }

    fn work(s: &Session) -> prism_doc::Path {
        s.active().unwrap().doc.work_path.clone().unwrap()
    }

    #[test]
    fn anchors_move_in_one_undoable_step() {
        let mut s = session();
        s.execute("path.set", json!({"path": square()})).unwrap();
        let before = work(&s);
        let r = s.execute("path.moveAnchors", json!({"anchors": [[0, 1], [0, 2]], "move": [5, -3]})).unwrap();
        assert_eq!((anchor(&r, 1), anchor(&r, 2), anchor(&r, 0)), ([65.0, 7.0], [65.0, 57.0], [10.0, 10.0]));
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Drag Anchor Points"));
        assert!(s.undo());
        assert_eq!(work(&s), before);
    }

    #[test]
    fn a_coalesced_drag_is_one_history_step() {
        let mut s = session();
        s.execute("path.set", json!({"path": square()})).unwrap();
        let steps = s.active().unwrap().history.entries().len();
        for _ in 0..3 {
            s.execute("path.moveAnchors", json!({"anchors": [[0, 0]], "move": [1, 0], "coalesce": "drag-1"})).unwrap();
        }
        assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
        assert_eq!(work(&s).subpaths[0].knots[0].anchor.x, 13.0);
    }

    #[test]
    fn handles_segments_and_convert_point() {
        let mut s = session();
        s.execute("path.set", json!({"path": square()})).unwrap();
        // Pull smooth handles out of the corner at (60, 10), then turn one: the other follows.
        s.execute("path.convertPoint", json!({"subpath": 0, "knot": 1, "out": [70, 10]})).unwrap();
        let r = s.execute("path.moveHandle", json!({"subpath": 0, "knot": 1, "handle": "out", "to": [60, 20]})).unwrap();
        let k = &r["path"]["subpaths"][0]["knots"][1];
        assert_eq!((k["in"].clone(), k["smooth"].clone()), (json!([60.0, 0.0]), json!(true)));
        let r = s.execute("path.moveHandle", json!({"subpath": 0, "knot": 1, "handle": "in", "to": [50, 0], "independent": true})).unwrap();
        let k = &r["path"]["subpaths"][0]["knots"][1];
        assert_eq!((k["out"].clone(), k["smooth"].clone()), (json!([60.0, 20.0]), json!(false)));
        // The straight bottom edge (2 → 3) drags with its anchors.
        let r = s.execute("path.bendSegment", json!({"subpath": 0, "knot": 2, "move": [0, 8]})).unwrap();
        assert_eq!((anchor(&r, 2), anchor(&r, 3)), ([60.0, 68.0], [10.0, 68.0]));
        let r = s.execute("path.convertPoint", json!({"subpath": 0, "knot": 1})).unwrap();
        assert_eq!(r["path"]["subpaths"][0]["knots"][1]["out"], json!([60.0, 10.0]));
    }

    #[test]
    fn shape_paths_and_vector_masks_are_edited_and_rerendered() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 120, "height": 100, "depth": depth})).unwrap();
            let id = s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 40], "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
            let r = s.execute("path.moveAnchors", json!({"name": "layer", "layer": id, "anchors": [[0, 1], [0, 2]], "move": [30, 0]})).unwrap();
            assert_eq!(anchor(&r, 1), [80.0, 10.0]);
            let st = s.active().unwrap();
            let prism_doc::LayerContent::Shape(ShapeLayer { live, cache, .. }) = &st.doc.layer(LayerId(id)).unwrap().content else { panic!("shape") };
            assert!(live.is_none(), "an edited rectangle is a plain path now");
            assert_eq!(cache.as_ref().unwrap().content_bounds().x1, 80, "depth {depth}: re-rendered");
        }
        let mut s = session();
        let id = s.active().unwrap().active_layer.unwrap().0;
        s.execute("layer.vectorMask.add", json!({"layer": id, "path": square()})).unwrap();
        s.execute("path.moveAnchors", json!({"name": "layer", "layer": id, "anchors": [[0, 0]], "move": [-5, -5]})).unwrap();
        let vm = s.active().unwrap().doc.layer(LayerId(id)).unwrap().vector_mask.clone().unwrap();
        assert_eq!(vm.path.subpaths[0].knots[0].anchor, prism_geom::Point::new(5.0, 5.0));
    }

    #[test]
    fn auto_add_and_delete_anchor_points() {
        let mut s = session();
        s.execute("path.set", json!({"path": square()})).unwrap();
        let r = s.execute("path.addAnchor", json!({"subpath": 0, "knot": 0, "t": 0.5})).unwrap();
        assert_eq!((r["knot"].clone(), anchor(&r, 1)), (json!([0, 1]), [35.0, 10.0]));
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
        let r = s.execute("path.deleteAnchor", json!({"subpath": 0, "knot": 1})).unwrap();
        assert_eq!(anchor(&r, 1), [60.0, 10.0]);
        assert_eq!(work(&s).subpaths[0].knots.len(), 4);
    }

    fn two_squares() -> serde_json::Value {
        json!({"subpaths": [
            {"closed": true, "knots": [[10, 10], [30, 10], [30, 30], [10, 30]]},
            {"closed": true, "op": "subtract", "knots": [[50, 40], [70, 40], [70, 60], [50, 60]]}
        ]})
    }

    #[test]
    fn components_align_arrange_and_change_operation() {
        let mut s = session();
        s.execute("path.set", json!({"path": two_squares()})).unwrap();
        s.execute("path.alignComponents", json!({"edge": "top"})).unwrap();
        assert_eq!(work(&s).subpaths[1].knots[0].anchor.y, 10.0);
        s.execute("path.alignComponents", json!({"edge": "right", "to": "canvas", "subpaths": [0]})).unwrap();
        assert_eq!(work(&s).subpaths[0].knots[1].anchor.x, 120.0);
        let r = s.execute("path.arrangeComponents", json!({"subpaths": [1], "to": "back"})).unwrap();
        assert_eq!(r["subpaths"], json!([0]));
        assert_eq!(work(&s).subpaths[1].knots[1].anchor.x, 120.0);
        s.execute("path.setOperation", json!({"op": "intersect"})).unwrap();
        assert_eq!(work(&s).subpaths[1].op, prism_doc::PathOp::Intersect);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Path Operation"));
        // Merging two squares that don't overlap with "exclude": two separate outlines.
        s.execute("path.setOperation", json!({"op": "exclude", "subpaths": [1]})).unwrap();
        s.execute("path.mergeComponents", json!({})).unwrap();
        assert_eq!(work(&s).components().len(), 2);
        // Shape layers take the same edits.
        let id = s.execute("shape.create", json!({"kind": "path", "path": two_squares()})).unwrap()["layer"].as_u64().unwrap();
        s.execute("path.distributeComponents", json!({"name": "layer", "layer": id, "edge": "left"})).unwrap_err();
        s.execute("path.alignComponents", json!({"name": "layer", "layer": id, "edge": "left"})).unwrap();
        let LayerContentShape(sh) = shape(&s, id);
        assert_eq!(sh.path.subpaths[1].knots[0].anchor.x, 10.0);
    }

    struct LayerContentShape(ShapeLayer);
    fn shape(s: &Session, id: u64) -> LayerContentShape {
        let prism_doc::LayerContent::Shape(sh) = &s.active().unwrap().doc.layer(LayerId(id)).unwrap().content else { panic!("shape") };
        LayerContentShape(sh.clone())
    }

    #[test]
    fn make_shape_consumes_the_work_path_in_one_step() {
        let mut s = session();
        s.execute("path.set", json!({"path": square()})).unwrap();
        let layers = s.active().unwrap().doc.layers.len();
        let r = s.execute("path.makeShape", json!({})).unwrap();
        let st = s.active().unwrap();
        assert!(st.doc.work_path.is_none());
        assert_eq!(st.doc.layers.len(), layers + 1);
        assert_eq!(st.history.undo_label(), Some("New Shape Layer"));
        let LayerContentShape(sh) = shape(&s, r["layer"].as_u64().unwrap());
        assert!(sh.fill.is_some() && sh.stroke.is_none());
        assert!(s.undo());
        assert!(s.active().unwrap().doc.work_path.is_some(), "one undo restores the work path");
        // A saved path stays.
        s.execute("path.set", json!({"name": "Saved", "path": square()})).unwrap();
        s.execute("path.makeShape", json!({"name": "Saved", "fill": null, "stroke": {"width": 2}})).unwrap();
        assert!(s.active().unwrap().doc.paths.iter().any(|p| p.name == "Saved"));
    }

    #[test]
    fn bad_params_fail_without_an_edit() {
        let mut s = session();
        assert!(s.execute("path.moveAnchors", json!({"anchors": [[0, 0]], "move": [1, 1]})).is_err(), "no work path");
        s.execute("path.set", json!({"path": square()})).unwrap();
        let steps = s.active().unwrap().history.entries().len();
        for (id, p) in [
            ("path.moveAnchors", json!({"anchors": [[0, 9]], "move": [1, 1]})),
            ("path.moveAnchors", json!({"anchors": [], "move": [1, 1]})),
            ("path.moveAnchors", json!({"anchors": [[0]], "move": [1, 1]})),
            ("path.moveAnchors", json!({"anchors": [[0, 0]], "move": [1e300, 1]})),
            ("path.moveAnchors", json!({"anchors": [[0, 0]]})),
            ("path.moveHandle", json!({"subpath": 0, "knot": 0, "handle": "up", "to": [1, 1]})),
            ("path.moveHandle", json!({"subpath": 0, "knot": 0, "handle": "in"})),
            ("path.moveHandle", json!({"subpath": -1, "knot": 0, "handle": "in", "to": [1, 1]})),
            ("path.bendSegment", json!({"subpath": 0, "knot": 0, "t": 2, "move": [1, 1]})),
            ("path.bendSegment", json!({"subpath": 5, "knot": 0, "move": [1, 1]})),
            ("path.convertPoint", json!({"subpath": 0, "knot": 4})),
            ("path.convertPoint", json!({"subpath": 0, "knot": 0, "out": "x"})),
            ("path.convertPoint", json!({"name": "nope", "subpath": 0, "knot": 0})),
            ("path.convertPoint", json!({"name": "layer", "subpath": 0, "knot": 0})),
            ("path.addAnchor", json!({"subpath": 0, "knot": 9})),
            ("path.addAnchor", json!({"subpath": 0, "knot": 0, "t": -1})),
            ("path.deleteAnchor", json!({"subpath": 2, "knot": 0})),
            ("path.deleteAnchor", json!({"subpath": 0})),
            ("path.setOperation", json!({"op": "join"})),
            ("path.setOperation", json!({"op": "combine", "subpaths": [4]})),
            ("path.alignComponents", json!({"edge": "middle"})),
            ("path.alignComponents", json!({"edge": "left", "to": "page"})),
            ("path.alignComponents", json!({"edge": "left", "subpaths": []})),
            ("path.alignComponents", json!({"edge": "left", "subpaths": ["a"]})),
            ("path.distributeComponents", json!({"edge": "left"})),
            ("path.arrangeComponents", json!({"to": "front"})),
            ("path.arrangeComponents", json!({"to": "up", "subpaths": [0]})),
            ("path.makeShape", json!({"name": "nope"})),
            ("path.makeShape", json!({"fill": "not a colour"})),
        ] {
            assert!(s.execute(id, p.clone()).is_err(), "{id} {p}");
        }
        assert_eq!(s.active().unwrap().history.entries().len(), steps);
    }
}
