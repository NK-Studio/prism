//! Direct Selection tool (A, #790), and the Pen's ⌘/Ctrl (Direct Selection) and ⌥/Alt (Convert
//! Point) modifiers, as in Photoshop. It edits the paths the canvas shows: the targeted vector
//! mask or the active shape layer's path, and the work path.
//!
//! * Click an anchor to select it (⇧ adds or removes, ⌥ selects its whole subpath), drag on empty
//!   canvas to marquee-select anchors, click empty canvas to deselect.
//! * Drag selected anchors to move them, a direction handle to turn it (a smooth point keeps its
//!   handles collinear; ⌥ breaks them apart), or a segment to bend it (straight segments move).
//! * Pen + ⌥: click an anchor to make it a corner, drag from it to pull out smooth handles, drag a
//!   handle to move it alone.
//!
//! A drag previews with `prism_vector::edit` and commits once on release through the engine
//! (`path.moveAnchors`, `path.moveHandle`, `path.bendSegment`, `path.convertPoint`): one history
//! step per drag, and the same commands drive it over MCP and the control channel.

use egui::{Color32, Modifiers, Pos2};
use prism_doc::vector::Path;
use prism_geom::Point;
use prism_vector::edit::{self, Handle, Hit};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PrismApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

/// A path Direct Selection edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathRef {
    Work,
    /// A layer's shape path, or its vector mask.
    Layer(u64),
}

impl PathRef {
    /// The engine's target params for this path.
    pub(crate) fn params(self) -> Value {
        match self {
            PathRef::Work => json!({"name": "work"}),
            PathRef::Layer(id) => json!({"name": "layer", "layer": id}),
        }
    }
}

/// Direct Selection state: which anchors are selected and the drag in progress.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DirectSelection {
    /// The path whose anchors are selected.
    pub target: Option<PathRef>,
    /// Selected anchors as `[subpath, knot]`.
    pub anchors: Vec<[usize; 2]>,
    /// A clicked segment (`[subpath, knot]` it leaves): its two handles show.
    pub segment: Option<[usize; 2]>,
    /// The subpaths of a component made active by a click inside it (the Pen with ⌘, or Direct
    /// Selection): its anchors show, none selected, as in Photoshop.
    pub shown: Vec<usize>,
    #[serde(skip)]
    pub drag: Option<Drag>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Drag {
    /// Editing `target`: `base` is its path at the press, `preview` the edit so far.
    Edit { target: PathRef, base: Path, edit: Edit, start: [f64; 2], end: [f64; 2], preview: Option<Path> },
    /// Marquee-selecting anchors (⇧ adds to the selection), with either tool: Path Selection's
    /// marquee also picks anchors, as Direct Selection's does (as in Photoshop).
    Marquee { start: [f64; 2], end: [f64; 2], add: bool },
    /// Path Selection moving (⌥: copying) whole components: subpaths `subs` of `target`'s
    /// `base` path, moved by whole pixels on a shape layer (`whole_px`).
    Components { target: PathRef, base: Path, subs: Vec<usize>, start: [f64; 2], end: [f64; 2], duplicate: bool, whole_px: bool, preview: Option<Path> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Anchors(Vec<[usize; 2]>),
    /// A handle, where it was at the press, and whether it moves alone (⌥).
    Handle([usize; 2], Handle, Point, bool),
    Segment([usize; 2], f64),
    /// Pen + ⌥ on an anchor: a click makes a corner, a drag pulls out smooth handles.
    Convert([usize; 2]),
}

impl Edit {
    /// The edit for a drag from `start` to `end`, as a command and its params (without target).
    fn command(&self, start: [f64; 2], end: [f64; 2]) -> (&'static str, Value) {
        let d = [end[0] - start[0], end[1] - start[1]];
        match self {
            Edit::Anchors(a) => ("path.moveAnchors", json!({"anchors": a, "move": d})),
            Edit::Handle([s, k], h, from, independent) => {
                ("path.moveHandle", json!({"subpath": s, "knot": k, "handle": h.name(), "to": [from.x + d[0], from.y + d[1]], "independent": independent}))
            }
            Edit::Segment([s, k], t) => ("path.bendSegment", json!({"subpath": s, "knot": k, "t": t, "move": d})),
            Edit::Convert([s, k]) => ("path.convertPoint", json!({"subpath": s, "knot": k, "out": end})),
        }
    }

    /// `base` edited as the command would: the drag preview.
    fn preview(&self, base: &Path, start: [f64; 2], end: [f64; 2]) -> Option<Path> {
        let d = [end[0] - start[0], end[1] - start[1]];
        let mut p = base.clone();
        match self {
            Edit::Anchors(a) => edit::move_anchors(&mut p, a, d),
            Edit::Handle(k, h, from, independent) => edit::move_handle(&mut p, *k, *h, Point::new(from.x + d[0], from.y + d[1]), *independent),
            Edit::Segment(k, t) => edit::bend_segment(&mut p, *k, *t, d),
            Edit::Convert(k) => edit::convert_point(&mut p, *k, Some(Point::new(end[0], end[1]))),
        }
        .ok()?;
        Some(p)
    }
}

/// Is Direct Selection what the pointer does now: its own tool, or the Pen or Path Selection
/// with ⌘/Ctrl held (Direct Selection with ⌘ is Path Selection), as in Photoshop.
fn direct(tool: Tool, mods: Modifiers) -> bool {
    (tool == Tool::DirectSelection && !mods.command) || (matches!(tool, Tool::Pen | Tool::PathSelection) && mods.command)
}

/// Whether the path tools' pointer is Direct Selection's white arrow (else Path Selection's
/// black one): what a press does now, or what the drag under way does.
pub(crate) fn direct_cursor(app: &PrismApp, mods: Modifiers) -> bool {
    match &app.ui.direct_selection.drag {
        // A Pen + ⌥ drag keeps Convert Point's cursor.
        Some(Drag::Components { .. } | Drag::Edit { edit: Edit::Convert(_), .. }) => false,
        Some(Drag::Edit { .. }) => true,
        _ => direct(app.ui.tool, mods),
    }
}

/// Does the canvas draw this overlay (Direct or Path Selection's) instead of the plain path
/// outlines?
pub fn shows(app: &PrismApp, mods: Modifiers) -> bool {
    // The Pen shows the paths' anchors too, as in Photoshop: what Auto Add/Delete, ⌥ (Convert
    // Point) and ⌘ act on.
    direct(app.ui.tool, mods)
        || matches!(app.ui.tool, Tool::PathSelection | Tool::DirectSelection | Tool::Pen)
        || app.ui.direct_selection.drag.is_some()
}

/// The direction handles the overlay draws on `path`: those of the selection (none for Path
/// Selection's whole components), and both of an anchor being converted (Pen + ⌥ drag), which
/// show while they are pulled out.
fn overlay_handles(ds: &DirectSelection, r: PathRef, path: &Path, whole: bool) -> Vec<([usize; 2], Handle)> {
    let mut out = if ds.target == Some(r) && !whole { shown_handles(ds, path) } else { Vec::new() };
    if let Some(Drag::Edit { target, edit: Edit::Convert(k), .. }) = &ds.drag
        && *target == r
    {
        out.extend([(*k, Handle::In), (*k, Handle::Out)]);
    }
    let knot = |r: [usize; 2]| path.subpaths.get(r[0]).and_then(|s| s.knots.get(r[1]));
    out.retain(|&(k, h)| knot(k).is_some_and(|kn| !retracted(kn, h)));
    out
}

/// The paths Direct Selection edits, front first: the targeted vector mask or the active shape
/// layer's path, then the work path.
pub(crate) fn candidates(app: &PrismApp) -> Vec<(PathRef, Path)> {
    let layer = crate::vector_ui::targeted_vector_mask(app).or_else(|| crate::vector_ui::active_shape_path(app));
    // A work path hidden with the Pen's ↩ isn't edited until it shows again.
    let work = crate::vector_ui::shown_work_path(app);
    layer.map(|(id, p)| (PathRef::Layer(id), p)).into_iter().chain(work.map(|p| (PathRef::Work, p))).collect()
}

fn retracted(k: &prism_doc::Knot, h: Handle) -> bool {
    let c = if h == Handle::In { k.in_ctrl } else { k.out_ctrl };
    (c.x - k.anchor.x).abs() < 1e-9 && (c.y - k.anchor.y).abs() < 1e-9
}

/// The handles shown on `path`: both of every selected anchor and the two of a clicked segment
/// (retracted handles sit on their anchor and don't show).
fn shown_handles(ds: &DirectSelection, path: &Path) -> Vec<([usize; 2], Handle)> {
    let knot = |r: [usize; 2]| path.subpaths.get(r[0]).and_then(|s| s.knots.get(r[1]));
    let mut out: Vec<([usize; 2], Handle)> = ds.anchors.iter().flat_map(|&r| [(r, Handle::In), (r, Handle::Out)]).collect();
    if let Some([s, k]) = ds.segment
        && let Some(sp) = path.subpaths.get(s)
    {
        let n = sp.knots.len();
        let next = k.checked_add(1).filter(|&j| j < n).or((sp.closed && n > 1).then_some(0));
        out.push(([s, k], Handle::Out));
        out.extend(next.map(|j| ([s, j], Handle::In)));
    }
    out.retain(|&(r, h)| knot(r).is_some_and(|k| !retracted(k, h)));
    out
}

/// Canvas pointer events for Direct Selection and the Pen's ⌘ / ⌥ modes; false when the event is
/// not theirs.
pub fn pointer(app: &mut PrismApp, ev: ToolEvent, mods: Modifiers) -> bool {
    let tool = app.ui.tool;
    match ev {
        ToolEvent::Down { x, y, .. } => {
            if crate::path_select::applies(tool, mods) {
                return crate::path_select::press(app, [x, y], mods);
            }
            let pen_idle = app.ui.pen.is_none();
            // Pen + ⌥ is Convert Point only while no path is being drawn.
            let convert = tool == Tool::Pen && mods.alt && !mods.command && pen_idle;
            if !(direct(tool, mods) || convert) {
                return false;
            }
            if tool == Tool::Pen && !pen_idle {
                // ⌘-click ends the path being drawn, which becomes editable, as in Photoshop.
                crate::vector_ui::pen_commit(app, false);
            }
            // Path Selection or the Pen with ⌘: a press on an anchor selects just that one, even
            // within a selected component (⇧ still adds), as in Photoshop.
            if matches!(tool, Tool::PathSelection | Tool::Pen) && mods.command && !mods.shift && !mods.alt {
                pick_only_anchor(app, [x, y]);
            }
            let used = press(app, [x, y], mods, convert);
            // ... and switches to the Direct Selection tool for good (its drag carries on).
            if tool == Tool::PathSelection && matches!(&app.ui.direct_selection.drag, Some(Drag::Edit { edit: Edit::Anchors(_), .. })) {
                app.ui.tool = Tool::DirectSelection;
            }
            used
        }
        ToolEvent::Move { x, y, .. } => {
            let ds = &mut app.ui.direct_selection;
            match &mut ds.drag {
                Some(Drag::Edit { base, edit, start, end, preview, .. }) => {
                    *end = [x, y];
                    *preview = edit.preview(base, *start, *end);
                }
                Some(Drag::Components { base, subs, start, end, duplicate, whole_px, preview, .. }) => {
                    *end = [x, y];
                    *preview = crate::path_select::preview(base, subs, *start, *end, *duplicate, *whole_px);
                }
                Some(Drag::Marquee { end, .. }) => *end = [x, y],
                None => return matches!(tool, Tool::DirectSelection | Tool::PathSelection),
            }
            true
        }
        ToolEvent::Up { x, y } => match app.ui.direct_selection.drag.take() {
            Some(Drag::Edit { target, edit, start, .. }) => {
                release(app, target, &edit, start, [x, y]);
                true
            }
            Some(Drag::Components { target, base, subs, start, duplicate, whole_px, .. }) => {
                crate::path_select::release(app, target, &base, &subs, start, [x, y], duplicate, whole_px);
                true
            }
            Some(Drag::Marquee { start, add, .. }) => {
                marquee(app, start, [x, y], add);
                true
            }
            None => matches!(tool, Tool::DirectSelection | Tool::PathSelection),
        },
    }
}

/// If an anchor is under `p`, makes it the only one selected (its path the target).
fn pick_only_anchor(app: &mut PrismApp, p: [f64; 2]) {
    let tol = 6.0 / f64::from(app.current_zoom().max(0.01));
    let hit = candidates(app).into_iter().find_map(|(r, path)| match edit::hit(&path, Point::new(p[0], p[1]), tol, &[])? {
        Hit::Anchor(k) => Some((r, k)),
        _ => None,
    });
    if let Some((r, k)) = hit {
        app.ui.direct_selection = DirectSelection { target: Some(r), anchors: vec![k], ..Default::default() };
    }
}

/// A press: pick what's under the pointer and start a drag. False lets the Pen handle a ⌥-press
/// that hit nothing.
fn press(app: &mut PrismApp, p: [f64; 2], mods: Modifiers, convert: bool) -> bool {
    let tol = 6.0 / f64::from(app.current_zoom().max(0.01));
    let ds = &app.ui.direct_selection;
    let found = candidates(app).into_iter().find_map(|(r, path)| {
        let handles = if ds.target == Some(r) { shown_handles(ds, &path) } else { Vec::new() };
        edit::hit(&path, Point::new(p[0], p[1]), tol, &handles).map(|h| (r, path, h))
    });
    let ds = &mut app.ui.direct_selection;
    let Some((target, base, hit)) = found else {
        if convert {
            return false;
        }
        ds.drag = Some(Drag::Marquee { start: p, end: p, add: mods.shift });
        return true;
    };
    if ds.target != Some(target) {
        *ds = DirectSelection { target: Some(target), ..Default::default() };
    }
    let edit = match hit {
        Hit::Handle(k, h) => {
            let kn = base.subpaths.get(k[0]).and_then(|s| s.knots.get(k[1]));
            let from = kn.map_or(Point::new(p[0], p[1]), |kn| if h == Handle::In { kn.in_ctrl } else { kn.out_ctrl });
            Edit::Handle(k, h, from, mods.alt)
        }
        Hit::Anchor(k) if convert => Edit::Convert(k),
        // Pen + ⌥ on a segment does nothing.
        Hit::Segment(..) if convert => return true,
        Hit::Anchor(k) => {
            ds.segment = None;
            if mods.alt {
                ds.anchors = edit::subpath_anchors(&base, k[0]);
            } else if mods.shift {
                // ⇧-click toggles the anchor; a click that deselects drags nothing.
                if let Some(i) = ds.anchors.iter().position(|a| *a == k) {
                    ds.anchors.remove(i);
                    return true;
                }
                ds.anchors.push(k);
            } else if !ds.anchors.contains(&k) {
                ds.anchors = vec![k];
            }
            Edit::Anchors(ds.anchors.clone())
        }
        Hit::Segment(k, _) if mods.alt => {
            ds.segment = None;
            ds.anchors = edit::subpath_anchors(&base, k[0]);
            Edit::Anchors(ds.anchors.clone())
        }
        Hit::Segment(k, t) => {
            ds.anchors.clear();
            ds.segment = Some(k);
            Edit::Segment(k, t)
        }
    };
    ds.drag = Some(Drag::Edit { target, base, edit, start: p, end: p, preview: None });
    true
}

/// End of an edit drag: commit it as one command. A click (no movement) commits nothing, except
/// Pen + ⌥ on an anchor, which makes it a corner.
fn release(app: &mut PrismApp, target: PathRef, edit: &Edit, start: [f64; 2], end: [f64; 2]) {
    let moved = (end[0] - start[0]).hypot(end[1] - start[1]) * f64::from(app.current_zoom()) >= 1.0;
    let (id, mut params) = match edit {
        Edit::Convert([s, k]) if !moved => ("path.convertPoint", json!({"subpath": s, "knot": k})),
        _ if !moved => return,
        e => e.command(start, end),
    };
    if let (Some(p), Some(t)) = (params.as_object_mut(), target.params().as_object()) {
        p.extend(t.clone());
    }
    match app.run(id, params) {
        // A converted anchor joins the selection, so the handles just pulled out keep showing
        // and can be dragged on; the rest of the selection stays, as in Photoshop (on another
        // path, that path's anchor is selected instead).
        Ok(_) => {
            if let Edit::Convert(k) = edit {
                let ds = &mut app.ui.direct_selection;
                if ds.target != Some(target) {
                    *ds = DirectSelection { target: Some(target), ..Default::default() };
                }
                if !ds.anchors.contains(k) {
                    ds.anchors.push(*k);
                }
            }
        }
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// End of a marquee: select the anchors inside it on the frontmost path that has any (⇧ adds).
/// A click on empty canvas deselects.
fn marquee(app: &mut PrismApp, start: [f64; 2], end: [f64; 2], add: bool) {
    let rect = [start[0], start[1], end[0], end[1]];
    let hits = candidates(app).into_iter().map(|(r, path)| (r, edit::anchors_in(&path, rect))).find(|(_, a)| !a.is_empty());
    // A click (no drag) inside a component makes it the active one: its anchors show, none
    // selected (Photoshop's ⌘-click with the Pen, or a click with Direct Selection).
    let click = (end[0] - start[0]).hypot(end[1] - start[1]) * f64::from(app.current_zoom()) < 2.0;
    let inside = if click && hits.is_none() && !add { crate::path_select::component_at(app, start) } else { None };
    let ds = &mut app.ui.direct_selection;
    match hits {
        Some((r, anchors)) if add && ds.target == Some(r) => {
            for a in anchors {
                if !ds.anchors.contains(&a) {
                    ds.anchors.push(a);
                }
            }
            ds.segment = None;
        }
        Some((r, anchors)) => *ds = DirectSelection { target: Some(r), anchors, ..Default::default() },
        None if add => {}
        None => match inside {
            Some((r, _, range)) => *ds = DirectSelection { target: Some(r), shown: range.collect(), ..Default::default() },
            None => {
                ds.anchors.clear();
                ds.segment = None;
                ds.shown.clear();
            }
        },
    }
}

/// The components of `path` (target `r`) whose anchors show: those holding selected anchors,
/// the clicked segment, or the active component. The rest show their outline only.
fn visible_components(ds: &DirectSelection, r: PathRef, path: &Path) -> Vec<std::ops::Range<usize>> {
    if ds.target != Some(r) {
        return Vec::new();
    }
    let subs: Vec<usize> = ds.anchors.iter().map(|a| a[0]).chain(ds.segment.map(|k| k[0])).chain(ds.shown.iter().copied()).collect();
    path.components().into_iter().filter(|c| subs.iter().any(|s| c.contains(s))).collect()
}

/// Paths with their anchors (selected ones filled), the shown handles, the drag's preview in
/// place of its path, and the marquee. `paths` false (View › Show › Target Path off) leaves only
/// the marquee. Path Selection shows only its selected components' anchors and no handles.
/// Under the pointer (no drag), Path Selection thickens the component a click would pick and
/// Direct Selection the anchor, or the subpath of the segment or handle.
pub fn draw_overlay(app: &PrismApp, painter: &egui::Painter, to_scr: &dyn Fn([f64; 2]) -> Pos2, accent: Color32, paths: bool) {
    let ds = &app.ui.direct_selection;
    let mods = painter.ctx().input(|i| i.modifiers);
    let whole = crate::path_select::applies(app.ui.tool, mods);
    // No hover feedback while the Pen draws a path (a press adds to it then).
    let drawing = app.ui.tool == Tool::Pen && app.ui.pen.is_some() && !mods.command;
    let hover = app.hover_doc.filter(|_| ds.drag.is_none() && paths && !drawing);
    let hovered_component = hover.filter(|_| whole).and_then(|p| crate::path_select::component_at(app, p)).map(|(r, _, range)| (r, range));
    let tol = 6.0 / f64::from(app.current_zoom().max(0.01));
    for (r, path) in if paths { candidates(app) } else { Vec::new() } {
        let path = match &ds.drag {
            Some(Drag::Edit { target, preview: Some(p), .. } | Drag::Components { target, preview: Some(p), .. }) if *target == r => p.clone(),
            _ => path,
        };
        crate::vector_ui::draw_outline(painter, &path, to_scr, accent);
        let ours = ds.target == Some(r);
        if whole {
            if let Some((_, range)) = hovered_component.as_ref().filter(|(h, _)| *h == r) {
                draw_subpaths(painter, &path, range.clone(), to_scr, accent);
            }
        } else if let Some(hit) = hover.and_then(|p| edit::hit(&path, Point::new(p[0], p[1]), tol, &if ours { shown_handles(ds, &path) } else { Vec::new() })) {
            match hit {
                Hit::Anchor([s, k]) => {
                    if let Some(kn) = path.subpaths.get(s).and_then(|sp| sp.knots.get(k)) {
                        let c = to_scr([kn.anchor.x, kn.anchor.y]);
                        painter.rect_stroke(egui::Rect::from_center_size(c, egui::vec2(10.0, 10.0)), 0.0, egui::Stroke::new(1.5, accent), egui::StrokeKind::Inside);
                    }
                }
                Hit::Segment([s, _], _) | Hit::Handle([s, _], _) => draw_subpaths(painter, &path, s..s + 1, to_scr, accent),
            }
        }
        for &(k, h) in &overlay_handles(ds, r, &path, whole) {
            if let Some(kn) = path.subpaths.get(k[0]).and_then(|s| s.knots.get(k[1])) {
                let c = if h == Handle::In { kn.in_ctrl } else { kn.out_ctrl };
                crate::vector_ui::draw_handle(painter, to_scr([kn.anchor.x, kn.anchor.y]), to_scr([c.x, c.y]), accent);
            }
        }
        // Only the selected or active components show anchors (solid when selected), as in
        // Photoshop; the others show their outline.
        let touched = visible_components(ds, r, &path);
        for (s, sp) in path.subpaths.iter().enumerate() {
            for (k, kn) in sp.knots.iter().enumerate() {
                let selected = ours && ds.anchors.contains(&[s, k]);
                // A copy being ⌥-dragged shows selected, like the components it copies.
                let copy = matches!(&ds.drag, Some(Drag::Components { target, duplicate: true, base, .. }) if *target == r && s >= base.subpaths.len());
                if selected || copy || touched.iter().any(|c| c.contains(&s)) {
                    crate::vector_ui::draw_anchor(painter, to_scr([kn.anchor.x, kn.anchor.y]), selected || copy, accent);
                }
            }
        }
    }
    if let Some(Drag::Marquee { start, end, .. }) = &ds.drag {
        let pts = [*start, [end[0], start[1]], *end, [start[0], end[1]]].map(to_scr);
        crate::tool_feedback::draw_ants(painter, &pts, true);
    }
}

/// Subpaths `range` of `path` drawn thick: the hover feedback.
fn draw_subpaths(painter: &egui::Painter, path: &Path, range: std::ops::Range<usize>, to_scr: &dyn Fn([f64; 2]) -> Pos2, accent: Color32) {
    let part = Path { subpaths: range.filter_map(|s| path.subpaths.get(s).cloned()).collect(), ..path.clone() };
    for (pts, closed) in crate::vector_ui::path_lines(&part, to_scr) {
        let stroke = egui::Stroke::new(2.5, accent);
        painter.add(if closed { egui::Shape::closed_line(pts, stroke) } else { egui::Shape::line(pts, stroke) });
    }
}

#[cfg(test)]
#[path = "direct_select_tests.rs"]
mod tests;
