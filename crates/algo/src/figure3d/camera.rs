//! The orbit camera: world (cm, Y up) → camera space → image plane → document pixels.
use super::math::{V3, add, cross, dot, normalize_or, scale, sub};
use prism_doc::figure::FigureCamera;
use prism_geom::{Affine, Point};

/// Nothing closer to the eye than this (centimetres) is drawn.
pub const NEAR: f64 = 1.0;

/// A camera ready to project points and cast rays.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub eye: V3,
    pub right: V3,
    pub up: V3,
    /// Viewing direction.
    pub fwd: V3,
    /// Image-plane units per unit of tangent.
    pub focal: f64,
    pub placement: Affine,
    pub inverse: Affine,
}

impl View {
    /// `None` when the placement cannot be inverted.
    pub fn new(cam: &FigureCamera, placement: Affine) -> Option<View> {
        let (yaw, pitch) = (cam.yaw.to_radians(), cam.pitch.clamp(-FigureCamera::MAX_PITCH, FigureCamera::MAX_PITCH).to_radians());
        let dir = [pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos()];
        let distance = cam.distance.clamp(FigureCamera::MIN_DISTANCE, FigureCamera::MAX_DISTANCE);
        let eye = add(cam.target, scale(dir, distance));
        let fwd = normalize_or(sub(cam.target, eye), [0.0, 0.0, -1.0]);
        let right = normalize_or(cross(fwd, [0.0, 1.0, 0.0]), [1.0, 0.0, 0.0]);
        let up = cross(right, fwd);
        let fov = cam.fov_deg.clamp(FigureCamera::MIN_FOV, FigureCamera::MAX_FOV).to_radians();
        let focal = FigureCamera::FOCAL_SPAN / 2.0 / (fov / 2.0).tan();
        let inverse = placement.inverse()?;
        Some(View { eye, right, up, fwd, focal, placement, inverse })
    }

    /// Camera space: x right, y up, z depth along the view.
    pub fn to_camera(&self, p: V3) -> V3 {
        let d = sub(p, self.eye);
        [dot(d, self.right), dot(d, self.up), dot(d, self.fwd)]
    }

    /// Image plane (y down) of a camera-space point in front of the near plane.
    pub fn image_of(&self, c: V3) -> Option<[f64; 2]> {
        (c[2] > NEAR).then(|| [self.focal * c[0] / c[2], -self.focal * c[1] / c[2]])
    }

    /// Document pixel of a world point, with its depth; `None` behind the near plane.
    pub fn project(&self, p: V3) -> Option<([f64; 2], f64)> {
        let c = self.to_camera(p);
        let [ix, iy] = self.image_of(c)?;
        let d = self.placement.apply(Point::new(ix, iy));
        Some(([d.x, d.y], c[2]))
    }

    /// The ray through a document pixel: origin and unit direction (world).
    pub fn ray(&self, doc: [f64; 2]) -> (V3, V3) {
        let i = self.inverse.apply(Point::new(doc[0], doc[1]));
        let d = add(add(scale(self.right, i.x / self.focal), scale(self.up, -i.y / self.focal)), self.fwd);
        (self.eye, normalize_or(d, self.fwd))
    }

    /// The world point under a document pixel at camera depth `depth`.
    pub fn unproject(&self, doc: [f64; 2], depth: f64) -> V3 {
        let i = self.inverse.apply(Point::new(doc[0], doc[1]));
        let c = [i.x * depth / self.focal, -i.y * depth / self.focal, depth];
        add(self.eye, add(add(scale(self.right, c[0]), scale(self.up, c[1])), scale(self.fwd, c[2])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::figure3d::math::len;

    #[test]
    fn project_and_unproject_agree_and_the_default_view_looks_at_the_front() {
        let cam = FigureCamera::default();
        let v = View::new(&cam, Affine::translate(500.0, 400.0)).unwrap();
        // Default camera: in front of the figure (+Z), looking along −Z, X to the right.
        assert!(v.eye[2] > 0.0 && v.fwd[2] < -0.99 && v.right[0] > 0.99 && v.up[1] > 0.99);
        let (doc, depth) = v.project(cam.target).unwrap();
        assert!((doc[0] - 500.0).abs() < 1e-9 && (doc[1] - 400.0).abs() < 1e-9, "the target is the image centre");
        let p = [12.0, 130.0, -7.0];
        let (doc, depth2) = v.project(p).unwrap();
        assert!(len(sub(v.unproject(doc, depth2), p)) < 1e-6);
        let (o, d) = v.ray(doc);
        let t = dot(sub(p, o), d);
        assert!(len(sub(add(o, scale(d, t)), p)) < 1e-6, "the ray passes through the point");
        assert!(depth > 0.0);
        // Higher world points are higher on the image (smaller y).
        let (hi, _) = v.project([0.0, 150.0, 0.0]).unwrap();
        let (lo, _) = v.project([0.0, 50.0, 0.0]).unwrap();
        assert!(hi[1] < lo[1]);
        assert!(v.project(v.eye).is_none(), "behind the near plane");
        assert!(View::new(&cam, Affine { m: [0.0; 6] }).is_none());
    }
}
