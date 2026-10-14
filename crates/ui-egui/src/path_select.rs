//! Path Selection tool (A), as in Photoshop: it picks whole path components, where Direct
//! Selection picks anchors. It works on the paths the canvas shows: the targeted vector mask or
//! the active shape layer's path, and the work path.
//!
//! * Click a component (its outline, or inside its filled area) to select it, ⇧-click to add or
//!   remove one, click empty canvas to deselect. A drag on empty canvas selects the anchors in
//!   the marquee, as Direct Selection does; dragging one of them moves just those.
//! * Drag a component to move the selected ones (⌥: copies of them). ⌘/Ctrl turns the tool into
//!   Direct Selection while held (its white arrow shows), and a ⌘-press on an anchor switches to
//!   the Direct Selection tool; with Direct Selection, ⌘ is Path Selection.
//! * ⌫ / Delete deletes the selected components, the arrow keys nudge them (⇧: 10 px).
//!
//! The selection is Direct Selection's (every anchor of a selected component), so the options
//! bar's Path Operations, Alignment and Arrangement act on it. A drag previews and commits once
//! on release (`path.moveAnchors`, `path.duplicateComponents`, or `shape.edit` `move`, which
//! keeps a live shape live): one history step, and the same commands drive it over MCP.

use std::ops::Range;

use egui::Modifiers;
use prism_doc::LayerContent;
use prism_doc::vector::Path;
use prism_geom::Point;
use prism_vector::edit::{self, Hit};
use serde_json::{Value, json};

use crate::PrismApp;
use crate::direct_select::{DirectSelection, Drag, PathRef, candidates};
use crate::state::Tool;

/// Is Path Selection what the pointer does now: its own tool (⌘/Ctrl makes it Direct
/// Selection), or Direct Selection with ⌘/Ctrl held, as in Photoshop.
pub(crate) fn applies(tool: Tool, mods: Modifiers) -> bool {
    (tool == Tool::PathSelection && !mods.command) || (tool == Tool::DirectSelection && mods.command)
}

/// Whether `p` is inside closed subpath `s` (even-odd, on its flattened outline).
fn inside(path: &Path, s: usize, p: [f64; 2]) -> bool {
    let Some(sp) = path.subpaths.get(s).filter(|sp| sp.closed && sp.knots.len() >= 3) else { return false };
    let pts = prism_vector::flatten_subpath(sp, 0.25).pts;
    let mut inside = false;
    let n = pts.len();
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        if (a.1 > p[1]) != (b.1 > p[1]) && p[0] < a.0 + (p[1] - a.1) / (b.1 - a.1) * (b.0 - a.0) {
            inside = !inside;
        }
    }
    inside
}

/// Whether `p` is in the area component `c` fills: inside its outline and not in a hole.
fn inside_component(path: &Path, c: &Range<usize>, p: [f64; 2]) -> bool {
    c.clone().filter(|&s| inside(path, s, p)).count() % 2 == 1
}

/// The component a click at `p` picks: under the pointer on an outline, else the topmost one
/// whose area holds it, on the frontmost path that has one. With the path and the component's
/// subpath range.
pub(crate) fn component_at(app: &PrismApp, p: [f64; 2]) -> Option<(PathRef, Path, Range<usize>)> {
    let tol = 6.0 / f64::from(app.current_zoom().max(0.01));
    candidates(app).into_iter().find_map(|(r, path)| {
        let comps = path.components();
        let on = edit::hit(&path, Point::new(p[0], p[1]), tol, &[]).and_then(|h| {
            let (Hit::Anchor([s, _]) | Hit::Segment([s, _], _) | Hit::Handle([s, _], _)) = h;
            comps.iter().find(|c| c.contains(&s)).cloned()
        });
        let range = on.or_else(|| comps.iter().rev().find(|c| inside_component(&path, c, p)).cloned())?;
        Some((r, path, range))
    })
}

fn anchors_of(path: &Path, subs: impl IntoIterator<Item = usize>) -> Vec<[usize; 2]> {
    subs.into_iter().flat_map(|s| edit::subpath_anchors(path, s)).collect()
}

/// The subpaths holding selected anchors, in order.
fn selected_subpaths(ds: &DirectSelection) -> Vec<usize> {
    let mut v: Vec<usize> = ds.anchors.iter().map(|a| a[0]).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Whether layer `target` is a shape layer (its moves keep whole pixels and, when the whole
/// path moves, the live shape).
fn is_shape(app: &PrismApp, target: PathRef) -> bool {
    let PathRef::Layer(id) = target else { return false };
    let Some(st) = app.session.active() else { return false };
    st.doc.layer(prism_doc::LayerId(id)).is_some_and(|l| matches!(l.content, LayerContent::Shape(_))) && crate::vector_ui::targeted_vector_mask(app).is_none()
}

/// A press: select the component under the pointer and start dragging the selection, or start
/// a marquee on empty canvas.
pub(crate) fn press(app: &mut PrismApp, p: [f64; 2], mods: Modifiers) -> bool {
    // A press on an anchor of a partial selection (a marquee's) drags those anchors, as Direct
    // Selection would.
    if !mods.shift && !mods.alt && partial_anchor_drag(app, p) {
        return true;
    }
    let found = component_at(app, p);
    let whole_px = found.as_ref().is_some_and(|(r, ..)| is_shape(app, *r));
    let ds = &mut app.ui.direct_selection;
    let Some((target, base, range)) = found else {
        // On empty canvas: a marquee, which picks anchors (Direct Selection's).
        ds.drag = Some(Drag::Marquee { start: p, end: p, add: mods.shift });
        return true;
    };
    if ds.target != Some(target) {
        *ds = DirectSelection { target: Some(target), ..Default::default() };
    }
    ds.segment = None;
    let mine = anchors_of(&base, range);
    let selected = !mine.is_empty() && mine.iter().all(|a| ds.anchors.contains(a));
    if mods.shift {
        // ⇧-click toggles the component; one that deselects drags nothing.
        if selected {
            ds.anchors.retain(|a| !mine.contains(a));
            return true;
        }
        for a in mine {
            if !ds.anchors.contains(&a) {
                ds.anchors.push(a);
            }
        }
    } else if !selected {
        ds.anchors = mine;
    }
    let subs = selected_subpaths(ds);
    ds.drag = Some(Drag::Components { target, base, subs, start: p, end: p, duplicate: mods.alt, whole_px, preview: None });
    true
}

/// A press at `p` on a selected anchor of a component that is only partly selected starts
/// dragging the selected anchors. Whether it did.
fn partial_anchor_drag(app: &mut PrismApp, p: [f64; 2]) -> bool {
    let tol = 6.0 / f64::from(app.current_zoom().max(0.01));
    let ds = &app.ui.direct_selection;
    let Some(target) = ds.target else { return false };
    let Some((_, base)) = candidates(app).into_iter().find(|(r, _)| *r == target) else { return false };
    let Some(Hit::Anchor(k)) = edit::hit(&base, Point::new(p[0], p[1]), tol, &[]) else { return false };
    let Some(comp) = base.components().into_iter().find(|c| c.contains(&k[0])) else { return false };
    let whole = anchors_of(&base, comp).iter().all(|a| ds.anchors.contains(a));
    if !ds.anchors.contains(&k) || whole {
        return false;
    }
    let edit = crate::direct_select::Edit::Anchors(ds.anchors.clone());
    app.ui.direct_selection.drag = Some(Drag::Edit { target, base, edit, start: p, end: p, preview: None });
    true
}

/// The offset of a drag from `start` to `end` (whole pixels on a shape layer).
fn offset(start: [f64; 2], end: [f64; 2], whole_px: bool) -> [f64; 2] {
    let d = [end[0] - start[0], end[1] - start[1]];
    if whole_px { d.map(f64::round) } else { d }
}

/// `base` as a component drag leaves it: the selected subpaths `subs` moved, or copies of them
/// added and moved (⌥).
pub(crate) fn preview(base: &Path, subs: &[usize], start: [f64; 2], end: [f64; 2], duplicate: bool, whole_px: bool) -> Option<Path> {
    let d = offset(start, end, whole_px);
    let mut p = base.clone();
    if duplicate {
        edit::duplicate_components(&mut p, subs, d).ok()?;
    } else {
        edit::move_anchors(&mut p, &anchors_of(base, subs.iter().copied()), d).ok()?;
    }
    Some(p)
}

fn report(app: &mut PrismApp, r: Result<Value, String>) -> Option<Value> {
    r.map_err(|e| {
        app.ui.status = e;
        app.ui.status_error = true;
    })
    .ok()
}

/// Runs a path command on `target` with `extra` params.
fn run_on(app: &mut PrismApp, id: &str, target: PathRef, extra: Value) -> Option<Value> {
    let mut q = target.params();
    if let (Some(q), Some(extra)) = (q.as_object_mut(), extra.as_object()) {
        q.extend(extra.clone());
    }
    let r = app.run(id, q);
    report(app, r)
}

/// Moves the selected components of `target` by `d` (one history step): a whole shape layer
/// path by `shape.edit` (it stays a live shape), anything else by its anchors.
fn move_selection(app: &mut PrismApp, target: PathRef, base: &Path, subs: &[usize], d: [f64; 2]) {
    if d == [0.0, 0.0] || subs.is_empty() {
        return;
    }
    let all = subs.len() == base.subpaths.len();
    match target {
        PathRef::Layer(id) if all && is_shape(app, target) => {
            let r = app.run("shape.edit", json!({"layer": id, "move": d}));
            report(app, r);
        }
        _ => {
            run_on(app, "path.moveAnchors", target, json!({"anchors": anchors_of(base, subs.iter().copied()), "move": d}));
        }
    }
}

/// End of a component drag: commit it. A click (no movement) only selected.
#[allow(clippy::too_many_arguments)]
pub(crate) fn release(app: &mut PrismApp, target: PathRef, base: &Path, subs: &[usize], start: [f64; 2], end: [f64; 2], duplicate: bool, whole_px: bool) {
    let moved = (end[0] - start[0]).hypot(end[1] - start[1]) * f64::from(app.current_zoom()) >= 1.0;
    if !moved {
        return;
    }
    let d = offset(start, end, whole_px);
    if !duplicate {
        move_selection(app, target, base, subs, d);
        return;
    }
    // ⌥-drag: the copies, moved, are what's selected afterwards.
    let Some(r) = run_on(app, "path.duplicateComponents", target, json!({"subpaths": subs, "move": d})) else { return };
    let copies: Vec<usize> = r["subpaths"].as_array().map(|a| a.iter().filter_map(|v| v.as_u64().and_then(|v| usize::try_from(v).ok())).collect()).unwrap_or_default();
    if let Some((_, path)) = candidates(app).into_iter().find(|(r, _)| *r == target) {
        app.ui.direct_selection = DirectSelection { target: Some(target), anchors: anchors_of(&path, copies), ..Default::default() };
    }
}

/// The selected components, if any: their path and its subpaths.
fn selection(app: &PrismApp) -> Option<(PathRef, Path, Vec<usize>)> {
    let ds = &app.ui.direct_selection;
    let target = ds.target?;
    let (_, path) = candidates(app).into_iter().find(|(r, _)| *r == target)?;
    let subs: Vec<usize> = selected_subpaths(ds).into_iter().filter(|&s| s < path.subpaths.len()).collect();
    (!subs.is_empty()).then_some((target, path, subs))
}

/// Path Selection's keys while components are selected: ⌫ / Delete deletes them, the arrow keys
/// nudge them (⇧: 10 px). Whether a key was used.
pub(crate) fn keys(app: &mut PrismApp, ctx: &egui::Context) -> bool {
    use egui::Key;
    if app.ui.tool != Tool::PathSelection || app.ui.direct_selection.drag.is_some() {
        return false;
    }
    let Some((target, path, subs)) = selection(app) else { return false };
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Backspace) || i.consume_key(Modifiers::NONE, Key::Delete)) {
        if run_on(app, "path.deleteComponents", target, json!({"subpaths": subs})).is_some() {
            app.ui.direct_selection = DirectSelection::default();
        }
        return true;
    }
    for (key, ux, uy) in [(Key::ArrowLeft, -1.0, 0.0), (Key::ArrowRight, 1.0, 0.0), (Key::ArrowUp, 0.0, -1.0), (Key::ArrowDown, 0.0, 1.0)] {
        // ⇧ first: `consume_key` ignores an extra ⇧.
        for mods in [Modifiers::SHIFT, Modifiers::NONE] {
            if ctx.input_mut(|i| i.consume_key(mods, key)) {
                let k = if mods.shift { 10.0 } else { 1.0 };
                move_selection(app, target, &path, &subs, [ux * k, uy * k]);
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
#[path = "path_select_tests.rs"]
mod tests;
