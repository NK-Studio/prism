//! Character meshes (VRM / glTF models) for 3D figure layers: a skinned triangle mesh in the
//! skeleton's rest pose, with materials and textures, deformed by the figure's pose.
//!
//! The mesh is kept in **rest world space** (centimetres, Y up, facing +Z, Prism's figure
//! convention), and the skeleton it was bound to is kept as a Prism joint tree (`rest`): the
//! same tree a figure is posed with. Skinning therefore needs only each joint's rigid change from
//! its rest place to its posed place (`posed · rest⁻¹`), matched by joint name, whatever the
//! original file's bind matrices and scales were (the reader bakes those in).
use super::math::{IDENTITY, Q, V3, add, normalize_or, qconj, qmul, rotate, scale, sub};
use super::skeleton::{Posed, pose};
use prism_doc::figure::{Figure, Joint};
use rayon::prelude::*;
use std::collections::HashMap;

/// Most vertices and triangles a model may hold.
pub const MAX_VERTICES: usize = 4_000_000;
pub const MAX_TRIANGLES: usize = 2_000_000;
/// Largest texture side.
pub const MAX_TEXTURE: u32 = 8192;

/// An RGBA8 texture (straight alpha, the file's colour encoding).
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Texture {
    /// Bilinear sample at `uv` (repeating), as 0..1 RGBA.
    pub fn sample(&self, uv: [f32; 2]) -> [f32; 4] {
        let (w, h) = (self.width.max(1) as f32, self.height.max(1) as f32);
        let wrap = |v: f32| if v.is_finite() { v - v.floor() } else { 0.0 };
        let (x, y) = (wrap(uv[0]) * w - 0.5, wrap(uv[1]) * h - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |xi: f32, yi: f32| -> [f32; 4] {
            let xi = (xi as i64).rem_euclid(i64::from(self.width.max(1))) as usize;
            let yi = (yi as i64).rem_euclid(i64::from(self.height.max(1))) as usize;
            let i = (yi * self.width as usize + xi) * 4;
            match self.rgba.get(i..i + 4) {
                Some(p) => [f32::from(p[0]) / 255.0, f32::from(p[1]) / 255.0, f32::from(p[2]) / 255.0, f32::from(p[3]) / 255.0],
                None => [1.0; 4],
            }
        };
        let (a, b, c, d) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
        let mut out = [0.0; 4];
        for (k, o) in out.iter_mut().enumerate() {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bottom = c[k] + (d[k] - c[k]) * fx;
            *o = top + (bottom - top) * fy;
        }
        out
    }
}

/// How a material's alpha is used.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AlphaMode {
    Opaque,
    /// Drawn where alpha ≥ the cutoff (hair cards, lashes).
    Mask(f32),
    /// Semi-transparent in the file; drawn as a cut-out at half alpha.
    Blend,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    /// Base colour factor (linear multiplier on the texture), RGBA.
    pub color: [f32; 4],
    /// Index into [`Model::textures`].
    pub texture: Option<usize>,
    pub alpha: AlphaMode,
}

impl Default for Material {
    fn default() -> Self {
        Self { color: [1.0; 4], texture: None, alpha: AlphaMode::Opaque }
    }
}

/// A skinned character mesh in rest world space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    /// The character's name (from its file's metadata, else the file name).
    pub title: String,
    pub positions: Vec<V3>,
    pub normals: Vec<V3>,
    pub uvs: Vec<[f32; 2]>,
    /// Up to four influences per vertex: indices into [`Model::joints`] and weights summing to 1
    /// (all zero: the vertex follows nothing and stays at rest).
    pub influences: Vec<[(u16, f32); 4]>,
    pub triangles: Vec<[u32; 3]>,
    /// Material of each triangle (index into [`Model::materials`]).
    pub triangle_material: Vec<u16>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    /// Names of the skeleton joints the influences refer to (joints of [`Model::rest`]).
    pub joints: Vec<String>,
    /// The skeleton in its rest pose; a figure using this model starts from it.
    pub rest: Joint,
}

impl Model {
    /// Check sizes and every index, so rendering can trust them.
    pub fn validate(&self) -> Result<(), String> {
        let n = self.positions.len();
        if n > MAX_VERTICES || self.triangles.len() > MAX_TRIANGLES {
            return Err("the model has too many vertices or triangles".into());
        }
        if self.normals.len() != n || self.uvs.len() != n || self.influences.len() != n {
            return Err("the model's vertex attributes disagree in length".into());
        }
        if self.triangle_material.len() != self.triangles.len() {
            return Err("the model's triangles and materials disagree".into());
        }
        if self.positions.iter().any(|p| p.iter().any(|v| !v.is_finite() || v.abs() > 1e7)) {
            return Err("the model has a vertex out of range".into());
        }
        if self.triangles.iter().flatten().any(|i| *i as usize >= n) {
            return Err("a triangle refers to a missing vertex".into());
        }
        if self.triangle_material.iter().any(|m| usize::from(*m) >= self.materials.len()) {
            return Err("a triangle refers to a missing material".into());
        }
        if self.materials.iter().any(|m| m.texture.is_some_and(|t| t >= self.textures.len())) {
            return Err("a material refers to a missing texture".into());
        }
        for t in &self.textures {
            let need = (t.width as usize).checked_mul(t.height as usize).and_then(|p| p.checked_mul(4));
            if t.width == 0 || t.height == 0 || t.width > MAX_TEXTURE || t.height > MAX_TEXTURE || need != Some(t.rgba.len()) {
                return Err("the model has a malformed texture".into());
            }
        }
        if self.influences.iter().flatten().any(|(j, w)| usize::from(*j) >= self.joints.len().max(1) || !w.is_finite()) {
            return Err("a vertex refers to a missing joint".into());
        }
        Figure { root: self.rest.clone(), ..Default::default() }.validate()
    }

    /// Bytes this model holds, for cache budgets.
    pub fn byte_size(&self) -> usize {
        self.positions.len() * (24 * 2 + 8 + 32) + self.triangles.len() * 14 + self.textures.iter().map(|t| t.rgba.len()).sum::<usize>()
    }
}

/// A joint's rigid change from rest to posed: `p ↦ rot·(p − from) + to`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Delta {
    rot: Q,
    from: V3,
    to: V3,
}

impl Delta {
    const NONE: Delta = Delta { rot: IDENTITY, from: [0.0; 3], to: [0.0; 3] };
}

fn deltas(model: &Model, posed: &[Posed]) -> Vec<Delta> {
    let rest = pose(&Figure { root: model.rest.clone(), ..Default::default() });
    let by_name = |list: &[Posed]| -> HashMap<String, (V3, Q)> { list.iter().map(|p| (p.name.clone(), (p.pos, p.rot))).collect() };
    let (rest, posed) = (by_name(&rest), by_name(posed));
    model
        .joints
        .iter()
        .map(|name| match (rest.get(name), posed.get(name)) {
            (Some((rp, rq)), Some((pp, pq))) => Delta { rot: qmul(*pq, qconj(*rq)), from: *rp, to: *pp },
            _ => Delta::NONE,
        })
        .collect()
}

/// The model's vertices and normals posed like `figure` (whose joints carry the model's joint
/// names; joints it lacks leave their vertices at rest).
pub fn skin(model: &Model, figure: &Figure) -> (Vec<V3>, Vec<V3>) {
    let d = deltas(model, &pose(figure));
    model
        .positions
        .par_iter()
        .zip(model.normals.par_iter())
        .zip(model.influences.par_iter())
        .map(|((p, n), inf)| {
            let (mut pos, mut nor, mut total) = ([0.0; 3], [0.0; 3], 0.0f64);
            for (j, w) in inf {
                if *w <= 0.0 {
                    continue;
                }
                let delta = d.get(usize::from(*j)).copied().unwrap_or(Delta::NONE);
                let w = f64::from(*w);
                pos = add(pos, scale(add(rotate(delta.rot, sub(*p, delta.from)), delta.to), w));
                nor = add(nor, scale(rotate(delta.rot, *n), w));
                total += w;
            }
            if total <= 1e-9 {
                return (*p, *n);
            }
            (scale(pos, 1.0 / total), normalize_or(nor, *n))
        })
        .unzip()
}

/// The axis-aligned box (min, max) of the model posed like `figure`.
pub fn posed_bounds(model: &Model, figure: &Figure) -> Option<(V3, V3)> {
    let (pos, _) = skin(model, figure);
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in &pos {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    lo.iter().zip(&hi).all(|(a, b)| a <= b).then_some((lo, hi))
}

#[cfg(test)]
#[path = "mesh_tests.rs"]
mod tests;
