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
            {"uuid": "a", "name": "Ink pen", "kind": "brush", "catalog": "Pack", "folders": ["ダウンロード:Pack-1:Brushes"], "status": "ready", "source": "downloadCache", "guidance": null, "thumbnail": null, "tags": ["ink", "lace"]},
            {"uuid": "b", "name": "Owned", "kind": "brush", "catalog": "Pack", "favorite": true, "folders": ["ダウンロード:Pack-1:Brushes"], "status": "ready", "source": "toolLibrary", "guidance": null, "thumbnail": null},
            {"uuid": "c", "name": "Cold", "kind": "brush", "catalog": "Pack", "folders": ["ダウンロード:Pack-1:Brushes"], "status": "noCache", "source": null, "guidance": "CLIP STUDIO PAINT has not prepared this material yet", "thumbnail": null},
            {"uuid": "d", "name": "Bucket", "kind": "other_tool", "catalog": "", "folders": ["自作ブラシ"], "status": "ready", "source": "materialFile", "guidance": null, "thumbnail": null, "tags": ["fill", "packed"]},
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
    // The folder's Select all checks its ready materials (not the cold one).
    h.state_mut().ui.csp_library.folder = Some("ダウンロード:Pack-1".into());
    h.run_steps(2);
    h.get_all_by_label("Select all").next().unwrap().click();
    h.run_steps(2);
    let mut picked = h.state().ui.csp_library.selected.clone();
    picked.sort();
    assert_eq!(picked, ["a", "b"]);
    h.get_all_by_label("Clear selection").next().unwrap().click();
    h.run_steps(2);
    assert!(h.state().ui.csp_library.selected.is_empty());
    // Kind filter and search (across all folders).
    h.state_mut().ui.csp_library.folder = None;
    h.state_mut().ui.csp_library.filter = KindFilter::SubTools;
    h.run_steps(2);
    assert!(h.query_by_label("Ink pen").is_none() && h.query_by_label("Bucket").is_some());
    h.state_mut().ui.csp_library.filter = KindFilter::All;
    h.state_mut().ui.csp_library.search = "zzz".into();
    h.run_steps(2);
    assert!(h.query_by_label("No materials match the filter").is_some());
    // Another folder shows only its own.
    h.state_mut().ui.csp_library.search.clear();
    h.state_mut().ui.csp_library.folder = Some("自作ブラシ".into());
    h.run_steps(2);
    assert!(h.query_by_label("Ink pen").is_none() && h.query_by_label("Bucket").is_some());
}

/// A wide panel: the folder tree beside the materials.
fn wide(app: PrismApp) -> Harness<'static, PrismApp> {
    let mut h = Harness::builder().with_size(egui::vec2(760.0, 560.0)).build_ui_state(
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
fn a_wide_panel_browses_clip_studio_folders_in_a_tree() {
    let mut app = app();
    app.ui.csp_library.listing = Some(with_layers());
    let mut h = wide(app);
    // System roots are translated; a pack loses its content id once opened.
    assert!(h.query_by_label("All materials").is_some());
    h.get_by_label("Download").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.csp_library.folder.as_deref(), Some("ダウンロード"));
    assert!(h.query_by_label("Isometric ruler and grid").is_some());
    h.get_by_label("Isometric ruler and grid").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.csp_library.folder.as_deref(), Some("ダウンロード:Isometric ruler and grid-1758445"));
    assert!(h.query_by_label("Isometric ruler").is_some() && h.query_by_label("Ink pen").is_none());
    // Favourites are their own folder.
    h.get_by_label("Favorite").click();
    h.run_steps(2);
    assert!(h.query_by_label("Owned").is_some() && h.query_by_label("Isometric ruler").is_none());
    h.get_by_label("All materials").click();
    h.run_steps(2);
    assert!(h.state().ui.csp_library.folder.is_none() && h.query_by_label("Ink pen").is_some());
    // Narrow: no tree, the folder menu instead.
    let mut narrow = super::tests::app();
    narrow.ui.csp_library.listing = Some(with_layers());
    let h = harness(narrow);
    assert!(h.query_by_label("Download").is_none(), "the tree is folded into the menu");
}

#[test]
fn dropping_a_material_uses_it_where_it_lands() {
    // A layer material needs a document; nothing happens without one.
    let ctx = egui::Context::default();
    let mut a = app();
    let ruler = MaterialDrag { uuid: "r".into(), name: "Isometric ruler".into(), layer: true };
    use_material(&mut a, &ctx, &ruler, None);
    assert!(a.session.active().is_none());
    // With one, the import runs (the test folder has no CLIP STUDIO data, so it reports why).
    a.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    let layers = a.session.active().unwrap().doc.layers.len();
    use_material(&mut a, &ctx, &ruler, None);
    assert_eq!(a.session.active().unwrap().doc.layers.len(), layers);
    // A brush goes to the brush import with this panel's folder.
    let brush = MaterialDrag { uuid: "a".into(), name: "Ink pen".into(), layer: false };
    let presets = a.session.tools.presets.len();
    use_material(&mut a, &ctx, &brush, None);
    assert_eq!(a.session.tools.presets.len(), presets, "nothing to import from the test folder");
}

#[test]
fn agents_pick_the_folder() {
    let mut app = app();
    let ctx = egui::Context::default();
    let set = |app: &mut PrismApp, v: Value| {
        let (req, _rx) = crate::control::ControlRequest::new("ui.set", v);
        crate::control::handle(app, &ctx, &req)
    };
    set(&mut app, json!({"cspLibraryFolder": "ダウンロード:Pack-1"}));
    assert_eq!(app.ui.csp_library.folder.as_deref(), Some("ダウンロード:Pack-1"));
    set(&mut app, json!({"cspLibraryFolder": "favorite"}));
    assert_eq!(app.ui.csp_library.folder.as_deref(), Some(tree::FAVORITE));
    for bad in [json!({"cspLibraryFolder": 3}), json!({"cspLibraryFolder": ""})] {
        set(&mut app, bad);
    }
    assert_eq!(app.ui.csp_library.folder.as_deref(), Some(tree::FAVORITE));
    set(&mut app, json!({"cspLibraryFolder": null}));
    assert!(app.ui.csp_library.folder.is_none());
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
    call(&mut app, json!({"cspLibraryView": "grid", "cspLibraryTags": ["lace", "레이스"], "cspLibraryFilter": "layers"}));
    assert_eq!(app.ui.csp_library.view, View::Grid);
    assert_eq!(app.ui.csp_library.filter, KindFilter::Layers);
    assert_eq!(app.ui.csp_library.tags, ["lace", "레이스"]);
    // Bad values change nothing.
    for bad in [json!({"cspLibraryView": "cards"}), json!({"cspLibraryFilter": "rulers"}), json!({"cspLibraryTags": "lace"}), json!({"cspLibraryTags": [""]}), json!({"cspLibraryTags": [3]})] {
        let (req, _rx) = crate::control::ControlRequest::new("ui.set", bad);
        let _ = crate::control::handle(&mut app, &ctx, &req);
    }
    assert_eq!(app.ui.csp_library.view, View::Grid);
    assert_eq!(app.ui.csp_library.tags, ["lace", "레이스"]);
    assert_eq!(app.ui.csp_library.filter, KindFilter::Layers);
}

fn with_layers() -> Value {
    let mut l = listing();
    l["layers"] = json!([
        {"uuid": "r", "name": "Isometric ruler", "kind": "image", "catalog": "Iso", "status": "ready", "guidance": null, "thumbnail": null, "tags": [], "ruler": true, "tiling": false, "folders": ["ダウンロード:Isometric ruler and grid-1758445:Isometric ruler and grid"]},
        {"uuid": "g", "name": "Isometric grid", "kind": "image", "catalog": "Iso", "status": "ready", "guidance": null, "thumbnail": null, "tags": [], "ruler": false, "tiling": true, "folders": ["ダウンロード:Isometric ruler and grid-1758445:Isometric ruler and grid"]}
    ]);
    l
}

#[test]
fn layer_materials_have_their_own_filter_and_need_a_document() {
    let mut app = app();
    app.ui.csp_library.listing = Some(with_layers());
    let mut h = harness(app);
    assert!(h.query_by_label("Isometric ruler").is_some(), "All lists layer materials too");
    h.state_mut().ui.csp_library.filter = KindFilter::Layers;
    h.run_steps(2);
    assert!(h.query_by_label("Ink pen").is_none() && h.query_by_label("Isometric grid").is_some());
    h.state_mut().ui.csp_library.filter = KindFilter::Brushes;
    h.run_steps(2);
    assert!(h.query_by_label("Isometric ruler").is_none() && h.query_by_label("Ink pen").is_some());
    assert!(!in_library(h.state(), &json!({"name": "Isometric ruler", "kind": "image", "catalog": "Iso"})));

    // No document: nothing is added and the selection stays for a second try.
    let mut a = super::tests::app();
    a.ui.csp_library.listing = Some(with_layers());
    a.ui.csp_library.selected = vec!["r".into()];
    import_selected(&mut a);
    assert_eq!(a.ui.csp_library.selected, ["r"]);
    // With a document, a material that cannot be read is reported, not added.
    a.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    let layers = a.session.active().unwrap().doc.layers.len();
    import_selected(&mut a);
    assert!(a.ui.status_error && a.ui.csp_library.selected.is_empty());
    assert_eq!(a.session.active().unwrap().doc.layers.len(), layers);
}

/// The user's own downloaded "Isometric ruler and grid" (ASSETS 1758445); never bundled.
#[test]
#[ignore = "reads the local CLIP STUDIO user data folder (PRISM_TEST_CSP_ROOT or the default)"]
fn local_isometric_ruler_is_added_from_the_panel() {
    let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    app.ui.csp_library.root = std::env::var_os("PRISM_TEST_CSP_ROOT").map(|r| r.to_string_lossy().into_owned());
    app.session.execute("file.new", json!({"width": 1280, "height": 720})).unwrap();
    refresh(&mut app);
    let listing = app.ui.csp_library.listing.clone().unwrap();
    assert!(materials(&listing).any(|b| b["uuid"] == "2dedbf35ba-a641-3e9e-79ad-6950ad1a78" && is_ruler(b)));
    app.ui.csp_library.selected = vec!["2dedbf35ba-a641-3e9e-79ad-6950ad1a78".into(), "8f65f2e32e-6d42-c3a8-ddbc-3cd24d4dd0".into()];
    import_selected(&mut app);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let doc = &app.session.active().unwrap().doc;
    assert!(doc.layers.iter().any(|l| l.name == "Isometric ruler" && l.special_rulers.is_some()));
    assert!(doc.layers.iter().any(|l| l.name == "Isometric grid"));
}

/// The wide palette over the local CLIP STUDIO folder, for a visual check:
/// `PRISM_LOOK_OUT=<dir> cargo test -p prism-ui-egui --lib render_the_material_palette -- --ignored`.
#[test]
#[ignore = "reads the local CLIP STUDIO folder and writes a PNG"]
fn render_the_material_palette() {
    let out = std::path::PathBuf::from(std::env::var_os("PRISM_LOOK_OUT").unwrap_or_else(|| "target/csp-ruler".into()));
    std::fs::create_dir_all(&out).unwrap();
    let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
    app.ui.csp_library.root = std::env::var_os("PRISM_TEST_CSP_ROOT").map(|r| r.to_string_lossy().into_owned());
    refresh(&mut app);
    app.ui.csp_library.view = View::Grid;
    app.ui.csp_library.folder = Some("ダウンロード:Isometric ruler and grid-1758445".into());
    app.ui.csp_library.expanded = vec!["ダウンロード".into()];
    let mut h = wide(app);
    h.set_size(egui::vec2(760.0, 620.0));
    h.run_steps(30);
    h.render().unwrap().save(out.join("palette-wide.png")).unwrap();
}
