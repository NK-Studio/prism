use super::*;
use egui_kittest::{Harness, kittest::Queryable};
use prism_engine::paint::BrushPreset;

/// A folder that is not a CLIP STUDIO data folder: tests never read the machine's real one.
const NOWHERE: &str = "/nonexistent/prism-csp-root";

fn app() -> PrismApp {
    let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    app.ui.csp_library.root = Some(NOWHERE.into());
    app
}

fn listing() -> Value {
    json!({
        "root": "x",
        "count": 5,
        "ready": 3,
        "notices": [prism_io::sut::library::LICENSE_NOTICE, "CLIP STUDIO appears to be running or has unsaved database changes; the newest downloads may be missing until it is closed"],
        "catalogs": [{"name": "Pack", "thumbnail": null, "tags": ["packed"]}],
        "brushes": [
            {"uuid": "a", "name": "Ink pen", "kind": "brush", "catalog": "Pack", "status": "ready", "source": "downloadCache", "guidance": null, "thumbnail": null, "tags": ["ink", "lace"]},
            {"uuid": "b", "name": "Owned", "kind": "brush", "catalog": "Pack", "status": "ready", "source": "toolLibrary", "guidance": null, "thumbnail": null},
            {"uuid": "c", "name": "Cold", "kind": "brush", "catalog": "Pack", "status": "noCache", "source": null, "guidance": "CLIP STUDIO PAINT has not prepared this material yet", "thumbnail": null},
            {"uuid": "d", "name": "Bucket", "kind": "other_tool", "catalog": "", "status": "ready", "source": "materialFile", "guidance": null, "thumbnail": null, "tags": ["fill", "packed"]},
            {"uuid": "e", "name": "Later", "kind": "brush", "catalog": "", "status": "downloading", "source": null, "guidance": "wait", "thumbnail": null}
        ]
    })
}

fn harness(app: PrismApp) -> Harness<'static, PrismApp> {
    let mut h = Harness::builder().with_size(egui::vec2(380.0, 560.0)).build_ui_state(
        |ui, app: &mut PrismApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            body(app, ui);
        },
        app,
    );
    PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(3);
    h
}

#[test]
fn the_extensions_menu_shows_the_panel_in_its_own_window_and_toggles_it() {
    let mut app = app();
    let ctx = egui::Context::default();
    // Window › Extensions comes right after the Workspace submenu, as in Photoshop.
    let items = crate::menus::menu_items(&app);
    let at = items.iter().position(|i| i.id == "window.extensions.clipStudio").expect("menu item");
    assert_eq!(items[at].path, ["Window", "Extensions"]);
    assert_eq!(items[at - 1].id, "window.workspace.lockWorkspace");
    crate::menus::invoke(&mut app, &ctx, "window.extensions.clipStudio", Value::Null).unwrap();
    let pro = crate::prefs_ui::layout(&app) == crate::theme::Layout::Compact;
    assert!(crate::dock_model::panel_showing(&app.ui, PanelId::ClipStudio, pro));
    // A floating window of its own, sized for the list (it docks and tabs like any panel).
    let float = app.ui.dock.floats.iter().find(|f| f.groups.iter().any(|g| app.ui.dock.tabs_of(*g, pro).contains(&PanelId::ClipStudio))).expect("float");
    assert_eq!(float.size, PanelId::ClipStudio.float_size());
    let item = |app: &PrismApp| crate::menus::menu_items(app).into_iter().find(|i| i.id == "window.extensions.clipStudio").and_then(|i| i.checked);
    assert_eq!(item(&app), Some(true));
    crate::menus::invoke(&mut app, &ctx, "window.extensions.clipStudio", Value::Null).unwrap();
    assert!(!crate::dock_model::panel_showing(&app.ui, PanelId::ClipStudio, pro));
    // The Clip Studio workspace's own panels are reachable from the Window menu too.
    crate::menus::invoke(&mut app, &ctx, "window.panel.subTool", Value::Null).unwrap();
    assert!(crate::dock_model::panel_showing(&app.ui, PanelId::SubTool, pro));
}

#[test]
fn a_missing_clip_studio_folder_shows_guidance_instead_of_a_list() {
    let mut app = app();
    app.ui.csp_library.selected = vec!["stale".into()];
    refresh(&mut app);
    assert!(app.ui.csp_library.error.is_some());
    assert!(app.ui.csp_library.selected.is_empty());
    let h = harness(app);
    assert!(h.query_by_label("Choose CLIP STUDIO data folder…").is_some());
    assert!(h.query_by_label("Open Files…").is_some(), "exported files can still be imported");
}

#[test]
fn materials_are_listed_by_collection_and_checked_by_row_or_collection() {
    let mut app = app();
    app.ui.csp_library.listing = Some(listing());
    app.session.tools.presets.push(BrushPreset { name: "Owned".into(), brush: Default::default(), builtin: false, group: "Pack".into(), sut: None });
    let mut h = harness(app);
    // Only ready materials are checkable rows.
    assert!(h.query_by_label("Ink pen").is_some());
    assert!(h.query_by_label("Cold").is_none());
    h.get_by_label("Ink pen").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.csp_library.selected, ["a"]);
    assert!(h.query_by_label("Import 1").is_some());
    // The collection's Select all checks its ready materials (not the cold one).
    h.get_all_by_label("Select all").next().unwrap().click();
    h.run_steps(2);
    let mut picked = h.state().ui.csp_library.selected.clone();
    picked.sort();
    assert_eq!(picked, ["a", "b"]);
    h.get_all_by_label("Clear selection").next().unwrap().click();
    h.run_steps(2);
    assert!(h.state().ui.csp_library.selected.is_empty());
    // Kind filter and search.
    h.state_mut().ui.csp_library.filter = KindFilter::SubTools;
    h.run_steps(2);
    assert!(h.query_by_label("Ink pen").is_none() && h.query_by_label("Bucket").is_some());
    h.state_mut().ui.csp_library.filter = KindFilter::All;
    h.state_mut().ui.csp_library.search = "zzz".into();
    h.run_steps(2);
    assert!(h.query_by_label("No materials match the filter").is_some());
    // Folding a collection hides its rows.
    h.state_mut().ui.csp_library.search.clear();
    h.state_mut().ui.csp_library.collapsed.push("Pack".into());
    h.run_steps(2);
    assert!(h.query_by_label("Ink pen").is_none() && h.query_by_label("Bucket").is_some());
}

#[test]
fn tags_come_from_the_material_and_its_collection_and_are_counted_by_use() {
    let l = listing();
    let by = |uuid: &str| materials(&l).find(|b| b["uuid"] == uuid).unwrap().clone();
    assert_eq!(tags_of(&l, &by("a")), ["ink", "lace", "packed"]);
    assert_eq!(tags_of(&l, &by("b")), ["packed"], "untagged materials carry their collection's tags");
    assert_eq!(tags_of(&l, &by("d")), ["fill", "packed"], "loose materials have only their own");
    assert!(tags_of(&l, &by("e")).is_empty());
    assert!(has_tags(&l));
    let mut state = CspLibraryState::default();
    let counts = tag_counts(&state, &l);
    assert_eq!(counts.first(), Some(&("packed".to_string(), 4)));
    assert_eq!(counts.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["packed", "fill", "ink", "lace"]);
    // Chosen tags combine (all must match) and stay listed even when nothing carries them.
    state.tags = vec!["packed".into(), "fill".into()];
    assert_eq!(materials(&l).filter(|b| shown(&state, &l, b)).count(), 1);
    state.tags = vec!["gone".into()];
    assert_eq!(tag_counts(&state, &l), [("gone".to_string(), 0)]);
    toggle_tag(&mut state, "gone");
    toggle_tag(&mut state, "ink");
    assert_eq!(state.tags, ["ink"]);
    // Malformed tag data is ignored, never a panic.
    let odd = json!({"catalogs": [{"name": "", "tags": "x"}], "brushes": [{"uuid": "z", "name": "Z", "catalog": "", "tags": [1, null, "ok"]}]});
    assert_eq!(tags_of(&odd, &odd["brushes"][0]), ["ok"]);
}

#[test]
fn tags_filter_the_list_from_the_tag_bar_and_search() {
    let mut app = app();
    app.ui.csp_library.listing = Some(listing());
    let mut h = harness(app);
    // Folded: only the toggle shows; open, every tag is a chip.
    assert!(h.query_by_label("fill").is_none());
    h.get_by_label("Tags").click();
    h.run_steps(2);
    assert!(h.state().ui.csp_library.tags_open);
    h.get_by_label("fill").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.csp_library.tags, ["fill"]);
    assert!(h.query_by_label("Bucket").is_some() && h.query_by_label("Ink pen").is_none());
    // A collection's tag reaches its untagged materials.
    h.state_mut().ui.csp_library.tags = vec!["packed".into()];
    h.run_steps(2);
    assert!(h.query_by_label("Owned").is_some() && h.query_by_label("Bucket").is_some());
    h.get_by_label("Clear tags").click();
    h.run_steps(2);
    assert!(h.state().ui.csp_library.tags.is_empty());
    // The search box matches tags too.
    h.state_mut().ui.csp_library.search = "LACE".into();
    h.run_steps(2);
    assert!(h.query_by_label("Ink pen").is_some() && h.query_by_label("Owned").is_none());
}

#[test]
fn the_grid_view_shows_two_columns_of_checkable_cards() {
    let mut app = app();
    app.ui.csp_library.listing = Some(listing());
    app.ui.csp_library.view = View::Grid;
    let mut h = harness(app);
    let ink = h.get_by_label("Ink pen").rect();
    let owned = h.get_by_label("Owned").rect();
    assert!((ink.top() - owned.top()).abs() < 0.5 && owned.left() > ink.right(), "side by side: {ink:?} {owned:?}");
    assert!(ink.height() > ROW_H, "cards are taller than rows");
    h.get_by_label("Owned").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.csp_library.selected, ["b"]);
    // Back to one column: rows stack.
    h.state_mut().ui.csp_library.view = View::List;
    h.run_steps(2);
    let (ink, owned) = (h.get_by_label("Ink pen").rect(), h.get_by_label("Owned").rect());
    assert!(owned.top() >= ink.bottom() && (ink.height() - ROW_H).abs() < 0.5);
    // The view is part of the saved state.
    let v = serde_json::to_value(&h.state().ui.csp_library).unwrap();
    assert_eq!(v["view"], "list");
}

#[test]
fn library_membership_follows_the_import_group() {
    let mut app = app();
    app.session.tools.presets.push(BrushPreset { name: "Owned".into(), brush: Default::default(), builtin: false, group: "Pack".into(), sut: None });
    app.session.tools.presets.push(BrushPreset { name: "Loose".into(), brush: Default::default(), builtin: false, group: "CLIP STUDIO".into(), sut: None });
    assert!(in_library(&app, &json!({"name": "Owned", "catalog": "Pack"})));
    assert!(!in_library(&app, &json!({"name": "Owned", "catalog": "Other"})));
    assert!(in_library(&app, &json!({"name": "Loose", "catalog": ""})), "loose materials import into CLIP STUDIO");
    assert!(!in_library(&app, &json!({"catalog": "Pack"})));
}

#[test]
fn previews_decode_once_and_bad_files_are_skipped() {
    use prism_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
    let dir = std::env::temp_dir().join(format!("prism-csp-thumbs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let im = Image::from_raw(2, 1, ChannelLayout::Rgba, SampleType::U8, vec![255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
    let good = dir.join("thumbnail.png");
    std::fs::write(&good, prism_codecs::encode(&im, Format::Png, &EncodeOptions::default()).unwrap()).unwrap();
    let bad = dir.join("broken.png");
    std::fs::write(&bad, b"not a png").unwrap();
    let ctx = egui::Context::default();
    assert_eq!(decode(&ctx, &good.display().to_string()).map(|t| t.size()), Some([2, 1]));
    assert!(decode(&ctx, &bad.display().to_string()).is_none());
    assert!(decode(&ctx, &dir.join("missing.png").display().to_string()).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_results_report_failures_and_clear_the_selection() {
    let mut app = app();
    app.ui.csp_library.selected = vec!["a".into()];
    imported(&mut app, &json!({"count":1,"brushes":1,"warnings":["x: thumbnail"],"failed":[{"uuid":"b","error":"not prepared"}]}));
    assert!(app.ui.csp_library.selected.is_empty());
    assert!(app.ui.status_error);
    assert_eq!(app.ui.brush_tab, 1);
}

#[test]
fn panel_state_is_driven_through_the_control_channel() {
    let mut app = app();
    let ctx = egui::Context::default();
    let pro = crate::prefs_ui::layout(&app) == crate::theme::Layout::Compact;
    let call = |app: &mut PrismApp, params: Value| {
        let (req, _rx) = crate::control::ControlRequest::new("ui.set", params);
        assert!(matches!(crate::control::handle(app, &ctx, &req), crate::control::Outcome::Done(_)));
    };
    call(&mut app, json!({"cspLibraryOpen": true}));
    assert!(crate::dock_model::panel_showing(&app.ui, PanelId::ClipStudio, pro));
    assert!(app.ui.csp_library.error.is_some(), "listed again on open");
    call(&mut app, json!({"cspLibraryOpen": false}));
    assert!(!crate::dock_model::panel_showing(&app.ui, PanelId::ClipStudio, pro));
    call(&mut app, json!({"cspLibraryView": "grid", "cspLibraryTags": ["lace", "레이스"]}));
    assert_eq!(app.ui.csp_library.view, View::Grid);
    assert_eq!(app.ui.csp_library.tags, ["lace", "레이스"]);
    // Bad values change nothing.
    for bad in [json!({"cspLibraryView": "cards"}), json!({"cspLibraryTags": "lace"}), json!({"cspLibraryTags": [""]}), json!({"cspLibraryTags": [3]})] {
        let (req, _rx) = crate::control::ControlRequest::new("ui.set", bad);
        let _ = crate::control::handle(&mut app, &ctx, &req);
    }
    assert_eq!(app.ui.csp_library.view, View::Grid);
    assert_eq!(app.ui.csp_library.tags, ["lace", "레이스"]);
}
