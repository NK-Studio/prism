//! The Type tool's searchable font menu (#1369): it keeps its height and focuses its search field.

use egui::accesskit::Role;
use egui::{Key, Modifiers, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

struct State {
    current: String,
    families: Vec<String>,
    popup: egui::Id,
    favorites: Vec<String>,
    names: std::collections::HashMap<String, String>,
    korean: std::collections::HashSet<String>,
    filter: crate::state::TypeFontFilter,
}

fn harness() -> Harness<'static, State> {
    // Enough families that the unfiltered menu hits its maximum height and scrolls.
    let mut families: Vec<String> = (0..60).map(|i| format!("Family {i:02}")).collect();
    families.push("Zeta Sans".into());
    let state = State {
        current: "Family 00".into(),
        families,
        popup: egui::Id::NULL,
        favorites: Vec::new(),
        names: Default::default(),
        korean: Default::default(),
        filter: Default::default(),
    };
    let mut h = Harness::builder().with_size(vec2(600.0, 900.0)).build_ui_state(
        |ui, s: &mut State| {
            // `ComboBox::from_id_salt` wraps its salt in an `IdSalt`; the popup id is the button's + "popup".
            s.popup = ui.make_persistent_id(egui::IdSalt::new("type-font")).with("popup");
            let fams = s.families.clone();
            let action = super::font_picker_catalog(ui, &mut s.current, 170.0, &fams, &s.favorites, &s.names, &s.korean, &mut s.filter);
            if let Some((family, selected)) = action.favorite {
                s.favorites.retain(|f| f != &family);
                if selected {
                    s.favorites.push(family);
                }
            }
        },
        state,
    );
    h.run();
    h
}

fn popup_rect(h: &Harness<'static, State>) -> Option<Rect> {
    let id = h.state().popup;
    let open = egui::Popup::is_id_open(&h.ctx, id);
    if open { h.ctx.memory(|m| m.area_rect(id)) } else { None }
}

fn open(h: &mut Harness<'static, State>) {
    h.get_by_role(Role::ComboBox).click();
    h.run();
}

#[test]
fn korean_labels_search_and_select_the_canonical_font_and_favorite() {
    let mut h = harness();
    h.state_mut().families = vec!["Nanum Gothic".into(), "Inter".into()];
    h.state_mut().names.insert("Nanum Gothic".into(), "나눔고딕".into());
    open(&mut h);
    h.get_by_role(Role::TextInput).type_text("나눔");
    h.run();
    assert!(h.query_by_label("Inter").is_none());
    h.get_by_label("Favorite 나눔고딕").click();
    h.run();
    assert_eq!(h.state().favorites, ["Nanum Gothic"]);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_role(Role::TextInput).type_text("nanum");
    h.run();
    h.get_by_label("나눔고딕").click();
    h.run();
    assert_eq!(h.state().current, "Nanum Gothic");
    assert!(popup_rect(&h).is_none());
    assert_eq!(h.get_by_role(Role::ComboBox).value().as_deref(), Some("나눔고딕"), "the collapsed picker also displays the Korean name");
}

#[test]
fn clearing_the_query_restores_the_menu_height() {
    let mut h = harness();
    open(&mut h);
    let full = popup_rect(&h).expect("menu open").height();
    h.get_by_role(Role::TextInput).type_text("zeta");
    h.run();
    assert!(h.query_by_label("Family 01").is_none(), "the query filters the list");
    let searching = popup_rect(&h).expect("still open").height();
    for _ in 0.."zeta".len() {
        h.key_press(Key::Backspace);
    }
    h.run();
    assert!(h.query_by_label("Family 01").is_some(), "the cleared query lists every family");
    let cleared = popup_rect(&h).expect("still open").height();
    assert!((cleared - full).abs() < 0.5, "height after clearing {cleared} != original {full}");
    assert!((searching - full).abs() < 0.5, "the menu keeps its height while searching: {searching} vs {full}");
}

#[test]
fn reopening_focuses_the_search_field_and_clicking_it_keeps_the_menu_open() {
    let mut h = harness();
    open(&mut h);
    assert!(h.get_by_role(Role::TextInput).is_focused(), "search is focused on first open");
    h.get_by_role(Role::TextInput).type_text("zeta");
    h.run();
    // Close without choosing, then reopen: the query stays and the field is focused again.
    h.key_press(Key::Escape);
    h.run();
    assert!(popup_rect(&h).is_none(), "Escape closes the menu");
    open(&mut h);
    let search = h.get_by_role(Role::TextInput);
    assert_eq!(search.value().as_deref(), Some("zeta"));
    assert!(search.is_focused(), "search is focused on reopen with a query");
    // Clicking the search field (to edit the query) must not close the menu.
    h.get_by_role(Role::TextInput).click();
    h.run();
    assert!(popup_rect(&h).is_some(), "clicking the search field keeps the menu open");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_role(Role::TextInput).type_text("Zeta S");
    h.run();
    assert!(popup_rect(&h).is_some());
    // Picking a font still closes the menu.
    h.get_by_label("Zeta Sans").click();
    h.run();
    assert_eq!(h.state().current, "Zeta Sans");
    assert!(popup_rect(&h).is_none(), "choosing a font closes the menu");
    assert!(!h.ctx.text_edit_focused(), "the closed font search must release typing shortcuts to the canvas");
}

#[test]
fn starring_keeps_the_menu_open_and_moves_the_family_above_the_rest() {
    let mut h = harness();
    open(&mut h);
    h.get_by_role(Role::TextInput).type_text("zeta");
    h.run();
    h.get_by_label("Favorite Zeta Sans").click();
    h.run();
    assert_eq!(h.state().favorites, ["Zeta Sans"]);
    assert_eq!(h.state().current, "Family 00", "starring must not change the selected font");
    assert!(popup_rect(&h).is_some());
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.key_press(Key::Backspace);
    h.run();
    let zeta = h.get_by_label("Zeta Sans").rect();
    let first = h.get_by_label("Family 01").rect();
    assert!(zeta.top() < first.top(), "favorite is above alphabetical families");
    h.get_by_label("Unfavorite Zeta Sans").click();
    h.run();
    assert!(h.state().favorites.is_empty());
    assert!(popup_rect(&h).is_some());
}

#[test]
fn favorites_are_searchable_and_missing_favorites_are_not_listed() {
    let families = vec!["Alpha".into(), "Beta".into(), "Zeta".into()];
    let favorites = vec!["Zeta".into(), "Missing".into()];
    assert_eq!(super::ordered_families(&families, &favorites, ""), vec![&families[2], &families[0], &families[1]]);
    assert_eq!(super::ordered_families(&families, &favorites, "beta"), vec![&families[1]]);
}

#[test]
#[ignore = "writes offscreen Type-picker visual evidence"]
fn font_favorites_visual_evidence() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/type-font-picker");
    std::fs::create_dir_all(&out).unwrap();
    for (theme, ppp) in
        [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::StudioLight].into_iter().flat_map(|theme| [1.0, 2.0].map(move |ppp| (theme, ppp)))
    {
        let names = super::korean_family_names();
        let mut favorites: Vec<String> =
            names.iter().filter(|(_, label)| label.chars().any(|c| ('가'..='힣').contains(&c))).map(|(family, _)| family.clone()).collect();
        favorites.sort();
        favorites.truncate(3);
        assert!(!favorites.is_empty(), "visual fixture requires installed fonts with Korean names");
        let state = State {
            current: favorites.first().cloned().unwrap_or_else(|| "Inter".into()),
            families: super::families().as_ref().clone(),
            popup: egui::Id::NULL,
            favorites,
            names: names.as_ref().clone(),
            korean: super::font_catalog().korean.clone(),
            filter: Default::default(),
        };
        let mut h = Harness::builder().with_size(vec2(650.0, 650.0)).with_pixels_per_point(ppp).wgpu().build_ui_state(
            |ui, s: &mut State| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                crate::i18n::set_current(crate::i18n::Lang::from_code("ko").unwrap());
                super::font_picker_catalog(ui, &mut s.current, 250.0, &s.families, &s.favorites, &s.names, &s.korean, &mut s.filter);
            },
            state,
        );
        crate::PrismApp::setup_context(&h.ctx, theme);
        h.run_steps(4);
        h.get_by_role(Role::ComboBox).click();
        h.run();
        h.render().unwrap().save(out.join(format!("{theme:?}-{ppp}x.png"))).unwrap();
    }
}

#[test]
fn local_file_dialog_loads_a_font_and_refreshes_the_cached_families() {
    let (dialog, asked) =
        crate::file_dialog::fake(vec![Some(crate::FileDialogAnswer::Contents("Local.ttf".into(), prism_text::fonts::INTER_REGULAR.to_vec()))]);
    let app = crate::PrismApp::new(prism_engine::Session::new(), crate::Services { file_dialog: Some(dialog), ..Default::default() });
    let mut h = Harness::builder().with_size(vec2(600.0, 900.0)).build_ui_state(
        |ui, app: &mut crate::PrismApp| {
            let mut current = app.ui.tool_options.type_font.clone();
            super::font_picker(app, ui, &mut current, 170.0);
            app.ui.tool_options.type_font = current;
        },
        app,
    );
    let previous = super::families();
    let previous_names = super::korean_family_names();
    h.run();
    h.get_by_role(Role::ComboBox).click();
    h.run();
    h.get_by_label("Load Font File…").click();
    h.run();
    assert!(h.state().file_dialog_open());
    let ctx = h.ctx.clone();
    h.state_mut().poll_file_dialog(&ctx, None);
    h.state_mut().poll_file_dialog(&ctx, None);
    assert_eq!(asked.borrow().len(), 1);
    assert!(!h.state().file_dialog_open());
    assert!(!h.state().ui.status_error);
    assert!(h.state().ui.status.contains("Local.ttf"));
    assert!(!std::sync::Arc::ptr_eq(&previous, &super::families()), "local registration refreshes the cached picker list");
    assert!(!std::sync::Arc::ptr_eq(&previous_names, &super::korean_family_names()), "local registration also refreshes localized names");
}

#[test]
fn korean_filter_keeps_canonical_selection_and_excludes_latin_favorites() {
    let mut h = harness();
    h.state_mut().families = vec!["Inter".into(), "Nanum Gothic".into()];
    h.state_mut().favorites = vec!["Inter".into()];
    h.state_mut().korean.insert("Nanum Gothic".into());
    open(&mut h);
    h.get_by_label("Korean only").click();
    h.run();
    assert_eq!(h.state().filter, crate::state::TypeFontFilter::Korean);
    assert!(h.query_by_label("Favorite Inter").is_none());
    assert!(h.query_by_label("Unfavorite Inter").is_none());
    h.get_by_label("Nanum Gothic").click();
    h.run();
    assert_eq!(h.state().current, "Nanum Gothic");
    let serialized = serde_json::to_value(crate::state::TypeFontFilter::Korean).unwrap();
    assert_eq!(serialized, "korean");
}

#[test]
fn menu_groups_favorites_then_korean_then_other_names() {
    let mut h = harness();
    h.state_mut().families = vec!["Z Favorite".into(), "A Latin".into(), "Z Korean".into(), "B Korean".into()];
    h.state_mut().favorites = vec!["Z Favorite".into()];
    h.state_mut().korean = ["Z Korean".into(), "B Korean".into()].into_iter().collect();
    open(&mut h);
    let top = |label: &str| h.get_by_label(label).rect().min.y;
    assert!(top("Z Favorite") < top("B Korean"));
    assert!(top("B Korean") < top("Z Korean"));
    assert!(top("Z Korean") < top("A Latin"));
    assert_eq!(h.state().filter, crate::state::TypeFontFilter::All);
}
