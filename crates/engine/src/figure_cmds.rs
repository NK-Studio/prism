//! 3D figure layers: CLIP STUDIO PAINT-style posable mannequins (`Layer::figure`), made from
//! Prism's standing figure or CLIP STUDIO pose materials, seen through an orbit camera and
//! rendered into the layer's pixels (`prism_algo::figure3d`). Every edit re-renders inside one
//! history step; drags send `"draft": true` (one sample per pixel) under one `coalesce` key and a
//! final call without it.
use crate::commands::{CommandSpec, layer_param};
use crate::{EngineError, Result, Session};
use prism_algo::figure3d::{self, RenderOptions};
use prism_doc::figure::{Figure, FigureScene, MAX_FIGURES, standing_figure};
use prism_io::sut::pose::{HandPose, Pose};
use prism_doc::{Document, Layer, LayerContent, LayerId};
use prism_geom::Rect;
use serde_json::{Value, json};

const NEW: &str = "layer.figure.new";
const ADD: &str = "layer.figure.addPose";
const REMOVE: &str = "layer.figure.removeFigure";
const CAMERA: &str = "layer.figure.setCamera";
const FRAME: &str = "layer.figure.frame";
const JOINT: &str = "layer.figure.setJoint";
const REACH: &str = "layer.figure.reachTo";
const RESET: &str = "layer.figure.resetPose";
const HAND: &str = "layer.figure.setHand";
const HAND_POSE: &str = "layer.figure.applyHandPose";
const POSE: &str = "layer.figure.applyPose";
const LOOK: &str = "layer.figure.setLook";
const RASTERIZE: &str = "layer.rasterize.figure";
/// Largest `.pose` / `.pep` file read.
const POSE_LIMIT: u64 = 16 << 20;
/// Figures are framed to this share of the canvas height.
const FRAME_SHARE: f64 = 0.8;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// The area a figure render may cover: the canvas grown by its own size on every side, so a
/// moved layer keeps the parts it carries off the canvas.
fn allowed(doc: &Document) -> Rect {
    let c = doc.bounds();
    let (w, h) = (i32::try_from(c.width()).unwrap_or(0), i32::try_from(c.height()).unwrap_or(0));
    Rect::new(c.x0.saturating_sub(w), c.y0.saturating_sub(h), c.x1.saturating_add(w), c.y1.saturating_add(h))
}

/// Re-render a figure layer's pixels from its scene.
pub(crate) fn render_into(doc: &Document, layer: &mut Layer, draft: bool) -> Result<()> {
    let Some(scene) = &layer.figure else { return Err(EngineError::Other("not a 3D figure layer".into())) };
    let surface = figure3d::render(scene, doc.pixel_format(), allowed(doc), RenderOptions { draft }, &prism_raster::Interrupt::NONE).map_err(EngineError::Other)?;
    layer.content = LayerContent::Raster(surface);
    layer.fill_cache = None;
    Ok(())
}

fn flag(p: &Value, key: &str, cmd: &str) -> Result<bool> {
    match p.get(key) {
        None => Ok(false),
        Some(v) => v.as_bool().ok_or_else(|| bad(cmd, format!("`{key}` must be a boolean"))),
    }
}

fn number(p: &Value, key: &str, cmd: &str, range: std::ops::RangeInclusive<f64>) -> Result<Option<f64>> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v.as_f64().filter(|v| v.is_finite() && range.contains(v)).map(Some).ok_or_else(|| bad(cmd, format!("`{key}` must be a number in {}..{}", range.start(), range.end()))),
    }
}

fn numbers<const N: usize>(p: &Value, key: &str, cmd: &str, limit: f64) -> Result<Option<[f64; N]>> {
    let Some(v) = p.get(key) else { return Ok(None) };
    let err = || bad(cmd, format!("`{key}` must be {N} finite numbers"));
    let a = v.as_array().filter(|a| a.len() == N).ok_or_else(err)?;
    let mut out = [0.0; N];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x.as_f64().filter(|x| x.is_finite() && x.abs() <= limit).ok_or_else(err)?;
    }
    Ok(Some(out))
}

fn text<'a>(p: &'a Value, key: &str, cmd: &str) -> Result<Option<&'a str>> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v.as_str().map(str::trim).filter(|s| !s.is_empty() && s.chars().count() <= 255).map(Some).ok_or_else(|| bad(cmd, format!("`{key}` must be a non-empty string"))),
    }
}

/// The figures a command's params name: CLIP STUDIO poses by material id (`uuid`/`uuids`, with
/// `root`), a `.pose`/`.pep` file (`path`/`data`), or none.
fn figures_from(p: &Value, cmd: &str) -> Result<Vec<Figure>> {
    poses_from(p, cmd)?
        .into_iter()
        .map(|pose| match pose {
            Pose::Body(f) => Ok(f),
            Pose::Hand(h) => Err(bad(cmd, format!("{} is a hand pose: apply it with {HAND_POSE}", h.name))),
        })
        .collect()
}

/// Whole figures and hand poses from pose ids, a file or base64 data (see [`figures_from`]).
fn poses_from(p: &Value, cmd: &str) -> Result<Vec<Pose>> {
    let ctl = prism_raster::Interrupt::NONE;
    let mut ids: Vec<&str> = Vec::new();
    if let Some(u) = text(p, "uuid", cmd)? {
        ids.push(u);
    }
    if let Some(v) = p.get("uuids") {
        let a = v.as_array().filter(|a| !a.is_empty() && a.len() <= MAX_FIGURES).ok_or_else(|| bad(cmd, format!("`uuids` must list 1..{MAX_FIGURES} pose ids")))?;
        for u in a {
            ids.push(u.as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| bad(cmd, "`uuids` must be pose id strings"))?);
        }
    }
    if !ids.is_empty() {
        if p.get("path").is_some() || p.get("data").is_some() {
            return Err(bad(cmd, "pass pose ids or a file, not both"));
        }
        let root = crate::csp_library_cmds::root(cmd, p)?;
        return ids.into_iter().map(|id| prism_io::sut::library::import_pose_any(&root, id, &ctl).map(|(_, f)| f).map_err(EngineError::Other)).collect();
    }
    if p.get("path").is_some() || p.get("data").is_some() {
        if p.get("root").is_some() {
            return Err(bad(cmd, "`root` goes with `uuid`"));
        }
        let (bytes, stem) = crate::preset_import_cmds::file_bytes_with_limit(p, cmd, ".pose or .pep file", POSE_LIMIT)?;
        let name = if stem.trim().is_empty() { "Pose" } else { stem.trim() };
        let mut pose = prism_io::sut::pose::read_any(&bytes, name, &ctl).map_err(|e| bad(cmd, e))?;
        if let Pose::Body(f) = &mut pose {
            f.source = Some(name.to_string());
        }
        return Ok(vec![pose]);
    }
    Ok(Vec::new())
}

fn canvas_centre(doc: &Document) -> [f64; 2] {
    let c = doc.bounds();
    [f64::from(c.x0 + c.x1) / 2.0, f64::from(c.y0 + c.y1) / 2.0]
}

fn new_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let name = text(p, "name", NEW)?.map(str::to_string);
    let at = numbers::<2>(p, "at", NEW, 1e7)?;
    let (mut figures, hands): (Vec<Figure>, Vec<HandPose>) = split_poses(poses_from(p, NEW)?);
    // A single pose names the layer; Prism's own figure gets "3D Figure N".
    let first = if figures.len() == 1 { figures.first().map(|f| f.name.clone()).unwrap_or_default() } else { String::new() };
    if figures.is_empty() {
        figures.push(standing_figure());
    }
    // Hand poses alone: Prism's standing figure with those hands.
    for f in &mut figures {
        apply_hands(f, &hands, None, [false; 5]).map_err(|e| bad(NEW, e))?;
    }
    let count = figures.len();
    let mut scene = FigureScene { figures, ..Default::default() };
    scene.validate().map_err(|e| bad(NEW, e))?;
    let doc = &st.doc;
    let height = f64::from(doc.bounds().height()).max(1.0) * FRAME_SHARE;
    figure3d::frame(&mut scene, at.unwrap_or_else(|| canvas_centre(doc)), height, true).map_err(|e| bad(NEW, e))?;
    let id = s.edit("New 3D Figure Layer", |doc, active| {
        let name = name.unwrap_or_else(|| if count == 1 && !first.is_empty() { first.clone() } else { doc.next_layer_name("3D Figure") });
        let mut l = Layer::raster(name, doc.pixel_format());
        l.figure = Some(scene);
        render_into(doc, &mut l, false)?;
        let id = doc.insert_above(*active, l);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layer": id.0, "figures": count}))
}

/// Run `f` on the scene of the figure layer named by `layer` (default: the active one), then
/// re-render, all as one history step named `label`.
fn edit_scene(s: &mut Session, p: &Value, cmd: &str, label: &str, f: impl FnOnce(&mut FigureScene) -> Result<()>) -> Result<Value> {
    let id = layer_param(s, p)?;
    let draft = flag(p, "draft", cmd)?;
    {
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let l = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        if l.figure.is_none() {
            return Err(bad(cmd, format!("\"{}\" is not a 3D figure layer", l.name)));
        }
    }
    s.edit(label, |doc, _| {
        let snapshot = doc.clone();
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let scene = l.figure.as_mut().ok_or_else(|| bad(cmd, "not a 3D figure layer"))?;
        f(scene)?;
        scene.validate().map_err(|e| bad(cmd, e))?;
        render_into(&snapshot, l, draft)
    })?;
    Ok(json!({"layer": id.0}))
}

fn figure_index(p: &Value, scene_len: usize, cmd: &str) -> Result<usize> {
    match p.get("figure") {
        None => Ok(0),
        Some(v) => v.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|i| *i < scene_len).ok_or_else(|| bad(cmd, format!("`figure` must be an index below {scene_len}"))),
    }
}

fn scene_len(s: &Session, p: &Value) -> Result<usize> {
    let id = layer_param(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    Ok(st.doc.layer(id).and_then(|l| l.figure.as_ref()).map_or(0, |f| f.figures.len()))
}

fn add_pose(s: &mut Session, p: &Value) -> Result<Value> {
    let figures = figures_from(p, ADD)?;
    if figures.is_empty() {
        return Err(bad(ADD, "pass `uuid`, `path` or `data`"));
    }
    edit_scene(s, p, ADD, "Add Pose", |scene| {
        if scene.figures.len() + figures.len() > MAX_FIGURES {
            return Err(bad(ADD, format!("a layer holds at most {MAX_FIGURES} figures")));
        }
        scene.figures.extend(figures);
        Ok(())
    })
}

fn remove_figure(s: &mut Session, p: &Value) -> Result<Value> {
    let n = scene_len(s, p)?;
    let i = figure_index(p, n, REMOVE)?;
    if p.get("figure").is_none() {
        return Err(bad(REMOVE, "`figure` is required"));
    }
    edit_scene(s, p, REMOVE, "Remove Figure", |scene| {
        if scene.figures.len() <= 1 {
            return Err(bad(REMOVE, "a 3D figure layer keeps at least one figure; delete the layer instead"));
        }
        if i < scene.figures.len() {
            scene.figures.remove(i);
        }
        Ok(())
    })
}

fn set_camera(s: &mut Session, p: &Value) -> Result<Value> {
    let yaw = number(p, "yaw", CAMERA, -1e6..=1e6)?;
    let pitch = number(p, "pitch", CAMERA, -89.0..=89.0)?;
    let distance = number(p, "distance", CAMERA, 1.0..=1e5)?;
    let fov = number(p, "fov", CAMERA, 5.0..=120.0)?;
    let target = numbers::<3>(p, "target", CAMERA, 1e6)?;
    let orbit = numbers::<2>(p, "orbitBy", CAMERA, 1e6)?;
    let pan = numbers::<2>(p, "panBy", CAMERA, 1e7)?;
    let dolly = number(p, "dollyBy", CAMERA, 1e-3..=1e3)?;
    edit_scene(s, p, CAMERA, "Move 3D Camera", |scene| {
        let c = &mut scene.camera;
        if let Some(v) = yaw {
            c.yaw = v.rem_euclid(360.0);
        }
        if let Some(v) = pitch {
            c.pitch = v;
        }
        if let Some(v) = distance {
            c.distance = v;
        }
        if let Some(v) = fov {
            c.fov_deg = v;
        }
        if let Some(v) = target {
            c.target = v;
        }
        if let Some([y, x]) = orbit {
            figure3d::orbit(scene, y, x);
        }
        if let Some(d) = dolly {
            figure3d::dolly(scene, d);
        }
        if let Some(d) = pan {
            figure3d::pan(scene, d);
        }
        Ok(())
    })
}

fn frame_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let face = flag(p, "faceFront", FRAME)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let (centre, height) = (canvas_centre(&st.doc), f64::from(st.doc.bounds().height()).max(1.0) * FRAME_SHARE);
    edit_scene(s, p, FRAME, "Frame 3D Figures", |scene| figure3d::frame(scene, centre, height, face).map_err(|e| bad(FRAME, e)))
}

/// What `rotateBy` turns a joint about.
enum Axis {
    World([f64; 3]),
    /// The camera's viewing direction: the joint turns in the picture plane.
    View,
    /// The joint's own bone (to its first child): a twist.
    Bone,
}

fn set_joint(s: &mut Session, p: &Value) -> Result<Value> {
    let n = scene_len(s, p)?;
    let i = figure_index(p, n, JOINT)?;
    let joint = text(p, "joint", JOINT)?.ok_or_else(|| bad(JOINT, "`joint` is required"))?.to_string();
    let rotation = numbers::<4>(p, "rotation", JOINT, 1e6)?;
    let by = p.get("rotateBy");
    if rotation.is_some() == by.is_some() {
        return Err(bad(JOINT, "pass `rotation` or `rotateBy`"));
    }
    let turn = match by {
        Some(b) => {
            let degrees = number(b, "degrees", JOINT, -1e4..=1e4)?.ok_or_else(|| bad(JOINT, "`rotateBy.degrees` is required"))?;
            let axis = match b.get("axis") {
                Some(Value::String(v)) if v == "view" => Axis::View,
                Some(Value::String(v)) if v == "bone" => Axis::Bone,
                Some(_) => Axis::World(numbers::<3>(b, "axis", JOINT, 1e6)?.ok_or_else(|| bad(JOINT, "`rotateBy.axis` must be [x, y, z], \"view\" or \"bone\""))?),
                None => return Err(bad(JOINT, "`rotateBy.axis` is required")),
            };
            Some((axis, degrees))
        }
        None => None,
    };
    edit_scene(s, p, JOINT, "Pose Joint", |scene| {
        match (rotation, turn) {
            (Some(q), _) => {
                let f = scene.figures.get_mut(i).ok_or_else(|| bad(JOINT, "no such figure"))?;
                let j = f.root.find_mut(&joint).ok_or_else(|| bad(JOINT, format!("the figure has no joint {joint:?}")))?;
                if q.iter().map(|v| v * v).sum::<f64>() < 1e-12 {
                    return Err(bad(JOINT, "`rotation` must not be zero"));
                }
                j.rotation = prism_doc::figure::normalize_quat(q);
                Ok(())
            }
            (None, Some((axis, degrees))) => {
                let axis = match axis {
                    Axis::World(a) => a,
                    Axis::View => figure3d::view_axis(scene).ok_or_else(|| bad(JOINT, "invalid camera"))?,
                    Axis::Bone => figure3d::bone_axis(scene, i, &joint).ok_or_else(|| bad(JOINT, format!("{joint:?} has no bone to twist about")))?,
                };
                figure3d::rotate_joint(scene, i, &joint, axis, degrees).map_err(|e| bad(JOINT, e))
            }
            (None, None) => Err(bad(JOINT, "pass `rotation` or `rotateBy`")),
        }
    })
}

fn reach(s: &mut Session, p: &Value) -> Result<Value> {
    let n = scene_len(s, p)?;
    let i = figure_index(p, n, REACH)?;
    let joint = text(p, "joint", REACH)?.ok_or_else(|| bad(REACH, "`joint` is required"))?.to_string();
    let point = numbers::<2>(p, "point", REACH, 1e7)?.ok_or_else(|| bad(REACH, "`point` is required"))?;
    edit_scene(s, p, REACH, "Pose Joint", |scene| figure3d::reach_to(scene, i, &joint, point).map_err(|e| bad(REACH, e)))
}

fn reset(s: &mut Session, p: &Value) -> Result<Value> {
    let n = scene_len(s, p)?;
    let only = p.get("figure").map(|_| figure_index(p, n, RESET)).transpose()?;
    edit_scene(s, p, RESET, "Reset Pose", |scene| {
        for (k, f) in scene.figures.iter_mut().enumerate() {
            if only.is_none_or(|i| i == k) {
                // Prism's standing pose, kept where the figure stands.
                let mut standing = standing_figure().root;
                standing.offset = f.root.offset;
                standing.rotation = f.root.rotation;
                f.root = standing;
            }
        }
        Ok(())
    })
}

fn split_poses(poses: Vec<Pose>) -> (Vec<Figure>, Vec<HandPose>) {
    let mut figures = Vec::new();
    let mut hands = Vec::new();
    for pose in poses {
        match pose {
            Pose::Body(f) => figures.push(f),
            Pose::Hand(h) => hands.push(h),
        }
    }
    (figures, hands)
}

/// Put hand poses on figure `f`, as CLIP STUDIO does with the whole figure selected: a left and a
/// right pose each go to their own hand; otherwise the (last) pose goes to both hands, mirrored
/// for the other one. `side` puts the last pose on that hand only.
fn apply_hands(f: &mut Figure, hands: &[HandPose], side: Option<figure3d::hand::Side>, locked: [bool; 5]) -> std::result::Result<(), String> {
    use figure3d::hand::{Side, apply_hand_pose};
    let side_of = |h: &HandPose| if h.left { Side::Left } else { Side::Right };
    let Some(last) = hands.last() else { return Ok(()) };
    let mut plan: Vec<(Side, &HandPose)> = Vec::new();
    match side {
        Some(s) => plan.push((s, last)),
        None => {
            let left = hands.iter().rev().find(|h| h.left);
            let right = hands.iter().rev().find(|h| !h.left);
            match (left, right) {
                (Some(l), Some(r)) => plan.extend([(Side::Left, l), (Side::Right, r)]),
                _ => plan.extend([(Side::Left, last), (Side::Right, last)]),
            }
        }
    }
    let mut posed = 0;
    let mut last_err = None;
    for (target, h) in plan {
        match apply_hand_pose(f, target, &h.hand, side_of(h), locked) {
            Ok(()) => posed += 1,
            Err(e) => last_err = Some(e),
        }
    }
    match (posed, last_err) {
        (0, Some(e)) => Err(e),
        _ => Ok(()),
    }
}

/// The finger names in `locked` (thumb first), as [`set_hand`] reads them.
fn locked_fingers(p: &Value, cmd: &str) -> Result<[bool; 5]> {
    use figure3d::hand::DIGITS;
    let mut locked = [false; 5];
    match p.get("locked") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            for v in list {
                let name = v.as_str().ok_or_else(|| bad(cmd, "`locked` lists finger names"))?;
                let i = DIGITS.iter().position(|d| d.eq_ignore_ascii_case(name)).ok_or_else(|| bad(cmd, format!("unknown finger {name:?} (thumb, index, middle, ring, pinky)")))?;
                if let Some(l) = locked.get_mut(i) {
                    *l = true;
                }
            }
        }
        Some(_) => return Err(bad(cmd, "`locked` lists finger names")),
    }
    Ok(locked)
}

/// The figure of the layer named by `p` with a joint nearest document point `pt`.
fn figure_at(s: &Session, p: &Value, pt: [f64; 2]) -> Result<Option<usize>> {
    let id = layer_param(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    Ok(joints(&st.doc, id).into_iter().map(|j| (j.figure, (j.doc[0] - pt[0]).hypot(j.doc[1] - pt[1]))).filter(|(_, d)| d.is_finite()).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(f, _)| f))
}

/// CLIP STUDIO's hand pose materials: pose the hands of a figure layer's figures.
fn apply_hand_pose_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    use figure3d::hand::Side;
    let n = scene_len(s, p)?;
    let at = numbers::<2>(p, "at", HAND_POSE, 1e7)?;
    let only = match (p.get("figure"), at) {
        (Some(_), _) => Some(figure_index(p, n, HAND_POSE)?),
        // Dropped on the canvas: the figure with a joint nearest the drop point.
        (None, Some(pt)) => figure_at(s, p, pt)?,
        (None, None) => None,
    };
    let side = match text(p, "side", HAND_POSE)?.unwrap_or("both") {
        "both" => None,
        "left" => Some(Side::Left),
        "right" => Some(Side::Right),
        other => return Err(bad(HAND_POSE, format!("`side` must be \"left\", \"right\" or \"both\", not {other:?}"))),
    };
    let locked = locked_fingers(p, HAND_POSE)?;
    let (bodies, hands) = split_poses(poses_from(p, HAND_POSE)?);
    if let Some(b) = bodies.first() {
        return Err(bad(HAND_POSE, format!("{} is a whole-figure pose, not a hand pose", b.name)));
    }
    if hands.is_empty() {
        return Err(bad(HAND_POSE, "pass a hand pose: `uuid`, `uuids`, `path` or `data`"));
    }
    edit_scene(s, p, HAND_POSE, "Hand Pose", |scene| {
        let mut posed = 0;
        let mut last_err = None;
        for (k, f) in scene.figures.iter_mut().enumerate() {
            if only.is_some_and(|i| i != k) {
                continue;
            }
            match apply_hands(f, &hands, side, locked) {
                Ok(()) => posed += 1,
                Err(e) => last_err = Some(e),
            }
        }
        if posed == 0 {
            return Err(bad(HAND_POSE, last_err.unwrap_or_else(|| "no figure to pose".into())));
        }
        Ok(())
    })
}

/// A whole-figure pose material on one figure, as CLIP STUDIO applies one to a selected figure:
/// the figure keeps where it stands and which way it faces (its root joint) and takes every joint
/// below from the pose, bone lengths included.
fn apply_pose_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let n = scene_len(s, p)?;
    let at = numbers::<2>(p, "at", POSE, 1e7)?;
    let index = match (p.get("figure"), at) {
        (Some(_), _) => figure_index(p, n, POSE)?,
        (None, Some(pt)) => figure_at(s, p, pt)?.ok_or_else(|| bad(POSE, "the layer has no figure to pose"))?,
        (None, None) if n == 1 => 0,
        (None, None) => return Err(bad(POSE, "the layer holds several figures: pass `figure` (or `at`)")),
    };
    let (bodies, hands) = split_poses(poses_from(p, POSE)?);
    if let Some(h) = hands.first() {
        return Err(bad(POSE, format!("{} is a hand pose: apply it with {HAND_POSE}", h.name)));
    }
    let [pose] = bodies.as_slice() else { return Err(bad(POSE, "pass one whole-figure pose: `uuid`, `path` or `data`")) };
    edit_scene(s, p, POSE, "Apply Pose", |scene| {
        let f = scene.figures.get_mut(index).ok_or_else(|| bad(POSE, "no such figure"))?;
        let (offset, rotation) = (f.root.offset, f.root.rotation);
        let mut root = pose.root.clone();
        if !root.name.eq_ignore_ascii_case(&f.root.name) {
            // A pose rooted elsewhere (at the hips, say) hangs under the figure's own root.
            root = prism_doc::figure::Joint { name: f.root.name.clone(), rotation: [1.0, 0.0, 0.0, 0.0], offset: [0.0; 3], children: vec![root] };
        }
        root.offset = offset;
        root.rotation = rotation;
        f.root = root;
        f.source = pose.source.clone();
        Ok(())
    })
}

/// CLIP STUDIO's hand setup: blend a hand between the flat, spread and closed shapes.
fn set_hand(s: &mut Session, p: &Value) -> Result<Value> {
    use figure3d::hand::{Closed, HandSetup, Side};
    let n = scene_len(s, p)?;
    let only = p.get("figure").map(|_| figure_index(p, n, HAND)).transpose()?;
    let sides: &[Side] = match text(p, "side", HAND)?.unwrap_or("both") {
        "both" => &[Side::Left, Side::Right],
        "left" => &[Side::Left],
        "right" => &[Side::Right],
        other => return Err(bad(HAND, format!("`side` must be \"left\", \"right\" or \"both\", not {other:?}"))),
    };
    let weights = numbers::<3>(p, "weights", HAND, 1e6)?.ok_or_else(|| bad(HAND, "`weights` is required: [flat, spread, closed]"))?;
    let closed = match text(p, "closed", HAND)? {
        None => Closed::Fist,
        Some(id) => Closed::from_id(id).ok_or_else(|| bad(HAND, format!("`closed` must be one of fist, catPaw, tube, grip, not {id:?}")))?,
    };
    let locked = locked_fingers(p, HAND)?;
    let setup = HandSetup { weights, closed, locked };
    edit_scene(s, p, HAND, "Hand Pose", |scene| {
        let mut posed = 0;
        let mut last_err = None;
        for (k, f) in scene.figures.iter_mut().enumerate() {
            if only.is_some_and(|i| i != k) {
                continue;
            }
            for side in sides {
                match figure3d::hand::set_hand(f, *side, &setup) {
                    Ok(()) => posed += 1,
                    Err(e) => last_err = Some(e),
                }
            }
        }
        if posed == 0 {
            return Err(bad(HAND, last_err.unwrap_or_else(|| "no hand to pose".into())));
        }
        Ok(())
    })
}

fn color_of(v: &Value) -> Option<[f32; 3]> {
    match v {
        Value::String(h) => {
            let h = h.strip_prefix('#')?;
            if h.len() != 6 || !h.is_ascii() {
                return None;
            }
            let c = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|b| f32::from(b) / 255.0);
            Some([c(0)?, c(2)?, c(4)?])
        }
        Value::Array(a) if a.len() == 3 => {
            let mut out = [0.0f32; 3];
            for (o, x) in out.iter_mut().zip(a) {
                *o = x.as_f64().filter(|x| (0.0..=1.0).contains(x))? as f32;
            }
            Some(out)
        }
        _ => None,
    }
}

fn set_look(s: &mut Session, p: &Value) -> Result<Value> {
    let color = p.get("color").map(|v| color_of(v).ok_or_else(|| bad(LOOK, "`color` must be \"#rrggbb\" or [r, g, b] in 0..1"))).transpose()?;
    let outline = p.get("outline").map(|v| v.as_bool().ok_or_else(|| bad(LOOK, "`outline` must be a boolean"))).transpose()?;
    let width = number(p, "outlineWidth", LOOK, 0.0..=prism_doc::figure::FigureLook::MAX_OUTLINE)?;
    edit_scene(s, p, LOOK, "3D Figure Look", |scene| {
        if let Some(c) = color {
            scene.look.color = c;
        }
        if let Some(o) = outline {
            scene.look.outline = o;
        }
        if let Some(w) = width {
            scene.look.outline_width = w;
        }
        Ok(())
    })
}

fn rasterize(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if st.doc.layer(id).ok_or(EngineError::NoLayer(id))?.figure.is_none() {
        return Err(bad(RASTERIZE, "the layer is not a 3D figure layer"));
    }
    s.edit("Rasterize 3D Figure", |doc, _| {
        doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.figure = None;
        Ok(())
    })?;
    Ok(json!({"layer": id.0}))
}

/// Joint handles of a figure layer for the canvas editor and agents.
pub fn joints(doc: &Document, id: LayerId) -> Vec<figure3d::JointPoint> {
    doc.layer(id).and_then(|l| l.figure.as_ref()).map(figure3d::joint_points).unwrap_or_default()
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_figure(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    d.doc.layer(id).filter(|l| l.figure.is_some()).map(|_| ()).ok_or_else(|| "the active layer is not a 3D figure layer".into())
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    vec![
        spec!(
            NEW,
            "New 3D Figure Layer",
            r#"{"name":str?,"uuid":pose id from brush.cspLibrary.list poses?|"uuids":[pose id]? (several figures in one layer),"root":CLIP STUDIO user data folder? (with uuid),"path":".pose or .pep file"?,"data":base64?,"at":[x,y]? (centre, default the canvas centre)} → {layer, figures} (no pose: Prism's standing figure; hand poses only: the standing figure with those hands; the camera faces the figures and they fill 80% of the canvas height)"#,
            has_doc,
            new_layer
        ),
        spec!(ADD, "Add Pose to 3D Figure Layer", r#"{"layer":id?,"uuid":pose id?|"path"?|"data"?,"root"?} (another figure, placed by its own pose)"#, has_figure, add_pose),
        spec!(REMOVE, "Remove Figure", r#"{"layer":id?,"figure":index}"#, has_figure, remove_figure),
        spec!(
            CAMERA,
            "Move 3D Camera",
            r#"{"layer":id?,"yaw":deg?,"pitch":-89..89?,"distance":cm?,"fov":5..120?,"target":[x,y,z] cm?,"orbitBy":[dyaw,dpitch]?,"dollyBy":factor?,"panBy":[dx,dy] px?,"draft":bool=false}"#,
            has_figure,
            set_camera
        ),
        spec!(FRAME, "Frame 3D Figures", r#"{"layer":id?,"faceFront":bool=false} (aim at every figure, 80% of the canvas height, centred)"#, has_figure, frame_cmd),
        spec!(
            JOINT,
            "Pose Joint",
            r#"{"layer":id?,"figure":index=0,"joint":name,"rotation":[w,x,y,z]? (local)|"rotateBy":{"axis":[x,y,z]|"view"|"bone","degrees":n}?,"draft":bool=false} (joint names as in CLIP STUDIO poses: "leftforearm_bb_", …)"#,
            has_figure,
            set_joint
        ),
        spec!(
            REACH,
            "Reach Joint To",
            r#"{"layer":id?,"figure":index=0,"joint":hand or foot name,"point":[x,y] document px,"draft":bool=false} (two-bone IK at the joint's current depth)"#,
            has_figure,
            reach
        ),
        spec!(
            HAND,
            "Hand Pose",
            r#"{"layer":id?,"figure":index? (default every figure),"side":"both"|"left"|"right"="both","weights":[flat,spread,closed] (≥0, blended by ratio: CLIP STUDIO's hand setup triangle),"closed":"fist"|"catPaw"|"tube"|"grip"="fist","locked":["thumb","index","middle","ring","pinky"]? (fingers left as they are),"draft":bool=false}"#,
            has_figure,
            set_hand
        ),
        spec!(
            HAND_POSE,
            "Apply Hand Pose",
            r#"{"layer":id?,"uuid":hand pose id from brush.cspLibrary.list poses (hand set)?|"uuids":[id]?|"path":".pose or .pep"?|"data":base64?,"root"?,"figure":index? (default every figure)|"at":[x,y] document px? (the figure nearest),"side":"both"|"left"|"right"="both","locked":[finger]?,"draft":bool=false} (CLIP STUDIO hand pose materials: the fingers turn as in the pose, mirrored for the other hand; a left and a right pose each go to their own hand)"#,
            has_figure,
            apply_hand_pose_cmd
        ),
        spec!(
            POSE,
            "Apply Pose",
            r#"{"layer":id?,"uuid":pose id from brush.cspLibrary.list poses?|"path":".pose or .pep"?|"data":base64?,"root"?,"figure":index? (default the only figure)|"at":[x,y] document px? (the figure nearest),"draft":bool=false} (a whole-figure CLIP STUDIO pose on one figure: it keeps its place and facing, every joint below takes the pose)"#,
            has_figure,
            apply_pose_cmd
        ),
        spec!(RESET, "Reset Pose", r#"{"layer":id?,"figure":index? (default every figure)} (Prism's standing pose where the figure stands)"#, has_figure, reset),
        spec!(LOOK, "3D Figure Look", r##"{"layer":id?,"color":"#rrggbb"|[r,g,b]?,"outline":bool?,"outlineWidth":0..64?}"##, has_figure, set_look),
        spec!(RASTERIZE, "3D Figure", r#"{"layer":id?} (keep the pixels, drop the figures)"#, has_figure, rasterize),
    ]
}

#[cfg(test)]
mod tests;
