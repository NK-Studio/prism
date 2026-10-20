use super::*;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use prism_engine::paint::BrushPreset;
use prism_engine::paint::sut::{SutBrushMetadata, SutDisplay};

const NAME: &str = "隙間無く囲って塗るツール";

/// An app in the Clip Studio workspace with an imported fill-family sub-tool picked.
fn app(input_op: i64, output_op: i64, visible: &[u32], params: &[(&str, f64)]) -> PrismApp {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    crate::workspace_ui::set_style(&mut app, prism_engine::prefs::WorkspaceStyle::ClipStudio);
    let sut = SutBrushMetadata {
        original: SutDisplay { visible: visible.to_vec(), show_categories: false },
        input_op,
        output_op,
        parameters: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        ..Default::default()
    };
    app.session.tools.presets.push(BrushPreset { name: NAME.into(), brush: Default::default(), builtin: false, group: "G".into(), sut: Some(sut) });
    let tool = if output_op == 20 { crate::csp_tools::CspTool::AutoSelect } else { crate::csp_tools::CspTool::Fill };
    let sub = crate::sub_tool_panel::sub_tools(&app, tool).into_iter().find(|s| s.name == NAME).unwrap();
    crate::sub_tool_panel::apply(&mut app, tool, &sub);
    app
}

fn harness(app: PrismApp) -> Harness<'static, PrismApp> {
    let mut h = Harness::builder().with_size(egui::vec2(320.0, 600.0)).build_ui_state(|ui, app: &mut PrismApp| crate::sub_tool_panel::property_body(app, ui), app);
    h.run_steps(3);
    h
}

fn param(h: &Harness<'_, PrismApp>, key: &str) -> Option<f64> {
    h.state().session.tools.presets.iter().find(|p| p.name == NAME)?.sut.as_ref()?.parameters.get(key).copied()
}

#[test]
fn close_gap_levels_follow_the_clip_studio_indicator() {
    for (px, level) in [(0.1, 1), (1.0, 1), (2.0, 1), (3.0, 2), (5.0, 3), (9.9, 3), (10.0, 4), (20.0, 5), (50.0, 5), (f64::NAN, 1), (-3.0, 1)] {
        assert_eq!(close_gap_level(px), level, "{px}");
    }
}

#[test]
fn enclose_and_fill_shows_and_edits_its_clip_studio_settings() {
    // The supplied tool: Close gap 5 (level 3), Area scaling −1, Refer multiple, Exclude draft
    // layers (a child of Refer multiple) and Stabilization.
    let params = [
        ("FillUseCloseGap", 1.0),
        ("FillCloseGap", 5.0),
        ("FillUseExpand", 1.0),
        ("FillExpandLength", -1.0),
        ("FillReferMultiLayer", 1.0),
        ("FillReferLayerType", 1.0),
        ("FillNotReferDraft", 1.0),
        ("FlickerReduction", 5.0),
    ];
    let mut h = harness(app(31, 51, &[5101, 5111, 5001, 5012, 40], &params));
    for label in ["Close gap", "Area scaling", "Refer multiple", "Exclude draft layers", "Stabilization", "Reference layer"] {
        assert!(h.query_by_label(label).is_some(), "{label} is shown");
    }
    assert!(h.query_by_label("Tolerance").is_none(), "only what CLIP STUDIO shows");
    // Level 3 is lit; picking level 4 stores 10 px and the fill uses it.
    let lit: Vec<bool> = [1, 3, 5, 10, 20].iter().map(|px| h.get_by_label(&format!("Close gap {px} px")).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)).collect();
    assert_eq!(lit, [true, true, true, false, false]);
    h.get_by_label("Close gap 10 px").click();
    h.run_steps(2);
    assert_eq!(param(&h, "FillCloseGap"), Some(10.0));
    assert_eq!(crate::enclose_fill_ui::sut_params(h.state(), NAME)["closeGap"], 10.0);
    // The layer kind and the Close gap switch are stored too.
    h.get_by_label("All layers").click();
    h.run_steps(2);
    assert_eq!(param(&h, "FillReferLayerType"), Some(0.0));
    h.get_by_label("Close gap").click();
    h.run_steps(2);
    assert_eq!(param(&h, "FillUseCloseGap"), Some(0.0));
    assert_eq!(crate::enclose_fill_ui::sut_params(h.state(), NAME)["closeGap"], 0.0);
}

#[test]
fn bucket_fill_marks_what_prism_does_not_run_and_syncs_tool_options() {
    let params = [("FillColorMargin", 20.0), ("FillContinuousPixel", 1.0), ("FillUseCloseGap", 1.0), ("FillCloseGap", 2.0), ("Opacity", 50.0)];
    let mut h = harness(app(50, 50, &[5200, 5101, 5120, 10, 4242], &params));
    assert_eq!(h.state().ui.tool_options.fill_opacity, 50.0);
    assert!(h.query_by_label("Kept for CLIP STUDIO; Prism does not use this setting yet").is_some(), "the bucket does not close gaps");
    assert!(h.query_by_label("CLIP STUDIO setting 4242").is_some(), "unknown settings stay listed");
    h.get_by_label("Apply to connected pixels only").click();
    h.run_steps(2);
    assert_eq!(param(&h, "FillContinuousPixel"), Some(0.0));
    assert!(!h.state().ui.tool_options.contiguous, "the tool option follows");
}
