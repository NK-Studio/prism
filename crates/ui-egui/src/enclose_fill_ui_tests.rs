use super::*;
use crate::canvas::{ToolEvent, tool_event};
use prism_engine::paint::BrushPreset;
use prism_engine::paint::sut::SutBrushMetadata;
use prism_geom::Rect;

/// A white canvas with a black square outline (20..=60) on the Background, an empty layer
/// above it, the Clip Studio workspace, and an imported enclose-and-fill sub-tool.
fn app(params: &[(&str, f64)]) -> PrismApp {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 80, "background": "white"})).unwrap();
    app.sync_views();
    let bottom = app.session.active().unwrap().active_layer.unwrap();
    app.session
        .edit("lines", |doc, _| {
            let s = doc.layer_mut(bottom).unwrap().surface_mut().unwrap();
            for r in [Rect::new(20, 20, 61, 21), Rect::new(20, 60, 61, 61), Rect::new(20, 20, 21, 61), Rect::new(60, 20, 61, 61)] {
                s.fill_rect(r, &[0.0, 0.0, 0.0, 1.0]);
            }
            Ok(())
        })
        .unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    crate::workspace_ui::set_style(&mut app, prism_engine::prefs::WorkspaceStyle::ClipStudio);
    let sut = SutBrushMetadata {
        input_op: 31,
        output_op: 51,
        node_icon: 10282,
        parameters: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        ..Default::default()
    };
    app.session.tools.presets.push(BrushPreset { name: "隙間無く囲って塗るツール".into(), brush: Default::default(), builtin: false, group: "G".into(), sut: Some(sut) });
    app
}

fn pick(app: &mut PrismApp) {
    let sub = crate::sub_tool_panel::sub_tools(app, CspTool::Fill).into_iter().find(|s| s.name == "隙間無く囲って塗るツール").unwrap();
    assert!(sub.usable, "the enclose-and-fill sub-tool is selectable");
    crate::sub_tool_panel::apply(app, CspTool::Fill, &sub);
}

fn drag(app: &mut PrismApp, pts: &[[f64; 2]]) {
    let mods = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: pts[0][0], y: pts[0][1], pressure: 1.0 }, mods);
    for p in &pts[1..] {
        tool_event(app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, mods);
    }
    let last = pts[pts.len() - 1];
    tool_event(app, ToolEvent::Up { x: last[0], y: last[1] }, mods);
}

fn px(app: &PrismApp, x: i32, y: i32) -> Vec<f32> {
    let d = app.session.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
}

const AROUND: [[f64; 2]; 5] = [[10.0, 10.0], [70.0, 10.0], [70.0, 70.0], [10.0, 70.0], [10.0, 12.0]];

#[test]
fn selecting_the_sub_tool_runs_a_lasso_that_fills_enclosed_areas() {
    let mut app = app(&[("FillReferMultiLayer", 1.0), ("FillColorMargin", 20.0), ("FillAntiAlias", 0.0)]);
    pick(&mut app);
    assert_eq!((app.ui.tool, app.ui.csp_tool), (Tool::Lasso, CspTool::Fill));
    assert!(active(&app));
    assert_eq!((app.ui.tool_options.sample_all_layers, app.ui.tool_options.tolerance, app.ui.tool_options.anti_alias), (true, 51.0, false));
    drag(&mut app, &AROUND);
    assert!(app.drag.is_none());
    assert_eq!(px(&app, 40, 40), vec![0.0, 0.0, 0.0, 1.0], "the enclosed area takes the foreground colour");
    assert_eq!(px(&app, 15, 15), vec![0.0; 4], "the area touching the lasso stays empty");
    assert_eq!(px(&app, 20, 40), vec![0.0, 0.0, 0.0, 1.0], "the fill runs under the line, like CLIP STUDIO PAINT");
    assert!(app.session.active().unwrap().doc.selection.is_none(), "no selection is made");
    // A lasso through the square fills only the lines inside it, not the open inside.
    app.run("edit.undo", json!({})).unwrap();
    drag(&mut app, &[[10.0, 10.0], [40.0, 10.0], [40.0, 70.0], [10.0, 70.0]]);
    assert_eq!(px(&app, 30, 40), vec![0.0; 4]);
    assert_eq!(px(&app, 20, 40)[3], 1.0);
    // A lasso around open space fills nothing and says so.
    app.run("edit.undo", json!({})).unwrap();
    drag(&mut app, &[[70.0, 10.0], [95.0, 10.0], [95.0, 70.0], [70.0, 70.0]]);
    assert_eq!(px(&app, 80, 40), vec![0.0; 4]);
    assert_eq!(app.ui.status, "No closed area inside the lasso to fill");
}

#[test]
fn a_press_inside_a_selection_still_draws_the_outline() {
    let mut app = app(&[("FillReferMultiLayer", 1.0)]);
    app.run("select.all", json!({})).unwrap();
    pick(&mut app);
    assert!(active(&app));
    drag(&mut app, &AROUND);
    assert_eq!(px(&app, 40, 40)[3], 1.0, "filled, not a selection move");
}

#[test]
fn close_gap_and_area_scaling_come_from_the_sub_tool() {
    let mut app = app(&[("FillUseCloseGap", 1.0), ("FillCloseGap", 5.0), ("FillUseExpand", 1.0), ("FillExpandLength", -1.0), ("FillExpandType", 2.0)]);
    pick(&mut app);
    let preset = preset(&app).unwrap();
    assert_eq!(sut_params(&app, &preset), json!({"closeGap": 5.0, "expand": -1.0}));
    // Disabled settings and absurd values.
    app.session.tools.presets.iter_mut().find(|p| p.name == preset).and_then(|p| p.sut.as_mut()).unwrap().parameters.extend([("FillUseCloseGap".into(), 0.0), ("FillExpandLength".into(), 1.0e6), ("FillExpandType".into(), 0.0)]);
    assert_eq!(sut_params(&app, &preset), json!({"closeGap": 0.0, "expand": 256.0}));
}

#[test]
fn the_plain_lasso_and_other_workspaces_still_select() {
    let mut app = app(&[]);
    pick(&mut app);
    crate::workspace_ui::set_style(&mut app, prism_engine::prefs::WorkspaceStyle::default());
    assert!(!active(&app), "only in the Clip Studio workspace");
    drag(&mut app, &AROUND);
    assert!(app.session.active().unwrap().doc.selection.is_some());
    assert_eq!(px(&app, 40, 40), vec![0.0; 4]);
}
