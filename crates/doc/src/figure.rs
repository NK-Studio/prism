//! 3D figure layers: posable mannequins, as CLIP STUDIO PAINT's 3D drawing figures. Pure data;
//! the renderer is `prism_algo::figure3d`, the commands `prism_engine::figure_cmds`. A layer with
//! [`Layer::figure`](crate::Layer::figure) keeps its rendered pixels as ordinary raster content,
//! so everything that reads pixels works unchanged; editing the scene re-renders them.
//!
//! Space: centimetres, Y up, right-handed. A joint's `offset` is its position in its parent's
//! frame and `rotation` its local rotation (unit quaternion `[w, x, y, z]`), the convention of
//! CLIP STUDIO's `.pep` pose files, whose joint names Prism keeps so poses map onto the figure.

use prism_geom::Affine;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Largest character model file a figure may carry (bytes).
pub const MAX_MODEL_BYTES: usize = 256 << 20;

/// A character model a figure is drawn with instead of Prism's mannequin: the original VRM /
/// glTF file, kept whole (`.prism` stores it as a blob; it is parsed when drawn). The figure's
/// joints are the model's skeleton.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FigureModel {
    /// The file's name (`character.vrm`), which also tells its kind.
    pub file_name: String,
    #[serde(skip)]
    pub bytes: Arc<Vec<u8>>,
}

impl PartialEq for FigureModel {
    fn eq(&self, other: &Self) -> bool {
        self.file_name == other.file_name && (Arc::ptr_eq(&self.bytes, &other.bytes) || self.bytes == other.bytes)
    }
}

/// Joints one figure may hold (a CLIP STUDIO figure has 67; VRM characters with hair and cloth
/// bones run to a few hundred).
pub const MAX_JOINTS: usize = 1024;
/// Deepest joint nesting accepted.
pub const MAX_DEPTH: usize = 64;
/// Figures one layer may hold.
pub const MAX_FIGURES: usize = 16;
/// Longest kept name (characters).
pub const MAX_NAME: usize = 255;
/// Largest coordinate magnitude accepted (centimetres): far beyond any figure.
pub const MAX_COORD: f64 = 1e6;

/// The figures of one layer, the camera they are seen through and how they are drawn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FigureScene {
    pub figures: Vec<Figure>,
    pub camera: FigureCamera,
    pub look: FigureLook,
    /// Camera image plane (see [`FigureCamera::FOCAL`]) to document pixels. Moving, scaling or
    /// rotating the layer composes into it, so a re-render lands where the pixels were.
    pub placement: Affine,
}

impl Default for FigureScene {
    fn default() -> Self {
        Self { figures: Vec::new(), camera: FigureCamera::default(), look: FigureLook::default(), placement: Affine::IDENTITY }
    }
}

/// One posable figure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Figure {
    pub name: String,
    /// Where the pose came from (a CLIP STUDIO material name or file), if anywhere.
    pub source: Option<String>,
    pub root: Joint,
    /// The character drawn for this figure (none: Prism's mannequin).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<FigureModel>,
}

impl Default for Figure {
    fn default() -> Self {
        Self { name: "Figure".into(), source: None, root: Joint::default(), model: None }
    }
}

/// One joint and the joints that hang from it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Joint {
    pub name: String,
    /// Local rotation, unit quaternion `[w, x, y, z]`.
    pub rotation: [f64; 4],
    /// Position in the parent's frame (centimetres).
    pub offset: [f64; 3],
    pub children: Vec<Joint>,
}

impl Default for Joint {
    fn default() -> Self {
        Self { name: String::new(), rotation: [1.0, 0.0, 0.0, 0.0], offset: [0.0; 3], children: Vec::new() }
    }
}

/// An orbit camera around `target`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FigureCamera {
    /// The point the camera looks at (centimetres).
    pub target: [f64; 3],
    /// Turn around the vertical axis, degrees (0 = looking along −Z, from the figure's front).
    pub yaw: f64,
    /// Tilt, degrees (positive looks down from above), within ±89.
    pub pitch: f64,
    /// Distance from the target, centimetres.
    pub distance: f64,
    /// Vertical field of view, degrees.
    pub fov_deg: f64,
}

impl FigureCamera {
    /// The image plane spans this many units over the field of view's height, so a figure's
    /// image size does not depend on the document; [`FigureScene::placement`] scales it.
    pub const FOCAL_SPAN: f64 = 1000.0;
    pub const MIN_DISTANCE: f64 = 1.0;
    pub const MAX_DISTANCE: f64 = 1e5;
    pub const MIN_FOV: f64 = 5.0;
    pub const MAX_FOV: f64 = 120.0;
    pub const MAX_PITCH: f64 = 89.0;
}

impl Default for FigureCamera {
    fn default() -> Self {
        Self { target: [0.0, 90.0, 0.0], yaw: 0.0, pitch: 0.0, distance: 400.0, fov_deg: 35.0 }
    }
}

/// How figures are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FigureLook {
    /// Surface colour, linear 0..1 RGB before shading.
    pub color: [f32; 3],
    /// Draw silhouette and crease lines.
    pub outline: bool,
    /// Line width in document pixels.
    pub outline_width: f64,
}

impl FigureLook {
    pub const MAX_OUTLINE: f64 = 64.0;
}

impl Default for FigureLook {
    fn default() -> Self {
        Self { color: [0.82, 0.82, 0.84], outline: true, outline_width: 1.5 }
    }
}

impl Joint {
    /// Every joint, depth first, with its depth (the root is 0). Stops at [`MAX_DEPTH`].
    pub fn walk(&self) -> Vec<(usize, &Joint)> {
        let mut out = Vec::new();
        let mut stack = vec![(0usize, self)];
        while let Some((d, j)) = stack.pop() {
            out.push((d, j));
            if d < MAX_DEPTH {
                stack.extend(j.children.iter().rev().map(|c| (d + 1, c)));
            }
        }
        out
    }

    /// The joint named `name`, depth first.
    pub fn find(&self, name: &str) -> Option<&Joint> {
        self.walk().into_iter().map(|(_, j)| j).find(|j| j.name == name)
    }

    /// The joint named `name`, mutably (depth first, bounded by [`MAX_DEPTH`]).
    pub fn find_mut(&mut self, name: &str) -> Option<&mut Joint> {
        fn go<'a>(j: &'a mut Joint, name: &str, depth: usize) -> Option<&'a mut Joint> {
            if j.name == name {
                return Some(j);
            }
            if depth >= MAX_DEPTH {
                return None;
            }
            j.children.iter_mut().find_map(|c| go(c, name, depth + 1))
        }
        go(self, name, 0)
    }

    /// The names from the root to `name` (inclusive), or `None` when absent.
    pub fn path_to(&self, name: &str) -> Option<Vec<String>> {
        fn go(j: &Joint, name: &str, depth: usize, path: &mut Vec<String>) -> bool {
            path.push(j.name.clone());
            if j.name == name {
                return true;
            }
            if depth < MAX_DEPTH && j.children.iter().any(|c| go(c, name, depth + 1, path)) {
                return true;
            }
            path.pop();
            false
        }
        let mut path = Vec::new();
        go(self, name, 0, &mut path).then_some(path)
    }
}

fn finite(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite() && x.abs() <= MAX_COORD)
}

/// A unit quaternion from any finite one; identity for a zero or non-finite input.
pub fn normalize_quat(q: [f64; 4]) -> [f64; 4] {
    let n = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    if n.is_finite() && n > 1e-12 { q.map(|v| v / n) } else { [1.0, 0.0, 0.0, 0.0] }
}

impl Figure {
    /// Check counts, depth, names and numbers; a scene that passes renders without surprises.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.chars().count() > MAX_NAME {
            return Err("figure name is too long".into());
        }
        if let Some(m) = &self.model
            && (m.file_name.chars().count() > MAX_NAME || m.bytes.len() > MAX_MODEL_BYTES)
        {
            return Err("figure model is too large".into());
        }
        let mut count = 0usize;
        let mut stack = vec![(0usize, &self.root)];
        while let Some((d, j)) = stack.pop() {
            count += 1;
            if count > MAX_JOINTS {
                return Err(format!("a figure holds at most {MAX_JOINTS} joints"));
            }
            if d > MAX_DEPTH {
                return Err(format!("figure joints nest deeper than {MAX_DEPTH}"));
            }
            if j.name.chars().count() > MAX_NAME {
                return Err("joint name is too long".into());
            }
            if !finite(&j.offset) || !finite(&j.rotation) {
                return Err(format!("joint {:?} has an invalid value", j.name));
            }
            let n = j.rotation.iter().map(|v| v * v).sum::<f64>();
            if !(1e-6..=1e6).contains(&n) {
                return Err(format!("joint {:?} has a zero rotation", j.name));
            }
            stack.extend(j.children.iter().map(|c| (d + 1, c)));
        }
        Ok(())
    }
}

impl FigureScene {
    /// Check every figure, the camera, the look and the placement.
    pub fn validate(&self) -> Result<(), String> {
        if self.figures.len() > MAX_FIGURES {
            return Err(format!("a layer holds at most {MAX_FIGURES} figures"));
        }
        for f in &self.figures {
            f.validate()?;
        }
        let c = &self.camera;
        if !finite(&c.target)
            || !(FigureCamera::MIN_DISTANCE..=FigureCamera::MAX_DISTANCE).contains(&c.distance)
            || !(FigureCamera::MIN_FOV..=FigureCamera::MAX_FOV).contains(&c.fov_deg)
            || !c.yaw.is_finite()
            || !(-FigureCamera::MAX_PITCH..=FigureCamera::MAX_PITCH).contains(&c.pitch)
        {
            return Err("invalid figure camera".into());
        }
        let l = &self.look;
        if !l.color.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)) || !(0.0..=FigureLook::MAX_OUTLINE).contains(&l.outline_width) {
            return Err("invalid figure look".into());
        }
        if !self.placement.m.iter().all(|v| v.is_finite() && v.abs() <= 1e9) || self.placement.determinant().abs() < 1e-12 {
            return Err("invalid figure placement".into());
        }
        Ok(())
    }
}

/// Prism's default figure: a standing person about 170 cm tall facing +Z (its left on +X), with
/// CLIP STUDIO's joint names (so poses map onto it) and Prism's own proportions. Every rotation is
/// the identity; the offsets point each bone where it hangs.
pub fn standing_figure() -> Figure {
    fn j(name: &str, offset: [f64; 3], children: Vec<Joint>) -> Joint {
        Joint { name: name.into(), rotation: [1.0, 0.0, 0.0, 0.0], offset, children }
    }
    // Fingers: four joints each along the hand's direction `s` (±1 for left/right).
    let fingers = |side: &str, s: f64| -> Vec<Joint> {
        let finger = |name: &str, base: [f64; 3], lengths: [f64; 3]| {
            let n = |i: usize| format!("{side}hand{name}{i}_bb_");
            j(&n(1), base, vec![j(&n(2), [0.0, -lengths[0], 0.0], vec![j(&n(3), [0.0, -lengths[1], 0.0], vec![j(&n(4), [0.0, -lengths[2], 0.0], vec![])])])])
        };
        vec![
            finger("thumb", [s * 1.4, -1.2, 1.6], [3.2, 2.8, 2.0]),
            finger("index", [s * 1.3, -6.5, 0.9], [3.3, 2.0, 1.9]),
            finger("middle", [s * 0.3, -6.6, 0.3], [3.5, 2.2, 2.0]),
            finger("ring", [s * -0.7, -6.3, -0.3], [3.3, 2.1, 1.8]),
            finger("pinky", [s * -1.6, -5.8, -0.8], [2.7, 1.8, 1.5]),
        ]
    };
    let arm = |side: &str, s: f64| {
        j(
            &format!("{side}shoulder_bb_"),
            [s * 2.5, 12.0, 0.0],
            vec![j(
                &format!("{side}arm_bb_"),
                [s * 15.0, -1.0, 0.0],
                vec![j(
                    &format!("{side}forearm_bb_"),
                    [0.0, -27.0, 0.0],
                    vec![j(&format!("{side}hand_bb_"), [0.0, -24.0, 0.0], fingers(side, s))],
                )],
            )],
        )
    };
    let leg = |side: &str, s: f64| {
        j(
            &format!("{side}upleg_bb_"),
            [s * 9.0, -6.0, 0.0],
            vec![j(
                &format!("{side}leg_bb_"),
                [0.0, -40.0, 0.0],
                vec![j(
                    &format!("{side}foot_bb_"),
                    [0.0, -40.0, 0.0],
                    vec![j(&format!("{side}toebase_bb_"), [0.0, -7.0, 12.0], vec![j(&format!("{side}toebase_end_bb_"), [0.0, 0.0, 6.0], vec![])])],
                )],
            )],
        )
    };
    let head = j("neck_bb_", [0.0, 15.0, 0.0], vec![j("head_bb_", [0.0, 9.0, 0.0], vec![j("head_end_bb_", [0.0, 21.0, 0.0], vec![])])]);
    let spine = j(
        "spine_bb_",
        [0.0, 2.0, 0.0],
        vec![j("spine1_bb_", [0.0, 10.0, 0.0], vec![j("spine2_bb_", [0.0, 10.0, 0.0], vec![head, arm("left", 1.0), arm("right", -1.0)])])],
    );
    let hips = j("hips_bb_", [0.0, 94.0, 0.0], vec![j("hips_end_bb_", [0.0, -12.0, 0.0], vec![]), spine, leg("left", 1.0), leg("right", -1.0)]);
    Figure { name: "Figure".into(), source: None, root: j("root_group_", [0.0; 3], vec![hips]), model: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standing_figure_is_valid_and_named_like_clip_studio() {
        let f = standing_figure();
        f.validate().unwrap();
        for name in ["hips_bb_", "head_bb_", "lefthand_bb_", "righthandindex3_bb_", "leftfoot_bb_", "righttoebase_end_bb_"] {
            assert!(f.root.find(name).is_some(), "{name}");
        }
        assert_eq!(f.root.walk().len(), 67, "as many joints as a CLIP STUDIO figure");
        assert_eq!(f.root.path_to("leftforearm_bb_").unwrap(), ["root_group_", "hips_bb_", "spine_bb_", "spine1_bb_", "spine2_bb_", "leftshoulder_bb_", "leftarm_bb_", "leftforearm_bb_"]);
        assert!(f.root.path_to("tail_bb_").is_none());
    }

    #[test]
    fn hostile_scenes_fail_validation() {
        let ok = FigureScene { figures: vec![standing_figure()], ..Default::default() };
        ok.validate().unwrap();
        let mut bad = ok.clone();
        bad.figures[0].root.offset[0] = f64::NAN;
        assert!(bad.validate().is_err());
        let mut bad = ok.clone();
        bad.figures[0].root.rotation = [0.0; 4];
        assert!(bad.validate().is_err());
        let mut bad = ok.clone();
        bad.camera.distance = 0.0;
        assert!(bad.validate().is_err());
        let mut bad = ok.clone();
        bad.placement = Affine { m: [0.0; 6] };
        assert!(bad.validate().is_err());
        let mut bad = ok.clone();
        bad.figures = vec![standing_figure(); MAX_FIGURES + 1];
        assert!(bad.validate().is_err());
        // Deep nesting is refused, and walking it stays bounded.
        let mut deep = Joint::default();
        for _ in 0..(MAX_DEPTH + 5) {
            deep = Joint { children: vec![deep], ..Default::default() };
        }
        assert!(Figure { root: deep.clone(), ..Default::default() }.validate().is_err());
        assert!(deep.walk().len() <= MAX_DEPTH + 1);
        assert_eq!(normalize_quat([0.0; 4]), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(normalize_quat([2.0, 0.0, 0.0, 0.0]), [1.0, 0.0, 0.0, 0.0]);
    }
}
