//! Windows: the menu bar's menus as native popup menus (`prism_ui_egui::native_popup`), through
//! winsafe's safe Win32 bindings: `TrackPopupMenu` shows a menu below its title in a window of
//! its own, above the always-on-top floating document windows.
//!
//! A native menu bar opens the neighbouring menu when the pointer moves onto another title, and
//! on ← / → at the top level. A popup menu doesn't, so while one is up a thread timer (it runs
//! inside the menu's modal loop) watches the pointer and the arrow keys and closes the menu with
//! `EndMenu`; the app then opens the next one ([`Outcome::Switch`]).
use std::cell::RefCell;

use prism_ui_egui::native_popup::{Entry, Outcome, PopupMenuFn, Request};
use winsafe::{self as w, HMENU, HWND, co, prelude::*};

/// How often the timer looks at the pointer and keys while a menu is up.
const TICK_MS: u32 = 15;

pub fn service() -> PopupMenuFn {
    Box::new(|req| match show(req) {
        Ok(outcome) => outcome,
        Err(e) => {
            log::warn!("native menu: {e}");
            Outcome::Closed
        }
    })
}

/// A popup menu and every submenu in it, destroyed together when dropped.
struct Popup(HMENU);

impl Drop for Popup {
    fn drop(&mut self) {
        let _ = self.0.DestroyMenu();
    }
}

/// `&` marks a mnemonic in a menu label; Prism's labels have none.
fn menu_text(label: &str, shortcut: Option<&str>) -> String {
    let label = label.replace('&', "&&");
    match shortcut {
        Some(s) => format!("{label}\t{}", s.replace('&', "&&")),
        None => label,
    }
}

/// Command ids are 16 bits and 0 means "no choice": `ids[n - 1]` is the command of item `n`.
fn fill(menu: &HMENU, entries: &[Entry], ids: &mut Vec<String>) -> w::SysResult<()> {
    for e in entries {
        match e {
            Entry::Separator => menu.AppendMenu(co::MF::SEPARATOR, w::IdMenu::None, w::BmpPtrStr::None)?,
            Entry::Item { id, label, shortcut, enabled, checked } => {
                let Ok(cmd) = u16::try_from(ids.len() + 1) else { continue };
                ids.push(id.clone());
                let mut flags = co::MF::STRING;
                if !enabled {
                    flags |= co::MF::GRAYED;
                }
                if *checked == Some(true) {
                    flags |= co::MF::CHECKED;
                }
                menu.AppendMenu(flags, w::IdMenu::Id(cmd), w::BmpPtrStr::from_str(&menu_text(label, shortcut.as_deref())))?;
            }
            Entry::Submenu { label, enabled, children } => {
                let sub = HMENU::CreatePopupMenu()?;
                // Owned by `menu` once appended; until then it is ours to free.
                let mut sub = Popup(sub);
                fill(&sub.0, children, ids)?;
                let mut flags = co::MF::POPUP;
                if !enabled || children.is_empty() {
                    flags |= co::MF::GRAYED;
                }
                menu.AppendMenu(flags, w::IdMenu::Menu(&sub.0), w::BmpPtrStr::from_str(&menu_text(label, None)))?;
                // `menu` destroys it now.
                sub.0 = HMENU::NULL;
            }
        }
    }
    Ok(())
}

/// The window the menu belongs to: Prism's main window, matched by its frame (as
/// `window_opacity` finds a document window), else the foreground window if it is Prism's.
fn owner(window: Option<egui::Rect>) -> Option<HWND> {
    let pid = std::process::id();
    let mut found = None;
    if let Some(want) = window {
        let _ = w::EnumWindows(|h| {
            if h.GetWindowThreadProcessId().1 == pid
                && h.IsWindowVisible()
                && let Ok(r) = h.GetWindowRect()
            {
                let got = egui::Rect::from_min_max(egui::pos2(r.left as f32, r.top as f32), egui::pos2(r.right as f32, r.bottom as f32));
                if (got.min - want.min).length() < 3.0 && (got.size() - want.size()).length() < 3.0 {
                    found = Some(h);
                    return false;
                }
            }
            true
        });
    }
    found.or_else(|| HWND::GetForegroundWindow().filter(|h| h.GetWindowThreadProcessId().1 == pid))
}

/// What the timer watches while a menu is up.
struct Watch {
    titles: Vec<egui::Rect>,
    index: usize,
    switch: Option<usize>,
    /// The arrow keys' toggle bits at the last tick: each press flips them.
    left: bool,
    right: bool,
    /// Menu windows open at the last tick: more than one means a submenu is open.
    open: usize,
    /// The pointer at the last tick: only a pointer moving onto another title switches (one
    /// resting on the title it opened from must not undo ← / →).
    pointer: Option<egui::Pos2>,
}

impl Watch {
    /// One tick: the pointer, the arrow keys' toggle bits and the open menu windows now. Returns
    /// the title to switch to (once).
    fn step(&mut self, pointer: Option<egui::Pos2>, left: bool, right: bool, open: usize) -> Option<usize> {
        if self.switch.is_some() {
            return None;
        }
        let n = self.titles.len();
        let (pressed_left, pressed_right) = (left != self.left, right != self.right);
        // Only the top menu was open before the key and still is: ← / → leave it. With a
        // submenu open, ← closes the submenu and → opens one (Windows does both itself).
        let top_only = self.open <= 1 && open <= 1;
        let moved = pointer.filter(|p| self.pointer != Some(*p));
        let next = if let Some(i) = moved.and_then(|p| self.titles.iter().position(|t| t.contains(p))).filter(|i| *i != self.index) {
            Some(i)
        } else if n > 0 && top_only && pressed_right {
            Some((self.index + 1) % n)
        } else if n > 0 && top_only && pressed_left {
            Some((self.index + n - 1) % n)
        } else {
            None
        };
        (self.left, self.right, self.open, self.pointer) = (left, right, open, pointer);
        self.switch = next;
        next
    }
}

thread_local! {
    static WATCH: RefCell<Option<Watch>> = const { RefCell::new(None) };
}

/// Prism's open menu windows (the top menu and its open submenus).
fn open_menus() -> usize {
    let mut n = 0;
    let _ = w::EnumThreadWindows(w::GetCurrentThreadId(), |h| {
        if h.IsWindowVisible() && h.GetClassName().is_ok_and(|c| c == "#32768") {
            n += 1;
        }
        true
    });
    n
}

extern "system" fn tick(_: HWND, _: co::WM, _: usize, _: u32) {
    let pointer = w::GetCursorPos().ok().map(|p| egui::pos2(p.x as f32, p.y as f32));
    let left = w::GetKeyState(co::VK::LEFT).1;
    let right = w::GetKeyState(co::VK::RIGHT).1;
    let open = open_menus();
    let switch = WATCH.with_borrow_mut(|watch| watch.as_mut().and_then(|watch| watch.step(pointer, left, right, open)));
    if switch.is_some() {
        let _ = w::EndMenu();
    }
}

fn show(req: &Request) -> w::SysResult<Outcome> {
    let Some(title) = req.titles.get(req.index).copied() else { return Ok(Outcome::Closed) };
    let owner = owner(req.window).ok_or(co::ERROR::INVALID_WINDOW_HANDLE)?;
    let menu = Popup(HMENU::CreatePopupMenu()?);
    let mut ids = Vec::new();
    fill(&menu.0, &req.entries, &mut ids)?;
    WATCH.set(Some(Watch {
        titles: req.titles.clone(),
        index: req.index,
        switch: None,
        left: w::GetKeyState(co::VK::LEFT).1,
        right: w::GetKeyState(co::VK::RIGHT).1,
        open: 0,
        pointer: w::GetCursorPos().ok().map(|p| egui::pos2(p.x as f32, p.y as f32)),
    }));
    let timer = HWND::NULL.SetTimer(0, TICK_MS, Some(tick)).ok();
    // A popup menu closes on clicks elsewhere only when its owner is the foreground window.
    let _ = owner.SetForegroundWindow();
    let at = w::POINT { x: title.left().round() as i32, y: title.bottom().round() as i32 };
    let chosen = menu.0.TrackPopupMenu(co::TPM::LEFTALIGN | co::TPM::TOPALIGN | co::TPM::LEFTBUTTON | co::TPM::RETURNCMD | co::TPM::NONOTIFY, at, &owner);
    if let Some(timer) = timer {
        let _ = HWND::NULL.KillTimer(timer);
    }
    let switch = WATCH.take().and_then(|w| w.switch);
    if let Some(next) = switch {
        return Ok(Outcome::Switch(next));
    }
    let alt = w::GetKeyState(co::VK::MENU).0;
    Ok(match chosen? {
        Some(n) => usize::try_from(n).ok().and_then(|n| ids.get(n.checked_sub(1)?)).map_or(Outcome::Closed, |id| Outcome::Chosen { id: id.clone(), alt }),
        None => Outcome::Closed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_escape_ampersands_and_put_the_shortcut_after_a_tab() {
        assert_eq!(menu_text("Find & Replace…", Some("Ctrl+F")), "Find && Replace…\tCtrl+F");
        assert_eq!(menu_text("Grid", None), "Grid");
    }

    /// Three titles side by side; the File menu (0) is up and the pointer rests on its title.
    fn watch() -> Watch {
        let title = |i: f32| egui::Rect::from_min_size(egui::pos2(i * 50.0, 0.0), egui::vec2(50.0, 20.0));
        Watch { titles: vec![title(0.0), title(1.0), title(2.0)], index: 0, switch: None, left: false, right: false, open: 0, pointer: Some(egui::pos2(20.0, 10.0)) }
    }

    #[test]
    fn the_pointer_moving_onto_another_title_switches_menus() {
        let mut w = watch();
        assert_eq!(w.step(Some(egui::pos2(20.0, 10.0)), false, false, 1), None, "resting on its own title");
        assert_eq!(w.step(Some(egui::pos2(30.0, 60.0)), false, false, 1), None, "inside the menu");
        assert_eq!(w.step(Some(egui::pos2(120.0, 10.0)), false, false, 1), Some(2));
        assert_eq!(w.step(Some(egui::pos2(70.0, 10.0)), false, false, 1), None, "once");
    }

    #[test]
    fn arrow_keys_leave_the_top_menu_but_not_a_submenu() {
        // → (a toggle flip) with the pointer resting on another title: the key wins, the pointer
        // that didn't move doesn't take it back (it did: → opened Edit, then File again).
        let mut w = Watch { index: 1, pointer: Some(egui::pos2(20.0, 10.0)), ..watch() };
        assert_eq!(w.step(Some(egui::pos2(20.0, 10.0)), false, true, 1), Some(2));
        // ← wraps around.
        let mut w = watch();
        assert_eq!(w.step(w.pointer, true, false, 1), Some(2));
        // With a submenu open (two menu windows), ← closes it: no switch.
        let mut w = watch();
        assert_eq!(w.step(w.pointer, false, false, 2), None);
        assert_eq!(w.step(w.pointer, true, false, 1), None, "the submenu was open before the key");
        // → that opened a submenu: no switch either.
        let mut w = watch();
        assert_eq!(w.step(w.pointer, false, false, 1), None);
        assert_eq!(w.step(w.pointer, false, true, 2), None);
    }

    #[test]
    fn menus_build_with_submenus_and_number_their_items() {
        let entries = vec![
            Entry::Item { id: "file.new".into(), label: "New…".into(), shortcut: Some("Ctrl+N".into()), enabled: true, checked: None },
            Entry::Separator,
            Entry::Submenu {
                label: "Open Recent".into(),
                enabled: true,
                children: vec![Entry::Item { id: "file.recent.0".into(), label: "a.psd".into(), shortcut: None, enabled: false, checked: Some(true) }],
            },
            Entry::Submenu { label: "Empty".into(), enabled: true, children: vec![] },
        ];
        let menu = Popup(HMENU::CreatePopupMenu().unwrap());
        let mut ids = Vec::new();
        fill(&menu.0, &entries, &mut ids).unwrap();
        assert_eq!(ids, ["file.new", "file.recent.0"]);
        assert_eq!(menu.0.GetMenuItemCount().unwrap(), 4);
        assert_eq!(menu.0.GetSubMenu(2).unwrap().GetMenuItemCount().unwrap(), 1);
        assert_eq!(menu.0.GetSubMenu(3).unwrap().GetMenuItemCount().unwrap(), 0);
        // By command id: winsafe 0.0.29's by-position GetMenuState passes 1, not MF_BYPOSITION.
        let state = menu.0.GetMenuState(w::IdPos::Id(2)).unwrap();
        assert!(state.has(co::MF::GRAYED) && state.has(co::MF::CHECKED), "{state:?}");
    }
}
