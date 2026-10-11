//! Rendered pixel regression for corner dragging on the GPU and CPU fallback canvas.
use egui::{Event, Modifiers, PointerButton, Pos2};
use prism_ui_egui::{PrismApp, canvas::ViewXform};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, PrismApp>;

fn harness(cpu: bool) -> Option<Harness> {
    match std::panic::catch_unwind(|| {
        egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 700.0)).with_max_steps(64).wgpu().build_eframe(move |cc| {
            PrismApp::setup_context(&cc.egui_ctx, Default::default());
            let mut a = PrismApp::new(prism_engine::Session::new(), Default::default());
            if !cpu && let Some(rs) = &cc.wgpu_render_state {
                a.set_wgpu(rs.clone());
            }
            a.run("file.new", json!({"width":300,"height":200,"background":"white"})).unwrap();
            a.run("shape.create", json!({"kind":"roundedRect","rect":[20,30,200,100],"radii":20,"fill":"#0000ff"})).unwrap();
            a.ui.tool = prism_ui_egui::state::Tool::Rectangle;
            a.ui.extras.snap = false;
            a.ui.extras.rulers = false;
            a.sync_views();
            a.ui.views[0].zoom = 1.0;
            a.ui.views[0].center = [150.0, 100.0];
            a
        })
    }) {
        Ok(h) => Some(h),
        Err(_) => {
            eprintln!("skipping: no GPU adapter");
            None
        }
    }
}

fn screen(h: &Harness, x: f32, y: f32) -> Pos2 {
    let a = h.state();
    let v = &a.ui.views[0];
    ViewXform {
        rect: prism_ui_egui::rulers::content_rect(a, a.last_canvas_rect),
        zoom: v.zoom,
        center: v.center,
        flip: a.ui.view.flip_horizontal,
        rotation: v.rotation,
    }
    .to_screen(x, y)
}

fn pixel(h: &mut Harness, x: f32, y: f32) -> [u8; 3] {
    let p = screen(h, x, y);
    let image = h.render().unwrap();
    let pixel = image.get_pixel(p.x.round() as u32, p.y.round() as u32).0;
    [pixel[0], pixel[1], pixel[2]]
}

fn sample(h: &mut Harness) -> [u8; 3] {
    pixel(h, 28.0, 38.0)
}

fn button(h: &mut Harness, x: f32, y: f32, pressed: bool) {
    let pos = screen(h, x, y);
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

#[test]
fn corner_drag_updates_filled_pixels_and_escape_restores_gpu_and_cpu_canvas_caches() {
    for cpu in [false, true] {
        let Some(mut h) = harness(cpu) else { return };
        h.run_steps(4);
        let history = h.state().session.active().unwrap().history.past_len();
        let original = sample(&mut h);
        assert!(original[2] > 200 && original[0] < 40, "blue before drag (cpu={cpu}): {original:?}");
        button(&mut h, 40.0, 50.0, true);
        h.event(Event::PointerMoved(screen(&h, 60.0, 70.0)));
        h.run_steps(4);
        let preview = sample(&mut h);
        assert!(preview.iter().all(|c| *c > 220), "white outside rounded corner (cpu={cpu}): {preview:?}");
        let old_outline = pixel(&mut h, 26.0, 36.0);
        assert!(old_outline.iter().all(|c| *c > 220), "the old radius outline must disappear (cpu={cpu}): {old_outline:?}");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
        h.key_press(egui::Key::Escape);
        h.run_steps(4);
        assert_eq!(sample(&mut h), original, "cancel restores cached pixels (cpu={cpu})");
        button(&mut h, 80.0, 90.0, false);
        h.run_steps(3);
        assert_eq!(sample(&mut h), original);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
        // The next drag can still commit, and Undo returns the exact original pixels.
        button(&mut h, 40.0, 50.0, true);
        h.event(Event::PointerMoved(screen(&h, 60.0, 70.0)));
        h.run_steps(3);
        button(&mut h, 60.0, 70.0, false);
        h.run_steps(4);
        assert_eq!(sample(&mut h), preview);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        h.run_steps(4);
        assert_eq!(sample(&mut h), original);
    }
}
