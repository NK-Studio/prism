//! 3D figure layers: render a [`FigureScene`] (posed mannequins seen through an orbit camera)
//! into a surface of any colour model and depth, and the posing operations the editor uses.
//!
//! Rendering ray-casts Prism's own procedural mannequin ([`mannequin`]: capsules and
//! ellipsoids, no meshes) for every pixel through the inverse of the scene's 2D placement, so a
//! moved, scaled or rotated layer re-renders exactly, without resampling. 2×2 samples per pixel
//! (one for drafts), Lambert + ambient gray shading, and lines where the depth jumps (silhouettes
//! and limbs in front of the body). Row bands render in parallel.
pub mod camera;
pub mod ik;
pub mod mannequin;
pub mod math;
pub mod skeleton;
pub mod trace;

use camera::{NEAR, View};
use mannequin::Shape;
use math::{V3, add, axis_angle, cross, dot, len, normalize_or, scale, sub};
use prism_color::PixelFormat;
use prism_doc::figure::{Figure, FigureCamera, FigureScene};
use prism_geom::{Affine, Point, Rect};
use prism_raster::{Interrupt, Surface};
use rayon::prelude::*;

/// Most pixels one render covers (a 8000 × 8000 figure).
pub const MAX_PIXELS: i64 = 64_000_000;
/// Rows per parallel band (even, so draft blocks never straddle two bands).
const BAND: i32 = 32;
/// Drafts larger than this many pixels trace one ray per 2 × 2 block.
const DRAFT_FULL_RES: i64 = 1_500_000;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RenderOptions {
    /// One sample per pixel (for live dragging); the final render uses four.
    pub draft: bool,
}

/// A solid placed for rendering: its shape and its document-space bounding box.
struct Solid {
    shape: Shape,
    bbox: [f64; 4],
}

fn solids(scene: &FigureScene, view: &View) -> Vec<Solid> {
    let margin = scene.look.outline_width.max(0.0) + 2.0;
    let mut out = Vec::new();
    for f in &scene.figures {
        for shape in mannequin::build(&skeleton::pose(f)) {
            let (c, r) = shape.bounds();
            let cc = view.to_camera(c);
            if cc[2] - r <= NEAR {
                // Reaches the eye: it would fill the view; leave it out rather than smear it.
                continue;
            }
            let Some([ix, iy]) = view.image_of(cc) else { continue };
            let ir = view.focal * r / (cc[2] - r);
            let corners = [[ix - ir, iy - ir], [ix + ir, iy - ir], [ix + ir, iy + ir], [ix - ir, iy + ir]].map(|[x, y]| view.placement.apply(Point::new(x, y)));
            let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
            for p in corners {
                x0 = x0.min(p.x);
                y0 = y0.min(p.y);
                x1 = x1.max(p.x);
                y1 = y1.max(p.y);
            }
            if [x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
                out.push(Solid { shape, bbox: [x0 - margin, y0 - margin, x1 + margin, y1 + margin] });
            }
        }
    }
    out
}

/// The document rectangle the scene's figures cover (outline included), if any.
pub fn bounds(scene: &FigureScene) -> Option<Rect> {
    let view = View::new(&scene.camera, scene.placement)?;
    let s = solids(scene, &view);
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for b in s.iter().map(|s| s.bbox) {
        x0 = x0.min(b[0]);
        y0 = y0.min(b[1]);
        x1 = x1.max(b[2]);
        y1 = y1.max(b[3]);
    }
    let clamp = |v: f64| v.clamp(-1e9, 1e9) as i32;
    (x0 < x1 && y0 < y1).then(|| Rect::new(clamp(x0.floor()), clamp(y0.floor()), clamp(x1.ceil()), clamp(y1.ceil())))
}

/// One shaded sample: depth along the view (or infinity) and premultiplied colour.
fn sample(view: &View, solids: &[&Solid], light: V3, color: [f32; 3], p: [f64; 2]) -> (f64, [f32; 4]) {
    let (o, d) = view.ray(p);
    let mut best: Option<(f64, V3)> = None;
    for s in solids {
        let b = s.bbox;
        if p[0] < b[0] || p[0] > b[2] || p[1] < b[1] || p[1] > b[3] {
            continue;
        }
        if let Some((t, n)) = trace::hit(&s.shape, o, d)
            && best.is_none_or(|(bt, _)| t < bt)
        {
            best = Some((t, n));
        }
    }
    match best {
        Some((t, n)) => {
            let depth = t * dot(d, view.fwd);
            let diffuse = dot(n, light).max(0.0);
            // A little light from the viewer keeps faces turned away readable.
            let fill = dot(n, scale(view.fwd, -1.0)).max(0.0) * 0.12;
            let shade = (0.30 + 0.62 * diffuse + fill).min(1.0) as f32;
            (depth, [color[0] * shade, color[1] * shade, color[2] * shade, 1.0])
        }
        None => (f64::INFINITY, [0.0; 4]),
    }
}

/// Render the scene's figures. The surface covers the figures' bounds within `allowed` (pass the
/// canvas grown by a margin; renders are not clipped to the canvas so moving the layer never
/// cuts an edge).
pub fn render(scene: &FigureScene, format: PixelFormat, allowed: Rect, opts: RenderOptions, ctl: &Interrupt) -> Result<Surface, String> {
    scene.validate()?;
    let view = View::new(&scene.camera, scene.placement).ok_or("the figure layer's placement cannot be inverted")?;
    let solids = solids(scene, &view);
    let mut surface = Surface::new(format);
    let Some(area) = bounds(scene).map(|b| b.intersect(&allowed)).filter(|r| !r.is_empty()) else {
        return Ok(surface);
    };
    let pixels = i64::from(area.width()) * i64::from(area.height());
    if pixels > MAX_PIXELS {
        return Err("the figures are too large to render: zoom the camera out or scale the layer down".into());
    }
    let light = normalize_or(add(add(scale(view.up, 0.55), scale(view.right, -0.35)), scale(view.fwd, -0.75)), [0.0, 0.0, 1.0]);
    let color = scene.look.color;
    let width = scene.look.outline.then_some(scene.look.outline_width).filter(|w| *w > 0.0);
    let reach = width.map_or(0, |w| w.ceil() as i32 + 1);
    // Depth jumps smaller than this are one surface, larger ones draw a line: a few centimetres,
    // more when a pixel spans more of the figure.
    let px_cm = view.distance_per_pixel(scene.camera.distance);
    let tol = (3.0f64).max(4.0 * px_cm);
    let line = [0.18f32, 0.18, 0.20];
    let offsets: &[[f64; 2]] = if opts.draft { &[[0.5, 0.5]] } else { &[[0.25, 0.25], [0.75, 0.25], [0.25, 0.75], [0.75, 0.75]] };
    let bands: Vec<Rect> = (area.y0..area.y1).step_by(BAND as usize).map(|y| Rect::new(area.x0, y, area.x1, (y + BAND).min(area.y1))).collect();
    let channels = format.channels();
    // Drafts of big figures trace one ray per 2 × 2 pixels.
    let block = if opts.draft && pixels > DRAFT_FULL_RES { 2 } else { 1 };
    let group = rayon::current_num_threads().max(1) * 2;
    for chunk in bands.chunks(group) {
        ctl.check().map_err(|e| e.to_string())?;
        let done: Vec<(Rect, Vec<f32>)> = chunk
            .par_iter()
            .map(|band| {
                if opts.draft {
                    return (*band, draft_band(&view, &solids, light, color, &format, *band, block));
                }
                let (w, ext0, ext1) = (i32::try_from(band.width()).unwrap_or(0), band.y0 - reach, band.y1 + reach);
                let near: Vec<&Solid> = solids.iter().filter(|s| s.bbox[3] >= f64::from(ext0) && s.bbox[1] <= f64::from(ext1)).collect();
                let wu = usize::try_from(w).unwrap_or(0);
                let rows = usize::try_from(ext1 - ext0).unwrap_or(0);
                // Centre-sample depth for the band and its outline margin.
                let mut depth = vec![f64::INFINITY; wu * rows];
                for (r, row) in depth.chunks_mut(wu.max(1)).enumerate() {
                    let y = ext0 + r as i32;
                    for (c, d) in row.iter_mut().enumerate() {
                        let x = band.x0 + c as i32;
                        *d = sample(&view, &near, light, color, [f64::from(x) + 0.5, f64::from(y) + 0.5]).0;
                    }
                }
                let at = |x: i32, y: i32| -> f64 {
                    let (c, r) = (x - band.x0, y - ext0);
                    if c < 0 || c >= w || r < 0 || y >= ext1 {
                        return f64::INFINITY;
                    }
                    depth.get(r as usize * wu + c as usize).copied().unwrap_or(f64::INFINITY)
                };
                let mut out = vec![0.0f32; wu * usize::try_from(band.height()).unwrap_or(0) * channels];
                let mut px = vec![0.0f32; channels.max(4)];
                for (i, y) in (band.y0..band.y1).enumerate() {
                    for c in 0..w {
                        let x = band.x0 + c;
                        let mut acc = [0.0f32; 4];
                        for o in offsets {
                            let (_, s) = sample(&view, &near, light, color, [f64::from(x) + o[0], f64::from(y) + o[1]]);
                            for k in 0..4 {
                                acc[k] += s[k];
                            }
                        }
                        let n = offsets.len() as f32;
                        let alpha = acc[3] / n;
                        let mut rgb = if acc[3] > 0.0 { [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3]] } else { [0.0; 3] };
                        // Lines: inside the figure, within `width` of a depth jump or its edge.
                        if let Some(lw) = width {
                            let d = at(x, y);
                            if d.is_finite() || alpha > 0.0 {
                                let own = if d.is_finite() { d } else { f64::INFINITY };
                                let mut nearest = f64::INFINITY;
                                for dy in -reach..=reach {
                                    for dx in -reach..=reach {
                                        if dx == 0 && dy == 0 {
                                            continue;
                                        }
                                        let q = at(x + dx, y + dy);
                                        if !q.is_finite() || (own.is_finite() && q > own + tol) {
                                            nearest = nearest.min(f64::from(dx * dx + dy * dy).sqrt());
                                        }
                                    }
                                }
                                let cover = (lw - (nearest - 0.5)).clamp(0.0, 1.0) as f32;
                                if cover > 0.0 {
                                    for k in 0..3 {
                                        rgb[k] += (line[k] - rgb[k]) * cover;
                                    }
                                }
                            }
                        }
                        let base = (i * wu + c as usize) * channels;
                        let wrote = prism_raster::from_rgba_into(&format, [rgb[0], rgb[1], rgb[2], alpha], &mut px);
                        if let Some(dst) = out.get_mut(base..base + channels) {
                            if format.alpha {
                                dst.copy_from_slice(px.get(..channels).unwrap_or(&[]));
                            } else if let Some(src) = px.get(..wrote) {
                                // No alpha channel: premultiply onto white so edges stay smooth.
                                for (d, s) in dst.iter_mut().zip(src) {
                                    *d = s * alpha + (1.0 - alpha);
                                }
                            }
                        }
                    }
                }
                (*band, out)
            })
            .collect();
        for (band, data) in done {
            surface.write_region(band, &data);
        }
    }
    surface.prune();
    Ok(surface)
}

/// A draft band: one sample per `block` × `block` pixels, no lines.
fn draft_band(view: &View, solids: &[Solid], light: V3, color: [f32; 3], format: &PixelFormat, band: Rect, block: i32) -> Vec<f32> {
    let channels = format.channels();
    let (w, h) = (i32::try_from(band.width()).unwrap_or(0), i32::try_from(band.height()).unwrap_or(0));
    let near: Vec<&Solid> = solids.iter().filter(|s| s.bbox[3] >= f64::from(band.y0) && s.bbox[1] <= f64::from(band.y1)).collect();
    let wu = usize::try_from(w).unwrap_or(0);
    let mut out = vec![0.0f32; wu * usize::try_from(h).unwrap_or(0) * channels];
    let mut px = vec![0.0f32; channels.max(4)];
    let block = block.max(1);
    for by in (0..h).step_by(block as usize) {
        for bx in (0..w).step_by(block as usize) {
            let centre = [f64::from(band.x0 + bx) + f64::from(block) / 2.0, f64::from(band.y0 + by) + f64::from(block) / 2.0];
            let (_, s) = sample(view, &near, light, color, centre);
            let alpha = s[3];
            let wrote = prism_raster::from_rgba_into(format, [s[0], s[1], s[2], alpha], &mut px);
            for y in by..(by + block).min(h) {
                for x in bx..(bx + block).min(w) {
                    let base = (y as usize * wu + x as usize) * channels;
                    if let Some(dst) = out.get_mut(base..base + channels) {
                        if format.alpha {
                            dst.copy_from_slice(px.get(..channels).unwrap_or(&[]));
                        } else if let Some(src) = px.get(..wrote) {
                            for (d, v) in dst.iter_mut().zip(src) {
                                *d = v * alpha + (1.0 - alpha);
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

impl View {
    /// World centimetres one document pixel spans at camera depth `depth`.
    pub fn distance_per_pixel(&self, depth: f64) -> f64 {
        let scale = self.placement.determinant().abs().sqrt();
        if scale > 0.0 && self.focal > 0.0 { depth / (self.focal * scale) } else { 1.0 }
    }
}

/// A joint's place on the document, for handles and picking.
#[derive(Clone, Debug, PartialEq)]
pub struct JointPoint {
    pub figure: usize,
    pub name: String,
    pub parent: Option<String>,
    pub doc: [f64; 2],
    pub depth: f64,
}

/// Every joint in front of the camera, projected to the document.
pub fn joint_points(scene: &FigureScene) -> Vec<JointPoint> {
    let Some(view) = View::new(&scene.camera, scene.placement) else { return Vec::new() };
    let mut out = Vec::new();
    for (fi, f) in scene.figures.iter().enumerate() {
        let posed = skeleton::pose(f);
        for p in &posed {
            if let Some((doc, depth)) = view.project(p.pos) {
                let parent = p.parent.and_then(|i| posed.get(i)).map(|q| q.name.clone());
                out.push(JointPoint { figure: fi, name: p.name.clone(), parent, doc, depth });
            }
        }
    }
    out
}

/// Every posed joint of every figure (world centimetres).
fn all_joints(scene: &FigureScene) -> Vec<V3> {
    scene.figures.iter().flat_map(|f| skeleton::pose(f).into_iter().map(|p| p.pos)).collect()
}

/// The direction a figure faces (horizontal), from its hips: `None` without both hips.
pub fn facing(figure: &Figure) -> Option<V3> {
    let posed = skeleton::pose(figure);
    let (l, r) = (skeleton::find(&posed, "leftupleg_bb_")?, skeleton::find(&posed, "rightupleg_bb_")?);
    // Facing +Z, a figure's left is on +X: front = (left − right) × up.
    let f = cross(sub(l.pos, r.pos), [0.0, 1.0, 0.0]);
    let f = [f[0], 0.0, f[2]];
    (len(f) > 1e-6).then(|| normalize_or(f, [0.0, 0.0, 1.0]))
}

/// Where to look from: a lone figure from its front; several from the side of the line between
/// the first two (so both show, as CLIP STUDIO's pose thumbnails do), on the side their fronts
/// lean towards.
fn view_direction(figures: &[Figure]) -> Option<V3> {
    let first = figures.first()?;
    let front = facing(first);
    let centre = |f: &Figure| skeleton::find(&skeleton::pose(f), "hips_bb_").map(|h| h.pos);
    if let Some(second) = figures.get(1)
        && let (Some(a), Some(b)) = (centre(first), centre(second))
    {
        let across = [b[0] - a[0], 0.0, b[2] - a[2]];
        if len(across) > 1.0 {
            let side = normalize_or(cross(across, [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
            let lean = figures.iter().filter_map(facing).fold([0.0; 3], add);
            let lean = if len(lean) > 1e-3 { lean } else { front.unwrap_or(side) };
            return Some(if dot(side, lean) >= 0.0 { side } else { scale(side, -1.0) });
        }
    }
    front
}

/// Aim the camera at every figure (from the first figure's front when `face_front`) and place
/// the image so the figures are `height` document pixels tall, centred on `center`.
pub fn frame(scene: &mut FigureScene, center: [f64; 2], height: f64, face_front: bool) -> Result<(), String> {
    let joints = all_joints(scene);
    if joints.is_empty() {
        return Err("the layer has no figures".into());
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in &joints {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let target = scale(add(lo, hi), 0.5);
    let radius = joints.iter().map(|p| len(sub(*p, target))).fold(0.0f64, f64::max) + 20.0;
    let cam = &mut scene.camera;
    cam.target = target;
    if face_front && let Some(front) = view_direction(&scene.figures) {
        cam.yaw = front[0].atan2(front[2]).to_degrees();
    }
    let half = (cam.fov_deg.clamp(FigureCamera::MIN_FOV, FigureCamera::MAX_FOV) / 2.0).to_radians();
    cam.distance = (radius / half.sin() * 1.05).clamp(FigureCamera::MIN_DISTANCE, FigureCamera::MAX_DISTANCE);
    // The figures' image extent with an identity placement, then scale and centre it.
    let view = View::new(&scene.camera, Affine::IDENTITY).ok_or("invalid camera")?;
    let (mut y0, mut y1, mut x0, mut x1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for p in &joints {
        if let Some(([x, y], _)) = view.project(*p) {
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
    }
    let span = (y1 - y0).max(x1 - x0) * 1.15;
    if !(span.is_finite() && span > 1e-6 && height.is_finite() && height > 0.0) {
        return Err("the figures cannot be framed".into());
    }
    let s = height / span;
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    scene.placement = Affine::translate(center[0] - s * cx, center[1] - s * cy).mul(&Affine::scale(s));
    Ok(())
}

/// Turn the camera around its target by `yaw` and `pitch` degrees.
pub fn orbit(scene: &mut FigureScene, yaw: f64, pitch: f64) {
    let c = &mut scene.camera;
    if yaw.is_finite() {
        c.yaw = (c.yaw + yaw).rem_euclid(360.0);
    }
    if pitch.is_finite() {
        c.pitch = (c.pitch + pitch).clamp(-FigureCamera::MAX_PITCH, FigureCamera::MAX_PITCH);
    }
}

/// Move the camera towards (`factor` < 1) or away from its target, keeping the target's place
/// on the document.
pub fn dolly(scene: &mut FigureScene, factor: f64) {
    if factor.is_finite() && factor > 0.0 {
        let c = &mut scene.camera;
        c.distance = (c.distance * factor).clamp(FigureCamera::MIN_DISTANCE, FigureCamera::MAX_DISTANCE);
    }
}

/// Shift the figures on the document by `delta` pixels.
pub fn pan(scene: &mut FigureScene, delta: [f64; 2]) {
    if delta.iter().all(|v| v.is_finite()) {
        scene.placement = Affine::translate(delta[0], delta[1]).mul(&scene.placement);
    }
}

/// Turn joint `name` of figure `figure` by `degrees` about `axis` (world).
pub fn rotate_joint(scene: &mut FigureScene, figure: usize, name: &str, axis: V3, degrees: f64) -> Result<(), String> {
    if !degrees.is_finite() || !axis.iter().all(|v| v.is_finite()) || len(axis) < 1e-9 {
        return Err("invalid joint rotation".into());
    }
    let f = scene.figures.get_mut(figure).ok_or("no such figure")?;
    ik::rotate_joint(f, name, axis_angle(axis, degrees.to_radians()))
}

/// Move end joint `name` (a hand, a foot) of figure `figure` towards the document point `doc`,
/// at the depth it is now (two-bone IK).
pub fn reach_to(scene: &mut FigureScene, figure: usize, name: &str, doc: [f64; 2]) -> Result<(), String> {
    let view = View::new(&scene.camera, scene.placement).ok_or("invalid placement")?;
    let f = scene.figures.get_mut(figure).ok_or("no such figure")?;
    let posed = skeleton::pose(f);
    let j = skeleton::find(&posed, name).ok_or_else(|| format!("the figure has no joint {name:?}"))?;
    let depth = view.to_camera(j.pos)[2];
    if depth <= NEAR {
        return Err("the joint is behind the camera".into());
    }
    let target = view.unproject(doc, depth);
    ik::reach(f, name, target)
}

/// The direction of joint `name`'s bone: from the joint to its first child (world).
pub fn bone_axis(scene: &FigureScene, figure: usize, name: &str) -> Option<V3> {
    let posed = skeleton::pose(scene.figures.get(figure)?);
    let (i, j) = posed.iter().enumerate().find(|(_, p)| p.name == name)?;
    let child = posed.iter().find(|c| c.parent == Some(i))?;
    let d = sub(child.pos, j.pos);
    (len(d) > 1e-9).then(|| normalize_or(d, [0.0, 1.0, 0.0]))
}

/// The camera's viewing direction (world), e.g. the axis to turn joints about on screen.
pub fn view_axis(scene: &FigureScene) -> Option<V3> {
    View::new(&scene.camera, scene.placement).map(|v| v.fwd)
}

#[cfg(test)]
mod tests;
