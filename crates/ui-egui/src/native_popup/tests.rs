use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use super::*;
use crate::menus::{MenuItem, TOP_MENUS, menu_bar};

fn row(path: &[&str], id: &str, label: &str) -> MenuItem {
    MenuItem {
        id: id.into(),
        label: label.into(),
        path: path.iter().map(|s| s.to_string()).collect(),
        shortcut: None,
        enabled: true,
        checked: None,
        color: None,
    }
}

#[test]
fn rows_come_in_the_in_window_order_with_separators_tidied() {
    let mut new = row(&["File"], "file.new", "New…");
    new.shortcut = Some("Cmd+N".into());
    let mut grid = row(&["View", "Show"], "view.show.grid", "Grid");
    grid.checked = Some(true);
    let mut closed = row(&["File"], "file.close", "Close");
    closed.enabled = false;
    let rows = [
        row(&["File"], "---", "---"),
        new,
        row(&["File"], "---", "---"),
        row(&["File"], "---", "---"),
        row(&["File", "Open Recent"], "file.recent.0", "a.psd"),
        closed,
        row(&["File", "Open Recent"], "file.recent.1", "b.psd"),
        grid,
        row(&["File"], "---", "---"),
    ];
    let rows: Vec<&MenuItem> = rows.iter().collect();
    let got = entries(&rows, 1, Lang::EN);
    let item = |id: &str, label: &str, shortcut: Option<String>, enabled: bool, checked: Option<bool>| Entry::Item { id: id.into(), label: label.into(), shortcut, enabled, checked };
    assert_eq!(
        got,
        [
            item("file.new", "New…", Some(crate::shortcuts::pretty("Cmd+N")), true, None),
            Entry::Separator,
            Entry::Submenu {
                label: "Open Recent".into(),
                enabled: true,
                children: vec![item("file.recent.0", "a.psd", None, true, None), item("file.recent.1", "b.psd", None, true, None)],
            },
            item("file.close", "Close", None, false, None),
            Entry::Submenu { label: "Show".into(), enabled: true, children: vec![item("view.show.grid", "Grid", None, true, Some(true))] },
        ],
        "no leading, doubled or trailing separator; a submenu where its first row is"
    );
}

/// A scripted platform: records each request and answers with the next outcome.
fn fake(outcomes: Vec<Outcome>) -> (crate::Services, Rc<RefCell<Vec<Request>>>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let mut outcomes = VecDeque::from(outcomes);
    let services = crate::Services {
        popup_menu: Some(Box::new(move |r: &Request| {
            log.borrow_mut().push(r.clone());
            outcomes.pop_front().unwrap_or(Outcome::Closed)
        })),
        ..Default::default()
    };
    (services, seen)
}

/// The menu bar alone, with the frame start's [`perform`], in an app whose windows are native.
fn bar(services: crate::Services) -> Harness<'static, PrismApp> {
    let app = PrismApp::new(prism_engine::Session::new(), services);
    let h = Harness::builder().with_size(egui::vec2(1200.0, 200.0)).build_ui_state(
        |ui, app: &mut PrismApp| {
            app.frame += 1;
            perform(app, ui.ctx());
            menu_bar(app, ui);
        },
        app,
    );
    PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.ctx.set_embed_viewports(false);
    h
}

#[test]
fn a_title_press_opens_its_native_menu_and_runs_the_choice() {
    let (services, seen) = fake(vec![Outcome::Chosen { id: "file.new".into(), alt: false }]);
    let mut h = bar(services);
    h.run_steps(2);
    h.get_by_label("File").click();
    h.run_steps(6);
    let seen = seen.borrow();
    assert_eq!(seen.len(), 1, "one popup");
    assert_eq!(seen[0].index, 0);
    assert_eq!(seen[0].titles.len(), TOP_MENUS.len());
    assert!(seen[0].entries.iter().any(|e| matches!(e, Entry::Item { id, .. } if id == "file.new")), "File's rows");
    assert!(h.query_by_label_contains("Open…").is_none(), "no in-window menu");
    assert!(!h.state().ui.dialogs.is_empty(), "File › New… ran: its dialog is open");
}

#[test]
fn moving_to_another_title_opens_that_menu_and_help_stays_in_the_window() {
    let help = TOP_MENUS.iter().position(|t| *t == "Help").unwrap();
    let (services, seen) = fake(vec![Outcome::Switch(1), Outcome::Switch(help)]);
    let mut h = bar(services);
    h.run_steps(2);
    h.get_by_label("File").click();
    h.run_steps(10);
    let indices: Vec<usize> = seen.borrow().iter().map(|r| r.index).collect();
    assert_eq!(indices, [0, 1], "File, then Edit; Help isn't a native menu");
    assert!(egui::Popup::is_any_open(&h.ctx), "Help opened in the window");
}

#[test]
fn automation_input_keeps_the_in_window_menus() {
    let (services, seen) = fake(vec![]);
    let mut h = bar(services);
    h.state_mut().automation_input = true;
    // The harness keeps the flag on: the shell clears it each frame after automation input.
    h.get_by_label("File").click();
    h.state_mut().automation_input = true;
    h.run_steps(1);
    h.state_mut().automation_input = true;
    h.run_steps(1);
    assert!(seen.borrow().is_empty(), "never blocks on a native popup");
    assert!(h.query_by_label_contains("Open…").is_some(), "the in-window menu opened");
    let _ = json!({});
}
