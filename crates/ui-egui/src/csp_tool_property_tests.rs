use super::*;
use prism_engine::paint::sut_catalog as cat;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use prism_engine::paint::BrushPreset;
use prism_engine::paint::csp_gradient::{CspGradient, CspGradientStop, CspStopColor};
use prism_engine::paint::sut::{SutBrushMetadata, SutDisplay};

const NAME: &str = "隙間無く囲って塗るツール";

/// An app in the Clip Studio workspace with an imported sub-tool picked.
fn app(input_op: i64, output_op: i64, visible: &[u32], params: &[(&str, f64)]) -> PrismApp {
    app_icon(input_op, output_op, 0, visible, params)
}

/// [`app`] for a sub-tool with CLIP STUDIO tool icon `node_icon`.
fn app_icon(input_op: i64, output_op: i64, node_icon: i64, visible: &[u32], params: &[(&str, f64)]) -> PrismApp {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    crate::workspace_ui::set_style(&mut app, prism_engine::prefs::WorkspaceStyle::ClipStudio);
    let sut = SutBrushMetadata {
        original: SutDisplay { visible: visible.to_vec(), show_categories: false },
        input_op,
        output_op,
        node_icon,
        parameters: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        ..Default::default()
    };
    let tool = crate::csp_tools::placement(&sut).unwrap();
    app.session.tools.presets.push(BrushPreset { name: NAME.into(), brush: Default::default(), builtin: false, group: "G".into(), sut: Some(sut) });
    let sub = crate::sub_tool_panel::sub_tools(&app, tool).into_iter().find(|s| s.name == NAME).unwrap();
    assert!(sub.usable, "Prism runs this sub-tool");
    crate::sub_tool_panel::apply(&mut app, tool, &sub);
    app
}

fn harness(app: PrismApp) -> Harness<'static, PrismApp> {
    let mut h = Harness::builder().with_size(egui::vec2(320.0, 1400.0)).build_ui_state(|ui, app: &mut PrismApp| crate::sub_tool_panel::property_body(app, ui), app);
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
    assert!(h.query_by_label(DIMMED).is_some(), "the bucket does not close gaps");
    assert!(h.query_by_label("CLIP STUDIO setting 4242").is_some(), "unknown settings stay listed");
    h.get_by_label("Apply to connected pixels only").click();
    h.run_steps(2);
    assert_eq!(param(&h, "FillContinuousPixel"), Some(0.0));
    assert!(!h.state().ui.tool_options.contiguous, "the tool option follows");
}

const NOT_RUN: &str = DIMMED;

/// A main colour to transparent white gradient whose first segment is a step.
fn stripe() -> CspGradient {
    CspGradient {
        stops: vec![
            CspGradientStop { position: 0.0, color: CspStopColor::Foreground, opacity: 1.0, curve: vec![[0.0, 0.0], [1.0, 0.0]] },
            CspGradientStop { position: 0.5, color: CspStopColor::Rgb([1.0, 1.0, 1.0]), opacity: 0.0, curve: vec![] },
        ],
    }
}

/// [`app`] for a gradient sub-tool with its own stops.
fn gradient_app(params: &[(&str, f64)], gradient: Option<CspGradient>) -> PrismApp {
    let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
    crate::workspace_ui::set_style(&mut app, prism_engine::prefs::WorkspaceStyle::ClipStudio);
    let sut = SutBrushMetadata {
        original: SutDisplay { visible: vec![5400, 5402, 5401, 5410, 5420, 3231, 5430, 10, 30], show_categories: false },
        input_op: 52,
        output_op: 52,
        parameters: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        gradient,
        ..Default::default()
    };
    let tool = crate::csp_tools::placement(&sut).unwrap();
    app.session.tools.presets.push(BrushPreset { name: NAME.into(), brush: Default::default(), builtin: false, group: "G".into(), sut: Some(sut) });
    let sub = crate::sub_tool_panel::sub_tools(&app, tool).into_iter().find(|s| s.name == NAME).unwrap();
    crate::sub_tool_panel::apply(&mut app, tool, &sub);
    app
}

fn stored_gradient(h: &Harness<'_, PrismApp>) -> Option<CspGradient> {
    h.state().session.tools.presets.iter().find(|p| p.name == NAME)?.sut.as_ref()?.gradient.clone()
}

#[test]
fn gradient_sub_tools_show_their_items_and_paint_their_own_stops() {
    let params = [("GradationReverseSaved", 1.0), ("GradationShapeKind", 2.0), ("GradationRepeatType", 1.0), ("LineLockAngleStep", 45.0), ("Opacity", 100.0), ("CompositeMode", 0.0)];
    let mut h = harness(gradient_app(&params, Some(stripe())));
    for label in ["Gradient", "Gradient bar", "Reverse gradient", "Stops", "Flip", "Shape", "Edge process", "Snap angle", "Where to create", "Opacity", "Blending mode"] {
        assert!(h.query_by_label(label).is_some(), "{label} is shown");
    }
    assert!(h.query_by_label("CLIP STUDIO setting 5401").is_none(), "the gradient bar's parts have no rows");
    assert!(h.query_by_label("Prism draws an ellipse gradient as a circle").is_some());
    assert!(h.query_by_label(GRADIENT_DATA).is_none() && h.query_by_label(GRADIENT_CURVE).is_none(), "the stops run");
    let o = &h.state().ui.tool_options;
    assert_eq!(o.gradient_style, "radial");
    assert!(!o.gradient_reverse, "GradationReverseSaved is not a flip");
    // The Gradient tool paints the sub-tool's stops: main colour up to the middle, then transparent.
    assert_eq!(h.state().session.presets.gradient.name, NAME);
    let (fg, bg) = (h.state().session.tools.foreground, h.state().session.tools.background);
    let at = |h: &Harness<'_, PrismApp>, t: f32| prism_algo::paint::sample_stops(&h.state().session.presets.gradient.resolve(fg, bg), t);
    assert_eq!(at(&h, 0.4), [fg[0], fg[1], fg[2], 1.0]);
    assert_eq!(at(&h, 0.6)[3], 0.0);
    // Reverse edits the stored stops, and the tool follows.
    h.get_by_label("Reverse gradient").click();
    h.run_steps(2);
    assert_eq!(stored_gradient(&h), Some(stripe().reversed()));
    assert_eq!(at(&h, 0.4)[3], 0.0);
    assert_eq!(at(&h, 0.6), [fg[0], fg[1], fg[2], 1.0]);
    // Flip is kept and editable.
    h.get_by_label("Flip").click();
    h.run_steps(2);
    assert_eq!(param(&h, "GradationReverseSaved"), Some(0.0));
}

#[test]
fn gradient_stops_are_editable_and_live_curves_are_marked() {
    let mut g = stripe();
    g.stops[0].curve = vec![[0.0, 0.0], [0.5, 0.1], [1.0, 1.0]];
    let mut h = harness(gradient_app(&[], Some(g)));
    assert!(h.query_by_label(GRADIENT_CURVE).is_some(), "a curve from the main colour runs straight");
    h.get_by_label("Stops").click();
    h.run_steps(2);
    for label in ["0%", "50%"] {
        assert!(h.query_by_label(label).is_some(), "the {label} stop has a row");
    }
    // Without stops the sub-tool paints Prism's current gradient.
    let h = harness(gradient_app(&[], None));
    assert!(h.query_by_label(GRADIENT_DATA).is_some());
    assert_eq!(stored_gradient(&h), None);
}

#[test]
fn text_sub_tools_pick_their_font() {
    let families = crate::type_tool::families();
    let Some(family) = families.iter().find(|f| f.chars().filter(|c| c.is_alphanumeric()).count() >= 3).cloned() else { return };
    // A unique name (a face of the family, punctuation and case changed) picks the family.
    let unique: String = format!("{}-Bold", family.replace(' ', "").to_uppercase());
    let mut app = app(70, 70, &[7000, 7001, 7014], &[("TextFontSize", 30.0)]);
    let m = app.session.tools.presets.iter_mut().find(|p| p.name == NAME).unwrap().sut.as_mut().unwrap();
    m.strings.insert("TextFontUniqueName".into(), unique.clone());
    m.strings.insert("TextRubyFontName".into(), "Not A Real Font 12345".into());
    app.ui.tool_options.type_font = "nothing".into();
    apply(&mut app, NAME);
    assert_eq!(app.ui.tool_options.type_font, family);
    assert_eq!(match_family(&unique, &families).as_deref(), Some(family.as_str()));
    assert_eq!(match_family("", &families), None);
    let h = harness(app);
    assert!(h.query_by_label("Font").is_some() && h.query_by_label("Reading font").is_some());
    assert!(h.query_by_label(FONT_MISSING).is_none(), "the family is installed");
    // A font Prism does not have keeps the Type tool's font and says so.
    let mut app = h.into_state();
    let m = app.session.tools.presets.iter_mut().find(|p| p.name == NAME).unwrap().sut.as_mut().unwrap();
    m.strings.insert("TextFontName".into(), "Not A Real Font 12345".into());
    apply(&mut app, NAME);
    assert_eq!(app.ui.tool_options.type_font, family);
    let h = harness(app);
    assert!(h.query_by_label(FONT_MISSING).is_some());
}

#[test]
fn text_sub_tools_edit_every_item_and_mark_what_prism_does_not_run() {
    let params = [("TextFontSize", 30.0), ("TextAlignment", 1.0), ("TextWrapping", 0.0), ("TextEdgeEnabled", 0.0), ("TextEdgeWidth", 1.0), ("TextEdgeColorRed", 4_294_967_295.0)];
    let mut h = harness(app(70, 70, &[7052, 7051, 7053, 7000, 7001, 7004, 7032, 7003, 7025, 7038, 7039, 4242], &params));
    for label in ["Apply to", "Drag", "Selection mode", "Font", "Size", "Alignment", "Wrap text at frame", "Text direction", "Text color", "Edge", "Edge color"] {
        assert!(h.query_by_label(label).is_some(), "{label} is shown");
    }
    assert_eq!(h.state().ui.tool_options.type_align, "center", "alignment runs");
    assert_eq!(h.state().ui.tool_options.type_size, 30.0);
    assert!(h.query_by_label(NOT_RUN).is_some(), "items Prism does not run are dimmed");
    h.get_by_label("Wrap text at frame").click();
    h.run_steps(2);
    assert_eq!(param(&h, "TextWrapping"), Some(1.0));
    h.get_by_label("Edge").click();
    h.run_steps(2);
    assert_eq!(param(&h, "TextEdgeEnabled"), Some(1.0));
    // The stored 32-bit colour channels survive the save.
    assert_eq!(param(&h, "TextEdgeColorRed"), Some(4_294_967_295.0));
}

#[test]
fn selection_mode_icons_set_the_marquee_mode() {
    let params = [("SelectCreateType", 0.0), ("FixedShapeAspectType", 2.0), ("FixedShapeWidth", 120.0), ("FixedShapeHeight", 80.0), ("AntiAlias", 2.0)];
    let mut h = harness(app_icon(30, 20, 200, &[3000, 3310, 3311, 3312, 3212, 20], &params));
    assert_eq!(h.state().ui.tool_options.marquee_style, "fixedSize");
    assert_eq!((h.state().ui.tool_options.marquee_width, h.state().ui.tool_options.marquee_height), (120.0, 80.0));
    for label in ["Aspect type", "W", "H", "Adjust angle after fixed", "Anti-aliasing"] {
        assert!(h.query_by_label(label).is_some(), "{label} is shown");
    }
    h.get_by_label("Add to selection").click();
    h.run_steps(2);
    assert_eq!(param(&h, "SelectCreateType"), Some(1.0));
    assert_eq!(h.state().ui.selection_mode, 1);
}

#[test]
fn eyedropper_and_move_layer_settings_are_saved() {
    let mut h = harness(app_icon(41, 41, 300, &[4100, 4130, 4140], &[("SpuitReferType", 0.0), ("SpuitNotReferText", 0.0)]));
    for label in ["Reference", "Exclude text", "Show color picker circle"] {
        assert!(h.query_by_label(label).is_some(), "{label} is shown");
    }
    assert!(h.query_by_label("Prism picks the displayed color").is_some());
    h.get_by_label("Exclude text").click();
    h.run_steps(2);
    assert_eq!(param(&h, "SpuitNotReferText"), Some(1.0));

    let mut h = harness(app_icon(40, 40, 301, &[4001, 4010, 4011, 4020], &[("LayerMoveClicked", 0.0)]));
    assert!(!h.state().ui.tool_options.move_auto_select);
    h.get_by_label("Move the object at the clicked position").click();
    h.run_steps(2);
    assert_eq!(param(&h, "LayerMoveClicked"), Some(1.0));
    assert!(h.state().ui.tool_options.move_auto_select, "Auto-Select follows");
    assert!(h.query_by_label("Moved layers become selected").is_some());
}

#[test]
fn figure_sub_tools_set_the_shape_tool() {
    let params = [("FixedShapeKind", 2.0), ("FixedShapeVertexNumber", 6.0), ("ShapeStrokeAndFillType", 0.0), ("BrushSize", 5.0), ("Opacity", 100.0)];
    let h = harness(app_icon(30, 30, 218, &[3300, 3301, 3200, 3310, 1000, 10, 20, 1900], &params));
    let o = &h.state().ui.tool_options;
    assert_eq!((o.polygon_sides, o.shape_fill, o.stroke_width), (6, true, 0.0), "Create fill: filled, no stroke");
    for label in ["Figure", "Number of corners", "Line/Fill", "Aspect type", "Brush Size", "Brush shape"] {
        assert!(h.query_by_label(label).is_some(), "{label} is shown");
    }
}

#[test]
fn every_catalogue_label_has_clip_studio_s_korean_term() {
    let item_labels = |i: &'static cat::Item| {
        let mut v = vec![i.label];
        for fl in i.fields {
            v.push(fl.label);
            match fl.kind {
                cat::Kind::List(l) => v.extend(l.iter().map(|c| c.1)),
                cat::Kind::Icons(l) => v.extend(l.iter().map(|c| c.1)),
                _ => {}
            }
        }
        v
    };
    crate::i18n::with_language(crate::i18n::Lang::from_pref("ko"), || {
        for i in cat::ITEMS {
            for s in item_labels(i).into_iter().filter(|s| s.chars().any(|c| c.is_ascii_alphabetic())) {
                assert_ne!(tc(i, s), s, "{} {s:?}", i.id);
            }
        }
        // Labels CLIP STUDIO translates per item.
        let item = |id| cat::item(id).unwrap();
        assert_eq!(tc(item(3000), "Selection mode"), "작성 방법");
        assert_eq!(tc(item(7053), "Selection mode"), "선택 추가");
        assert_eq!(tc(item(4001), "Mode"), "이동 대상");
        assert_eq!(tc(item(5112), "Rectangle"), "사각형으로 확장");
        assert_eq!(tc(item(3300), "Rectangle"), "직사각형");
        assert_eq!(tc(item(4200), "Exclude text"), "문자를 선택하지 않음");
        assert_eq!(tc(item(4130), "Exclude text"), "문자를 참조하지 않음");
        for note in [ALL_LAYERS, SEMI_TRANSPARENT, SHRINK_ONLY, ELLIPSE, GRADIENT_DATA, GRADIENT_CURVE, FONT_MISSING, NORMAL_ONLY, DISPLAYED, NOT_RUN] {
            assert_ne!(tl!(note), note);
        }
    });
}

#[test]
#[ignore = "writes Tool Property visual evidence to target/csp-tool-property"]
fn tool_property_visual_evidence() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/csp-tool-property");
    std::fs::create_dir_all(&out).unwrap();
    type Case = (&'static str, fn() -> PrismApp);
    let cases: [Case; 3] = [
        ("text", || app(70, 70, &[7052, 7051, 7053, 7000, 7001, 7004, 7032, 7003, 7025, 7038, 7039], &[("TextFontSize", 30.0), ("TextAlignment", 1.0), ("TextEdgeEnabled", 1.0), ("TextEdgeWidth", 2.0), ("TextEdgeColorRed", 4_294_967_295.0)])),
        ("gradient", || {
            let mut g = stripe();
            g.stops[1].position = 0.4;
            g.stops.push(CspGradientStop { position: 1.0, color: CspStopColor::Rgb([0.2, 0.4, 0.9]), opacity: 1.0, curve: vec![] });
            gradient_app(&[("GradationShapeKind", 1.0), ("LineLockAngle", 1.0), ("LineLockAngleStep", 45.0), ("Opacity", 100.0)], Some(g))
        }),
        ("enclose", || app(31, 51, &[5300, 5101, 5120, 5111, 5112, 5001, 5012, 5020, 3600, 10], &[("FillUseCloseGap", 1.0), ("FillCloseGap", 5.0), ("FillReferMultiLayer", 1.0), ("CloseFillReferColorType", 7.0)])),
    ];
    for (name, make) in cases {
        for lang in ["en", "ko"] {
            let mut h = Harness::builder().with_size(egui::vec2(300.0, 900.0)).with_pixels_per_point(1.5).wgpu().build_ui_state(
                move |ui, app: &mut PrismApp| crate::i18n::with_language(crate::i18n::Lang::from_pref(lang), || crate::sub_tool_panel::property_body(app, ui)),
                make(),
            );
            PrismApp::setup_context(&h.ctx, Default::default());
            h.run_steps(5);
            h.render().unwrap().save(out.join(format!("{name}-{lang}.png"))).unwrap();
        }
    }
}
