use super::*;
use prism_engine::paint::BrushPreset;
use prism_engine::paint::sut::SutBrushMetadata;

fn preset(name: &str, sut: Option<(i64, i64, i64)>) -> BrushPreset {
    BrushPreset {
        name: name.into(),
        brush: Default::default(),
        builtin: false,
        group: "G".into(),
        sut: sut.map(|(i, o, icon)| SutBrushMetadata { input_op: i, output_op: o, node_icon: icon, ..Default::default() }),
    }
}

/// An imported copy-stamp sub-tool runs on the Clone Stamp with its brush, and its "Adjust
/// position" becomes the tool's Aligned option.
#[test]
fn copy_stamp_sub_tools_run_the_clone_stamp() {
    let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    let mut stamp = preset("Stamp", Some((13, 13, 109)));
    stamp.brush.size = 33.0;
    app.session.tools.presets = vec![stamp, preset("Blender", Some((12, 12, 109)))];
    let pick = |app: &mut PrismApp, name: &str| {
        let sub = sub_tools(app, CspTool::Blend).into_iter().find(|s| s.name == name).unwrap();
        apply(app, CspTool::Blend, &sub);
    };
    assert_eq!(app.session.tools.presets[0].sut.as_ref().unwrap().kind(), prism_engine::paint::sut::SutToolKind::CopyStamp);
    // No stored "Adjust position": the tool's Aligned stays as it was.
    app.ui.tool_options.clone_aligned = false;
    pick(&mut app, "Stamp");
    assert_eq!((app.ui.tool, app.session.tools.brush.size, app.ui.tool_options.clone_aligned), (Tool::CloneStamp, 33.0, false));
    for (stored, aligned) in [(1.0, true), (0.0, false), (f64::NAN, true)] {
        app.session.tools.presets[0].sut.as_mut().unwrap().parameters.insert("StampAdjustPosition".into(), stored);
        pick(&mut app, "Stamp");
        assert_eq!((app.ui.tool, app.ui.tool_options.clone_aligned), (Tool::CloneStamp, aligned), "{stored}");
    }
    // Another sub-tool of the same CSP tool goes back to its own Prism tool.
    if sub_tools(&app, CspTool::Blend).iter().any(|s| s.name == "Blender" && s.usable) {
        pick(&mut app, "Blender");
        assert_ne!(app.ui.tool, Tool::CloneStamp);
    }
}

#[test]
fn sub_tools_list_built_ins_then_imported_and_mark_unsupported_ones() {
    let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    app.session.tools.presets =
        vec![preset("Mine", None), preset("Pen A", Some((11, 11, 101))), preset("Fill A", Some((50, 50, 400))), preset("Text A", Some((70, 70, 500)))];
    let names = |app: &PrismApp, tool| sub_tools(app, tool).into_iter().map(|s| (s.name, s.usable)).collect::<Vec<_>>();
    assert_eq!(names(&app, CspTool::Pen), [("Pen A".to_string(), true)]);
    // Prism's own brush presets live under Brush.
    assert_eq!(names(&app, CspTool::Brush), [("Mine".to_string(), true)]);
    // Imported fill and text sub-tools run on Prism's bucket and type tools; comic ones can't.
    assert_eq!(
        names(&app, CspTool::Fill),
        [("Refer only to editing layer".to_string(), true), ("Refer other layers".to_string(), true), ("Fill A".to_string(), true)]
    );
    assert_eq!(names(&app, CspTool::Text).last(), Some(&("Text A".to_string(), true)));
    // Enclose-and-fill runs on the Lasso (see `enclose_fill_ui`); lasso fill (53) can't run yet.
    app.session.tools.presets.push(preset("Enclose", Some((31, 51, 406))));
    assert_eq!(names(&app, CspTool::Fill).last(), Some(&("Enclose".to_string(), true)));
    app.session.tools.presets.push(preset("Lasso fill", Some((31, 53, 216))));
    assert_eq!(names(&app, CspTool::Fill).last(), Some(&("Lasso fill".to_string(), false)));
    let lasso_fill = sub_tools(&app, CspTool::Fill).into_iter().find(|s| s.name == "Lasso fill").unwrap();
    let before = app.ui.tool;
    apply(&mut app, CspTool::Fill, &lasso_fill);
    assert_eq!(app.ui.tool, before, "an unsupported sub-tool changes nothing");
    let enclose = sub_tools(&app, CspTool::Fill).into_iter().find(|s| s.name == "Enclose").unwrap();
    apply(&mut app, CspTool::Fill, &enclose);
    assert_eq!(app.ui.tool, Tool::Lasso);
    // The imported fill's reference, colour margin and adjacency become the bucket's options.
    if let Some(m) = app.session.tools.presets.iter_mut().find(|p| p.name == "Fill A").and_then(|p| p.sut.as_mut()) {
        m.parameters.extend([("FillReferMultiLayer".into(), 1.0), ("FillColorMargin".into(), 20.0), ("FillContinuousPixel".into(), 0.0)]);
    }
    let fill = sub_tools(&app, CspTool::Fill).into_iter().find(|s| s.name == "Fill A").unwrap();
    apply(&mut app, CspTool::Fill, &fill);
    let o = &app.ui.tool_options;
    assert_eq!((app.ui.tool, o.sample_all_layers, o.tolerance, o.contiguous), (Tool::PaintBucket, true, 51.0, false));
    // Built-ins pick their Prism tool and options.
    for (tool, name, expect) in [
        (CspTool::Selection, "Ellipse", Tool::EllipseMarquee),
        (CspTool::Figure, "Straight line", Tool::Line),
        (CspTool::Text, "Vertical text", Tool::VerticalType),
        (CspTool::Blend, "Finger tip", Tool::Smudge),
    ] {
        let sub = sub_tools(&app, tool).into_iter().find(|s| s.name == name).unwrap();
        apply(&mut app, tool, &sub);
        assert_eq!(app.ui.tool, expect, "{name}");
    }
    let radial = sub_tools(&app, CspTool::Gradient).into_iter().find(|s| s.name == "Radial gradient").unwrap();
    apply(&mut app, CspTool::Gradient, &radial);
    assert_eq!((app.ui.tool, app.ui.tool_options.gradient_style.as_str()), (Tool::Gradient, "radial"));
    assert_eq!(current(&app).map(|s| s.name), Some("Radial gradient".to_string()));
}
