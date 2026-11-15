use super::*;
use prism_color::{ColorMode, SampleType};
use prism_doc::figure::{FigureLook, standing_figure};

fn scene(canvas: [f64; 2]) -> FigureScene {
    let mut s = FigureScene { figures: vec![standing_figure()], ..Default::default() };
    frame(&mut s, &[], [canvas[0] / 2.0, canvas[1] / 2.0], canvas[1] * 0.8, true).unwrap();
    s
}

fn px(s: &Surface, x: i32, y: i32) -> Vec<f32> {
    let mut out = vec![0.0; s.format().channels()];
    s.read_pixel(x, y, &mut out);
    out
}

fn alpha_at(s: &Surface, x: i32, y: i32) -> f32 {
    *px(s, x, y).last().unwrap()
}

#[test]
fn a_framed_figure_renders_inside_its_bounds() {
    let s = scene([200.0, 300.0]);
    let b = bounds(&s, &[]).unwrap();
    assert!(b.y0 >= 0 && b.y1 <= 300 && b.x0 >= 0 && b.x1 <= 200, "{b:?}");
    let h = f64::from(b.height());
    assert!((200.0..=280.0).contains(&h), "framed to about 80% of the height: {h}");
    let out = render(&s, &[], PixelFormat::RGBA8, Rect::new(-200, -300, 400, 600), RenderOptions::default(), &Interrupt::NONE).unwrap();
    let cb = prism_raster::Surface::content_bounds(&out);
    assert!(!cb.is_empty() && b.intersect(&cb) == cb, "content {cb:?} inside bounds {b:?}");
    // The belly is solid, a corner is empty.
    let belly = joint_points(&s).into_iter().find(|j| j.name == "spine1_bb_").unwrap().doc;
    let (bx, by) = (belly[0] as i32, belly[1] as i32);
    assert!(alpha_at(&out, bx, by) > 0.99);
    assert_eq!(alpha_at(&out, 2, 2), 0.0);
    let p = px(&out, bx, by);
    assert!(p[0] > 0.15 && p[0] < 0.95 && (p[0] - p[2]).abs() < 0.1, "shaded gray: {p:?}");
}

#[test]
fn every_depth_and_colour_model_gets_the_same_coverage() {
    let s = scene([120.0, 160.0]);
    let allowed = Rect::new(0, 0, 120, 160);
    let reference = render(&s, &[], PixelFormat::RGBA8, allowed, RenderOptions { draft: true }, &Interrupt::NONE).unwrap();
    for format in [
        PixelFormat::RGBA16,
        PixelFormat::RGBA32F,
        PixelFormat::GRAYA8,
        PixelFormat::CMYKA8,
        PixelFormat::new(ColorMode::Lab, SampleType::U16, true),
    ] {
        let out = render(&s, &[], format, allowed, RenderOptions { draft: true }, &Interrupt::NONE).unwrap();
        for (x, y) in [(60, 80), (60, 20), (5, 5), (60, 150)] {
            assert!((alpha_at(&out, x, y) - alpha_at(&reference, x, y)).abs() < 0.01, "{format:?} at {x},{y}");
        }
    }
}

#[test]
fn moving_the_placement_moves_the_render_exactly() {
    let s = scene([120.0, 160.0]);
    let mut moved = s.clone();
    pan(&mut moved, [10.0, 7.0]);
    let allowed = Rect::new(-50, -50, 250, 250);
    let a = render(&s, &[], PixelFormat::RGBA8, allowed, RenderOptions::default(), &Interrupt::NONE).unwrap();
    let b = render(&moved, &[], PixelFormat::RGBA8, allowed, RenderOptions::default(), &Interrupt::NONE).unwrap();
    for (x, y) in [(60, 80), (55, 30), (70, 120), (40, 100)] {
        assert_eq!(px(&a, x, y), px(&b, x + 10, y + 7), "at {x},{y}");
    }
    // Scaling the placement by two doubles the figure.
    let mut big = s.clone();
    big.placement = Affine::scale(2.0).mul(&s.placement);
    let (bb, sb) = (bounds(&big, &[]).unwrap(), bounds(&s, &[]).unwrap());
    assert!((f64::from(bb.height()) / f64::from(sb.height()) - 2.0).abs() < 0.1);
}

#[test]
fn the_nearer_solid_wins_and_lines_mark_depth_jumps() {
    let s = scene([100.0, 100.0]);
    let view = View::new(&s.camera, s.placement).unwrap();
    let t = s.camera.target;
    let near_ball = Solid { shape: Shape::Capsule { a: add(t, [0.0, 0.0, 30.0]), b: add(t, [0.0, 0.0, 30.0]), r: 5.0 }, bbox: [-1e9, -1e9, 1e9, 1e9] };
    let far_ball = Solid { shape: Shape::Capsule { a: t, b: t, r: 20.0 }, bbox: [-1e9, -1e9, 1e9, 1e9] };
    let light = [0.0, 0.0, 1.0];
    let (centre, _) = view.project(t).unwrap();
    let (d_both, _) = sample(&view, &[&far_ball, &near_ball], light, [1.0; 3], centre);
    let (d_far, _) = sample(&view, &[&far_ball], light, [1.0; 3], centre);
    assert!(d_both < d_far - 10.0, "the near ball hides the far one");
    // With outlines, the near ball's rim is darker than its centre; without, it is not drawn.
    let mut lined = s.clone();
    lined.look = FigureLook { outline: true, outline_width: 2.0, ..FigureLook::default() };
    let mut plain = lined.clone();
    plain.look.outline = false;
    let allowed = Rect::new(0, 0, 100, 100);
    let a = render(&lined, &[], PixelFormat::RGBA8, allowed, RenderOptions::default(), &Interrupt::NONE).unwrap();
    let b = render(&plain, &[], PixelFormat::RGBA8, allowed, RenderOptions::default(), &Interrupt::NONE).unwrap();
    let edge = prism_raster::Surface::content_bounds(&b);
    let mid_y = (edge.y0 + edge.y1) / 2;
    let x = (edge.x0..edge.x1).find(|x| alpha_at(&b, *x, mid_y) > 0.99).unwrap();
    assert!(px(&a, x, mid_y)[0] < px(&b, x, mid_y)[0] - 0.05, "outlined edge is darker");
}

#[test]
fn hostile_scenes_are_errors_or_empty() {
    let empty = FigureScene::default();
    assert!(bounds(&empty, &[]).is_none());
    assert!(render(&empty, &[], PixelFormat::RGBA8, Rect::new(0, 0, 10, 10), RenderOptions::default(), &Interrupt::NONE).unwrap().content_bounds().is_empty());
    assert!(frame(&mut empty.clone(), &[], [0.0, 0.0], 100.0, true).is_err());
    let mut bad = scene([100.0, 100.0]);
    bad.placement = Affine { m: [0.0; 6] };
    assert!(render(&bad, &[], PixelFormat::RGBA8, Rect::new(0, 0, 10, 10), RenderOptions::default(), &Interrupt::NONE).is_err());
    let mut huge = scene([100.0, 100.0]);
    huge.placement = Affine::scale(1e5).mul(&huge.placement);
    assert!(render(&huge, &[], PixelFormat::RGBA8, Rect::new(-1_000_000_000, -1_000_000_000, 1_000_000_000, 1_000_000_000), RenderOptions::default(), &Interrupt::NONE).is_err());
    // A camera inside the figure leaves out what touches the eye instead of failing.
    let mut inside = scene([100.0, 100.0]);
    inside.camera.distance = FigureCamera::MIN_DISTANCE;
    assert!(render(&inside, &[], PixelFormat::RGBA8, Rect::new(0, 0, 100, 100), RenderOptions { draft: true }, &Interrupt::NONE).is_ok());
    assert!(rotate_joint(&mut inside, 9, "hips_bb_", [0.0, 1.0, 0.0], 10.0).is_err());
    assert!(rotate_joint(&mut inside, 0, "hips_bb_", [0.0; 3], 10.0).is_err());
    assert!(rotate_joint(&mut inside, 0, "hips_bb_", [0.0, 1.0, 0.0], f64::NAN).is_err());
}

#[test]
fn framing_faces_the_figure_and_the_camera_moves() {
    let mut s = FigureScene { figures: vec![standing_figure()], ..Default::default() };
    // Turn the figure to face +X: framing turns the camera round to its front.
    ik::rotate_joint(&mut s.figures[0], "root_group_", axis_angle([0.0, 1.0, 0.0], std::f64::consts::FRAC_PI_2)).unwrap();
    frame(&mut s, &[], [50.0, 50.0], 80.0, true).unwrap();
    assert!((s.camera.yaw - 90.0).abs() < 1e-6, "{}", s.camera.yaw);
    let before = s.camera;
    orbit(&mut s, 400.0, 200.0);
    assert!((s.camera.yaw - (before.yaw + 40.0)).abs() < 1e-9 && s.camera.pitch == FigureCamera::MAX_PITCH);
    dolly(&mut s, 0.5);
    assert!((s.camera.distance - before.distance * 0.5).abs() < 1e-9);
    dolly(&mut s, f64::NAN);
    // Back to a level view of the figure's front for the handle checks.
    s.camera.pitch = 0.0;
    frame(&mut s, &[], [50.0, 50.0], 80.0, true).unwrap();
    let joints = joint_points(&s);
    assert_eq!(joints.len(), 67);
    assert!(joints.iter().any(|j| j.name == "lefthand_bb_" && j.parent.as_deref() == Some("leftforearm_bb_")));
    // Reaching on screen keeps the hand's depth and moves it to the point (towards the shoulder:
    // the standing arm is straight, so it can't reach further out).
    let hand = joints.iter().find(|j| j.name == "lefthand_bb_").unwrap();
    let goal = [hand.doc[0], hand.doc[1] - 15.0];
    reach_to(&mut s, 0, "lefthand_bb_", goal).unwrap();
    let hand = joint_points(&s).into_iter().find(|j| j.name == "lefthand_bb_").unwrap();
    assert!((hand.doc[0] - goal[0]).abs() < 1e-6 && (hand.doc[1] - goal[1]).abs() < 1e-6, "{:?} vs {goal:?}", hand.doc);
}
