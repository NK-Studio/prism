//! Character meshes on the document: skinned triangles projected through the camera and the
//! layer's placement, rasterized onto grids of sample points with a depth buffer. The figure
//! renderer asks for the same sample points it ray-traces mannequins at, so meshes and
//! mannequins meet in one depth order and share anti-aliasing and outlines.
use super::camera::{NEAR, View};
use super::math::{V3, dot, normalize_or, scale};
use super::mesh::{AlphaMode, Model, skin};
use prism_doc::figure::FigureScene;
use prism_geom::Point;
use std::sync::Arc;

/// A projected triangle: document positions, and 1/z, uv/z, normal/z per corner for
/// perspective-correct interpolation (z: camera depth).
struct Tri {
    p: [[f64; 2]; 3],
    iz: [f64; 3],
    uvz: [[f64; 2]; 3],
    nz: [V3; 3],
    model: usize,
    material: u16,
    /// Document bounding box: x0, y0, x1, y1.
    bbox: [f64; 4],
}

/// The scene's character meshes, posed and projected, ready to rasterize.
pub(crate) struct Meshes<'a> {
    tris: Vec<Tri>,
    models: Vec<&'a Model>,
}

/// A camera-space corner during near-plane clipping.
#[derive(Clone, Copy)]
struct Corner {
    c: V3,
    uv: [f64; 2],
    n: V3,
}

fn lerp_corner(a: Corner, b: Corner, t: f64) -> Corner {
    let l3 = |x: V3, y: V3| [x[0] + (y[0] - x[0]) * t, x[1] + (y[1] - x[1]) * t, x[2] + (y[2] - x[2]) * t];
    Corner { c: l3(a.c, b.c), uv: [a.uv[0] + (b.uv[0] - a.uv[0]) * t, a.uv[1] + (b.uv[1] - a.uv[1]) * t], n: l3(a.n, b.n) }
}

/// The part of a triangle in front of the near plane (0, 1 or 2 triangles).
fn clip_near(t: [Corner; 3]) -> Vec<[Corner; 3]> {
    let z0 = NEAR * 1.0001;
    let mut poly: Vec<Corner> = Vec::with_capacity(4);
    for i in 0..3 {
        let (a, b) = (t[i], t[(i + 1) % 3]);
        let (ina, inb) = (a.c[2] > z0, b.c[2] > z0);
        if ina {
            poly.push(a);
        }
        if ina != inb {
            let s = (z0 - a.c[2]) / (b.c[2] - a.c[2]);
            poly.push(lerp_corner(a, b, s));
        }
    }
    match poly.as_slice() {
        [a, b, c] => vec![[*a, *b, *c]],
        [a, b, c, d] => vec![[*a, *b, *c], [*a, *c, *d]],
        _ => Vec::new(),
    }
}

impl<'a> Meshes<'a> {
    /// Pose and project every figure that has a model (`models[i]` belongs to figure `i`).
    pub(crate) fn build(scene: &FigureScene, models: &'a [Option<Arc<Model>>], view: &View) -> Meshes<'a> {
        let mut tris = Vec::new();
        let mut used = Vec::new();
        for (f, m) in scene.figures.iter().zip(models) {
            let Some(model) = m.as_deref() else { continue };
            let mi = used.len();
            used.push(model);
            let (pos, nor) = skin(model, f);
            for (t, mat) in model.triangles.iter().zip(&model.triangle_material) {
                let corner = |i: u32| -> Option<Corner> {
                    let i = i as usize;
                    let uv = model.uvs.get(i)?;
                    Some(Corner { c: view.to_camera(*pos.get(i)?), uv: [f64::from(uv[0]), f64::from(uv[1])], n: *nor.get(i)? })
                };
                let (Some(a), Some(b), Some(c)) = (corner(t[0]), corner(t[1]), corner(t[2])) else { continue };
                for part in clip_near([a, b, c]) {
                    let mut p = [[0.0; 2]; 3];
                    let mut iz = [0.0; 3];
                    let mut uvz = [[0.0; 2]; 3];
                    let mut nz = [[0.0; 3]; 3];
                    let mut ok = true;
                    for (k, q) in part.iter().enumerate() {
                        let Some([ix, iy]) = view.image_of(q.c) else {
                            ok = false;
                            break;
                        };
                        let d = view.placement.apply(Point::new(ix, iy));
                        p[k] = [d.x, d.y];
                        let w = 1.0 / q.c[2];
                        iz[k] = w;
                        uvz[k] = [q.uv[0] * w, q.uv[1] * w];
                        nz[k] = scale(q.n, w);
                    }
                    if !ok || p.iter().flatten().any(|v| !v.is_finite()) {
                        continue;
                    }
                    let bbox = [p[0][0].min(p[1][0]).min(p[2][0]), p[0][1].min(p[1][1]).min(p[2][1]), p[0][0].max(p[1][0]).max(p[2][0]), p[0][1].max(p[1][1]).max(p[2][1])];
                    tris.push(Tri { p, iz, uvz, nz, model: mi, material: *mat, bbox });
                }
            }
        }
        Meshes { tris, models: used }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// Document box of every triangle (x0, y0, x1, y1), if any.
    pub(crate) fn bbox(&self) -> Option<[f64; 4]> {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for t in &self.tris {
            b = [b[0].min(t.bbox[0]), b[1].min(t.bbox[1]), b[2].max(t.bbox[2]), b[3].max(t.bbox[3])];
        }
        (b[0] < b[2] && b[1] < b[3]).then_some(b)
    }

    /// The material's alpha mode and the texel/colour alpha at a hit, for cut-outs.
    fn coverage(&self, t: &Tri, uv: [f32; 2]) -> bool {
        let Some(m) = self.models.get(t.model).and_then(|m| m.materials.get(usize::from(t.material))) else { return true };
        let cutoff = match m.alpha {
            AlphaMode::Opaque => return true,
            AlphaMode::Mask(c) => c,
            AlphaMode::Blend => 0.5,
        };
        let tex = m.texture.and_then(|i| self.models.get(t.model).and_then(|md| md.textures.get(i))).map_or(1.0, |tx| tx.sample(uv)[3]);
        tex * m.color[3] >= cutoff
    }

    /// Rasterize onto the grid of sample points `(x0 + c·step, y0 + r·step)`: per point the
    /// nearest depth (infinity when empty) and premultiplied colour.
    pub(crate) fn raster(&self, grid: Grid, light: V3, view: &View) -> Vec<(f64, [f32; 4])> {
        let n = grid.cols.saturating_mul(grid.rows);
        let mut depth = vec![f64::INFINITY; n];
        // Winning triangle and its screen barycentrics per point.
        let mut hit: Vec<Option<(u32, [f64; 3])>> = vec![None; n];
        let (gx1, gy1) = (grid.x0 + grid.step * grid.cols as f64, grid.y0 + grid.step * grid.rows as f64);
        for (ti, t) in self.tris.iter().enumerate() {
            let b = t.bbox;
            if b[2] < grid.x0 - grid.step || b[0] > gx1 || b[3] < grid.y0 - grid.step || b[1] > gy1 {
                continue;
            }
            let [a, bb, c] = t.p;
            let area = (bb[0] - a[0]) * (c[1] - a[1]) - (bb[1] - a[1]) * (c[0] - a[0]);
            if area.abs() < 1e-12 {
                continue;
            }
            let inv = 1.0 / area;
            let col = |x: f64| ((x - grid.x0) / grid.step).max(0.0);
            let (c0, c1) = (col(b[0]).ceil() as usize, (col(b[2]).floor() as usize + 1).min(grid.cols));
            let row = |y: f64| ((y - grid.y0) / grid.step).max(0.0);
            let (r0, r1) = (row(b[1]).ceil() as usize, (row(b[3]).floor() as usize + 1).min(grid.rows));
            for r in r0..r1 {
                let y = grid.y0 + r as f64 * grid.step;
                for cc in c0..c1 {
                    let x = grid.x0 + cc as f64 * grid.step;
                    let w0 = ((bb[0] - x) * (c[1] - y) - (bb[1] - y) * (c[0] - x)) * inv;
                    let w1 = ((c[0] - x) * (a[1] - y) - (c[1] - y) * (a[0] - x)) * inv;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let iz = w0 * t.iz[0] + w1 * t.iz[1] + w2 * t.iz[2];
                    if iz <= 0.0 {
                        continue;
                    }
                    let z = 1.0 / iz;
                    let i = r * grid.cols + cc;
                    if depth.get(i).is_none_or(|d| z >= *d) {
                        continue;
                    }
                    let uv = [((w0 * t.uvz[0][0] + w1 * t.uvz[1][0] + w2 * t.uvz[2][0]) * z) as f32, ((w0 * t.uvz[0][1] + w1 * t.uvz[1][1] + w2 * t.uvz[2][1]) * z) as f32];
                    if !self.coverage(t, uv) {
                        continue;
                    }
                    if let (Some(d), Some(h)) = (depth.get_mut(i), hit.get_mut(i)) {
                        *d = z;
                        *h = Some((ti as u32, [w0, w1, w2]));
                    }
                }
            }
        }
        depth
            .iter()
            .zip(&hit)
            .enumerate()
            .map(|(i, (d, h))| {
                let Some((ti, w)) = h else { return (f64::INFINITY, [0.0; 4]) };
                let Some(t) = self.tris.get(*ti as usize) else { return (f64::INFINITY, [0.0; 4]) };
                let (r, c) = (i / grid.cols.max(1), i % grid.cols.max(1));
                let p = [grid.x0 + c as f64 * grid.step, grid.y0 + r as f64 * grid.step];
                (*d, self.shade(t, *w, *d, p, light, view))
            })
            .collect()
    }

    fn shade(&self, t: &Tri, w: [f64; 3], z: f64, p: [f64; 2], light: V3, view: &View) -> [f32; 4] {
        let uv = [((w[0] * t.uvz[0][0] + w[1] * t.uvz[1][0] + w[2] * t.uvz[2][0]) * z) as f32, ((w[0] * t.uvz[0][1] + w[1] * t.uvz[1][1] + w[2] * t.uvz[2][1]) * z) as f32];
        let n: V3 = std::array::from_fn(|k| (w[0] * t.nz[0][k] + w[1] * t.nz[1][k] + w[2] * t.nz[2][k]) * z);
        let mut n = normalize_or(n, [0.0, 0.0, 1.0]);
        // Double-sided: a face seen from behind is lit like its front.
        let (_, ray) = view.ray(p);
        if dot(n, ray) > 0.0 {
            n = scale(n, -1.0);
        }
        let material = self.models.get(t.model).and_then(|m| m.materials.get(usize::from(t.material)));
        let tex = material.and_then(|m| m.texture.and_then(|i| self.models.get(t.model).and_then(|md| md.textures.get(i)))).map_or([1.0; 4], |tx| tx.sample(uv));
        let base = material.map_or([1.0; 4], |m| m.color);
        // Gentle, toon-friendly light: characters keep their painted colours.
        let shade = (0.62 + 0.38 * dot(n, light).max(0.0)) as f32;
        [tex[0] * base[0] * shade, tex[1] * base[1] * shade, tex[2] * base[2] * shade, 1.0]
    }
}

/// A grid of sample points: `(x0 + c·step, y0 + r·step)` for `c < cols`, `r < rows`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Grid {
    pub x0: f64,
    pub y0: f64,
    pub step: f64,
    pub cols: usize,
    pub rows: usize,
}
