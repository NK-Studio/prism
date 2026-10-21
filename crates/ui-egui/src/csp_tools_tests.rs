use super::*;
use egui_kittest::{Harness, kittest::Queryable};
use prism_engine::paint::BrushPreset;
use prism_engine::paint::sut::SutBrushMetadata;

fn sut(name: &str, ops: (i64, i64), icon: i64) -> BrushPreset {
    BrushPreset {
        name: name.into(),
        brush: Default::default(),
        builtin: false,
        group: "SUT".into(),
        sut: Some(SutBrushMetadata { input_op: ops.0, output_op: ops.1, node_icon: icon, ..Default::default() }),
    }
}

fn csp_app() -> PrismApp {
    let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    for p in [
        sut("G-pen", (11, 11), 101),
        sut("Gouache", (10, 10), 110),
        sut("Rose", (10, 10), 128),
        sut("Spray", (10, 10), 105),
        sut("Kneaded", (12, 12), 106),
        sut("Bucket", (50, 50), 400),
        sut("Enclose", (31, 51), 406),
    ] {
        app.session.tools.presets.push(p);
    }
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "window.workspace.clipStudio", serde_json::Value::Null).unwrap();
    app
}

#[test]
fn imported_sub_tools_are_placed_by_icon_then_by_operation() {
    let place = |ops, icon| placement(sut("x", ops, icon).sut.as_ref().unwrap());
    // CSP's own icons tell apart the sub-tools that share output 10.
    assert_eq!(
        [place((10, 10), 110), place((10, 10), 105), place((10, 10), 128), place((10, 10), 109)],
        [Some(CspTool::Brush), Some(CspTool::Airbrush), Some(CspTool::Decoration), Some(CspTool::Blend)]
    );
    assert_eq!(
        [place((11, 11), 101), place((11, 11), 102), place((31, 31), 216), place((30, 30), 213)],
        [Some(CspTool::Pen), Some(CspTool::Pencil), Some(CspTool::Fill), Some(CspTool::Figure)]
    );
    // Custom icons fall back to the operation family; tools Prism has no counterpart for stay out.
    assert_eq!(
        [place((11, 11), 10_000), place((14, 14), 0), place((50, 50), 10_007), place((30, 61), 610), place((0, 0), 0)],
        [Some(CspTool::Pen), Some(CspTool::Pen), Some(CspTool::Fill), None, Some(CspTool::Brush)]
    );
}

#[test]
fn picking_a_csp_tool_applies_its_sub_tool_and_keys_cycle() {
    let mut app = csp_app();
    select(&mut app, CspTool::Decoration);
    assert_eq!((app.ui.csp_tool, app.ui.tool, app.session.tools.brush_preset.as_deref()), (CspTool::Decoration, Tool::Brush, Some("Rose")));
    assert_eq!(app.session.journal.last().unwrap().0, "tools.setBrush");
    // Built-in sub-tools come first: the eraser starts on a plain hard round tip.
    select(&mut app, CspTool::Eraser);
    assert_eq!((app.ui.tool, app.session.tools.brush.hardness, app.session.tools.brush.tip.clone()), (Tool::Eraser, 1.0, prism_engine::paint::TipShape::Round));
    let kneaded = crate::sub_tool_panel::sub_tools(&app, CspTool::Eraser).into_iter().find(|s| s.name == "Kneaded").unwrap();
    crate::sub_tool_panel::apply(&mut app, CspTool::Eraser, &kneaded);
    assert_eq!((app.ui.tool, app.session.tools.brush_preset.as_deref()), (Tool::Eraser, Some("Kneaded")));
    // Each tool remembers its sub-tool.
    let fill = crate::sub_tool_panel::sub_tools(&app, CspTool::Fill);
    let others = fill.iter().find(|s| s.name == "Refer other layers").unwrap().clone();
    crate::sub_tool_panel::apply(&mut app, CspTool::Fill, &others);
    assert!(app.ui.tool_options.sample_all_layers);
    select(&mut app, CspTool::Pen);
    select(&mut app, CspTool::Fill);
    assert_eq!((app.ui.tool, app.ui.csp_sub_tools.get(&CspTool::Fill).map(String::as_str)), (Tool::PaintBucket, Some("builtin:Refer other layers")));
    assert!(set_from_name(&mut app, "airbrush").is_ok() && app.session.tools.brush_preset.as_deref() == Some("Spray"));
    assert!(set_from_name(&mut app, "lasso tool").is_err());
}

#[test]
fn clip_studio_tools_window_and_panels_replace_photoshop_tools() {
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut PrismApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::toolbar(app, ui);
            crate::sub_tool_panel::body(app, ui);
            crate::sub_tool_panel::property_body(app, ui);
        },
        csp_app(),
    );
    PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(4);
    // The workspace docks Sub Tool, Tool Property and Color Slider in a left column.
    let column = h.state().ui.dock.columns.first().cloned().expect("a column");
    let tabs: Vec<_> = column.groups.iter().flat_map(|g| h.state().ui.dock.tabs_of(*g, true)).collect();
    assert_eq!(tabs, [crate::dock_model::PanelId::SubTool, crate::dock_model::PanelId::ToolProperty, crate::dock_model::PanelId::ColorSlider]);
    // CSP's tools, not Photoshop's.
    h.get_by_label("Decoration").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.csp_tool, CspTool::Decoration);
    assert!(h.query_by_label("Spot Healing Brush Tool").is_none());
    // The Sub Tool panel lists the tool's sub-tools; an unsupported imported one is listed, greyed.
    // (The Tool Property header repeats the current sub-tool's name; the list comes first.)
    assert!(h.get_all_by_label("Rose").next().is_some());
    crate::csp_tools::select(h.state_mut(), CspTool::Fill);
    h.run_steps(3);
    assert!(h.get_all_by_label("Bucket").next().is_some());
    h.get_all_by_label("Refer only to editing layer").next().unwrap().click();
    h.run_steps(3);
    assert!(!h.state().ui.tool_options.sample_all_layers);
    // An imported fill runs on the bucket; enclose-and-fill runs on the lasso (enclose_fill_ui).
    h.get_all_by_label("Enclose").next().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().ui.csp_sub_tools.get(&CspTool::Fill).map(String::as_str), Some("sut:Enclose"));
    assert_eq!(h.state().ui.tool, Tool::Lasso);
    h.get_all_by_label("Bucket").next().unwrap().click();
    h.run_steps(3);
    assert_eq!((h.state().ui.tool, h.state().ui.csp_sub_tools.get(&CspTool::Fill).map(String::as_str)), (Tool::PaintBucket, Some("sut:Bucket")));

    // A Photoshop workspace brings Photoshop's tools back.
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.essentials", serde_json::Value::Null).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Decoration").is_none());
}
