use super::*;
use crate::figure3d::math::axis_angle;
use crate::figure3d::{RenderOptions, bounds, frame, render};
use prism_color::PixelFormat;
use prism_doc::figure::FigureScene;
use prism_geom::Rect;
use prism_raster::Interrupt;
use std::sync::Arc;

fn joint(name: &str, offset: V3, children: Vec<Joint>) -> Joint {
    Joint { name: name.into(), rotation: IDENTITY, offset, children }
}

/// A 2 × 20 cm strip standing on the origin, its lower half on joint `a`, its upper half on `b`
/// (hanging 10 cm above `a`), one texture: red left half, blue right half.
fn strip() -> Model {
    let rest = joint("root_group_", [0.0; 3], vec![joint("a", [0.0; 3], vec![joint("b", [0.0, 10.0, 0.0], vec![joint("tip", [0.0, 10.0, 0.0], vec![])])])]);
    let mut m = Model { rest, joints: vec!["a".into(), "b".into()], ..Default::default() };
    for (k, y) in [0.0, 10.0, 20.0].into_iter().enumerate() {
        for x in [-1.0, 1.0] {
            m.positions.push([x, y, 0.0]);
            m.normals.push([0.0, 0.0, 1.0]);
            m.uvs.push([if x < 0.0 { 0.25 } else { 0.75 }, 0.5]);
            let j = if k == 0 { 0 } else if k == 2 { 1 } else { 0 };
            m.influences.push([(j, 1.0), (0, 0.0), (0, 0.0), (0, 0.0)]);
        }
    }
    m.triangles = vec![[0, 1, 3], [0, 3, 2], [2, 3, 5], [2, 5, 4]];
    m.triangle_material = vec![0; 4];
    m.textures = vec![Texture { width: 2, height: 1, rgba: vec![255, 0, 0, 255, 0, 0, 255, 255] }];
    m.materials = vec![Material { texture: Some(0), ..Default::default() }];
    m
}

fn figure(m: &Model) -> Figure {
    Figure { root: m.rest.clone(), ..Default::default() }
}

#[test]
fn skinning_follows_the_joints() {
    let m = strip();
    m.validate().unwrap();
    let mut f = figure(&m);
    let (rest, _) = skin(&m, &f);
    assert_eq!(rest, m.positions, "unposed: the rest mesh");
    // Turn `b` a quarter about Z: the top row swings to −X around (0, 10).
    f.root.find_mut("b").unwrap().rotation = axis_angle([0.0, 0.0, 1.0], std::f64::consts::FRAC_PI_2);
    let (posed, normals) = skin(&m, &f);
    for i in [4, 5] {
        let p = posed[i];
        assert!((p[0] + 10.0).abs() < 1e-9 && (p[1] - 10.0 - m.positions[i][0]).abs() < 1e-9, "{i}: {p:?}");
    }
    assert_eq!(posed[0], m.positions[0], "joint `a` did not move");
    assert!((normals[4][2] - 1.0).abs() < 1e-9, "a turn about the normal keeps it");
    // A joint the figure lacks leaves its vertices at rest.
    let mut bare = f.clone();
    bare.root.children.clear();
    assert_eq!(skin(&m, &bare).0, m.positions);
}

#[test]
fn bad_models_are_refused() {
    let mut m = strip();
    m.triangles.push([0, 1, 99]);
    m.triangle_material.push(0);
    assert!(m.validate().is_err());
    let mut m = strip();
    m.materials[0].texture = Some(3);
    assert!(m.validate().is_err());
    let mut m = strip();
    m.textures[0].rgba.pop();
    assert!(m.validate().is_err());
    let mut m = strip();
    m.influences[0][0].0 = 7;
    assert!(m.validate().is_err());
    let mut m = strip();
    m.positions[0][1] = f64::NAN;
    assert!(m.validate().is_err());
}

#[test]
fn textures_sample_bilinearly_and_wrap() {
    let t = &strip().textures[0];
    assert_eq!(t.sample([0.25, 0.5]), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(t.sample([0.75, 0.5]), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(t.sample([1.25, -3.5]), [1.0, 0.0, 0.0, 1.0], "repeats");
    let mid = t.sample([0.5, 0.5]);
    assert!((mid[0] - 0.5).abs() < 1e-6 && (mid[2] - 0.5).abs() < 1e-6);
}

fn scene_of(m: &Model) -> (FigureScene, Vec<Option<Arc<Model>>>) {
    let mut s = FigureScene { figures: vec![figure(m)], ..Default::default() };
    // The strip is only a few pixels wide: no outline over it.
    s.look.outline = false;
    let models = vec![Some(Arc::new(m.clone()))];
    frame(&mut s, &models, [50.0, 50.0], 80.0, true).unwrap();
    (s, models)
}

fn pixel(surface: &Surface, x: i32, y: i32) -> [f32; 4] {
    let mut px = [0.0f32; 4];
    surface.read_pixel(x, y, &mut px);
    px
}
use prism_raster::Surface;

#[test]
fn a_model_renders_its_texture_where_its_mesh_is() {
    let m = strip();
    let (s, models) = scene_of(&m);
    let b = bounds(&s, &models).unwrap();
    assert!(b.height() > 60 && b.width() > 3 && b.width() < 30, "a tall thin strip: {b:?}");
    let out = render(&s, &models, PixelFormat::RGBA8, Rect::new(0, 0, 100, 100), RenderOptions::default(), &Interrupt::NONE).unwrap();
    let (cx, cy) = (b.x0 + b.width() as i32 / 2, b.y0 + b.height() as i32 / 2);
    let left = pixel(&out, cx - 2, cy);
    let right = pixel(&out, cx + 2, cy);
    // UVs run 0.25 → 0.75 across, so the colour blends from red to blue.
    assert!(left[3] > 0.99 && left[0] > left[2] + 0.2 && left[1] < 0.05, "red side: {left:?}");
    assert!(right[3] > 0.99 && right[2] > right[0] + 0.2 && right[1] < 0.05, "blue side: {right:?}");
    assert_eq!(pixel(&out, b.x0 - 5, cy)[3], 0.0, "nothing beside it");
    // Drafts and every depth render too.
    for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        let d = render(&s, &models, format, Rect::new(0, 0, 100, 100), RenderOptions { draft: true }, &Interrupt::NONE).unwrap();
        assert!(pixel(&d, cx - 2, cy)[3] > 0.99, "{format:?}");
    }
}

#[test]
fn a_model_and_a_mannequin_share_one_depth_order() {
    let m = strip();
    let mut s = FigureScene { figures: vec![prism_doc::figure::standing_figure(), figure(&m)], ..Default::default() };
    // The strip stands 30 cm in front of the mannequin's chest.
    s.figures[1].root.offset = [0.0, 110.0, 30.0];
    let models = vec![None, Some(Arc::new(m))];
    s.look.outline = false;
    frame(&mut s, &models, [100.0, 100.0], 180.0, true).unwrap();
    let out = render(&s, &models, PixelFormat::RGBA8, Rect::new(0, 0, 200, 200), RenderOptions::default(), &Interrupt::NONE).unwrap();
    let chest = crate::figure3d::joint_points(&s).into_iter().find(|j| j.figure == 1 && j.name == "b").unwrap().doc;
    let px = pixel(&out, chest[0].round() as i32 - 1, chest[1].round() as i32);
    assert!(px[3] > 0.99 && px[1] < 0.05 && px[0] + px[2] > 0.4, "the strip's colours show in front of the gray body: {px:?}");
}
