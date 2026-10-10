//! Pixel checks cover live colour changes, Cancel, OK and Undo on both canvas renderers.
use photocraft_ui_egui::{
    PhotocraftApp,
    canvas::ViewXform,
    control::{ControlRequest, Outcome, handle},
};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;

fn call(h: &mut Harness, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params);
    let ctx = h.ctx.clone();
    match handle(h.state_mut(), &ctx, &req) {
        Outcome::Done(value) => {
            assert_eq!(value["ok"], true, "{method}: {value}");
            value["result"].clone()
        }
        _ => panic!("synchronous request"),
    }
}

fn harness(cpu: bool) -> Option<Harness> {
    match std::panic::catch_unwind(|| {
        egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_max_steps(64).wgpu().build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            if !cpu && let Some(rs) = &cc.wgpu_render_state {
                a.set_wgpu(rs.clone());
            }
            a.run("file.new", json!({"width":600,"height":320,"background":"white"})).unwrap();
            a.run("shape.create", json!({"kind":"roundedRect","rect":[20,40,200,200],"radii":20,"fill":"#0000ff"})).unwrap();
            a.sync_views();
            a.ui.extras.rulers = false;
            a.ui.views[0].zoom = 1.0;
            a.ui.views[0].center = [400.0, 160.0];
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

fn pixel(h: &mut Harness) -> [u8; 3] {
    let a = h.state();
    let v = &a.ui.views[0];
    let p = ViewXform { rect: photocraft_ui_egui::rulers::content_rect(a, a.last_canvas_rect), zoom: v.zoom, center: v.center, flip: false, rotation: 0.0 }
        .to_screen(80.0, 120.0);
    let img = h.render().unwrap();
    let p = img.get_pixel(p.x.round() as u32, p.y.round() as u32).0;
    [p[0], p[1], p[2]]
}

#[test]
fn picker_previews_real_pixels_and_cancel_ok_undo_restore_canvas_caches() {
    for cpu in [false, true] {
        let Some(mut h) = harness(cpu) else { return };
        h.run_steps(4);
        let history = h.state().session.active().unwrap().history.past_len();
        let original = pixel(&mut h);
        assert!(original[2] > 200 && original[0] < 40, "blue original: {original:?}");
        let id = call(&mut h, "ui.menu.invoke", json!({"id":"layer.layerContentOptions"}))["dialog"].as_u64().unwrap();
        for color in ["#ff0000", "#00ff00"] {
            call(&mut h, "ui.dialog.set", json!({"dialog":id,"field":"color","value":color}));
            h.run_steps(4);
            let actual = pixel(&mut h);
            let channel = if color == "#ff0000" { 0 } else { 1 };
            assert!(actual[channel] > 200 && actual[2] < 40, "live {color} (cpu={cpu}): {actual:?}");
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
        }
        h.key_press(egui::Key::Escape);
        h.run_steps(4);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(pixel(&mut h), original, "Escape restores canvas (cpu={cpu})");
        let id = call(&mut h, "ui.dialog.open", json!({"kind":"colorPicker","target":"shapeFill"}))["dialog"].as_u64().unwrap();
        call(&mut h, "ui.dialog.set", json!({"dialog":id,"field":"color","value":"#ff0000"}));
        h.run_steps(4);
        let preview = pixel(&mut h);
        // Return to the original colour while the dialog is still open.
        call(&mut h, "ui.dialog.set", json!({"dialog":id,"field":"color","value":"#0000ff"}));
        h.run_steps(4);
        assert_eq!(pixel(&mut h), original);
        call(&mut h, "ui.dialog.set", json!({"dialog":id,"field":"color","value":"#ff0000"}));
        h.run_steps(4);
        call(&mut h, "ui.dialog.confirm", json!({"dialog":id}));
        h.run_steps(4);
        assert_eq!(pixel(&mut h), preview);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        h.run_steps(4);
        assert_eq!(pixel(&mut h), original);
    }
}
