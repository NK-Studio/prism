//! Hover in a window that isn't the key window. macOS sends mouse-moved events to the key window
//! only, so with a floating document focused the main window's buttons and panels saw no pointer
//! (and the other way round): hover highlights, tooltips and the brush cursor stayed where the
//! pointer had last been while that window was focused. This input hook reads the pointer from the
//! OS and feeds it to the unfocused window under it. Where the OS already delivers motion to the
//! window under the pointer (Windows) no probe is installed and the hook does nothing.
//!
//! The cursor's shape has the same gap: winit sets a window's cursor through AppKit cursor
//! rects, which only the key window's are honoured, and egui-winit skips a window that has seen
//! no native pointer. So at the end of a pass that fed a window its hover, the cursor that window
//! asked for goes to the OS directly ([`install_cursor`]).
use egui::{Context, Event, Id, Pos2, ViewportId};
use std::sync::Arc;

/// Where the OS pointer is, in points (unscaled by egui's zoom).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoverProbe {
    /// On the desktop, from the top-left of the primary screen (the frame winit reports window
    /// positions in, before its scale factor).
    pub screen: Pos2,
    /// In the content area of the frontmost of our windows under the pointer.
    pub window: Pos2,
}
/// Reads the pointer from the OS; `None` when it is over none of our windows.
pub type HoverProbeFn = Arc<dyn Fn() -> Option<HoverProbe> + Send + Sync>;

#[derive(Clone)]
struct Installed(HoverProbeFn);
fn probe_key() -> Id {
    Id::new("hover-follow-probe")
}
/// Let unfocused windows follow the pointer with `probe` (the app binary's OS binding).
pub fn install(ctx: &Context, probe: HoverProbeFn) {
    ctx.data_mut(|d| d.insert_temp(probe_key(), Installed(probe)));
}

/// The cursor an unfocused window under the pointer asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum HoverCursor {
    /// A stock cursor; `CursorIcon::None` hides the pointer (the canvas draws its own).
    Icon(egui::CursorIcon),
    /// A bitmap (tool artwork, brush tips), sized in points as winit's AppKit backend takes it.
    Image(egui::CustomCursorImage),
    /// The pointer left for somewhere that isn't ours: back to the arrow.
    Release,
}
/// Shows a cursor now, whichever window is key.
pub type HoverCursorFn = Arc<dyn Fn(&HoverCursor) + Send + Sync>;

#[derive(Clone)]
struct InstalledCursor(HoverCursorFn);
fn cursor_key() -> Id {
    Id::new("hover-follow-cursor")
}
/// Let unfocused windows set the cursor's shape with `set` (the app binary's OS binding).
pub fn install_cursor(ctx: &Context, set: HoverCursorFn) {
    ctx.data_mut(|d| d.insert_temp(cursor_key(), InstalledCursor(set)));
}
fn set_cursor(ctx: &Context, cursor: &HoverCursor) {
    if let Some(InstalledCursor(set)) = ctx.data(|d| d.get_temp::<InstalledCursor>(cursor_key())) {
        set(cursor);
    }
}

/// The cursor a pass's output asks for: the bitmap when there is one, as egui-winit prefers.
pub(crate) fn wanted(icon: egui::CursorIcon, image: Option<egui::CustomCursorImage>) -> HoverCursor {
    image.map_or(HoverCursor::Icon(icon), HoverCursor::Image)
}

/// What the hook did last in a viewport: the hover there is ours to take away again.
#[derive(Clone, Copy, Default, PartialEq)]
struct Followed(Option<Pos2>);
fn followed_key(id: ViewportId) -> Id {
    Id::new(("hover-follow", id))
}
/// Viewports with a pointer button held: a drag owns the pointer, wherever it goes.
#[derive(Clone, Default)]
struct Held(Vec<ViewportId>);
fn held_key() -> Id {
    Id::new("hover-follow-held")
}

/// How far the two readings of one pointer may differ (points) and still be the same window.
const SAME_WINDOW: f32 = 1.5;

/// The pointer in `inner` (a viewport's content area on the desktop, in egui points) when that
/// viewport's window is the one `probe` found under the pointer. Two windows give the same
/// content position only when they sit exactly on top of each other.
pub(crate) fn over(probe: HoverProbe, inner_min: Pos2, zoom: f32) -> Option<Pos2> {
    if !(zoom.is_finite() && zoom > 0.0) {
        return None;
    }
    let here = (probe.screen.to_vec2() / zoom - inner_min.to_vec2()).to_pos2();
    let front = (probe.window.to_vec2() / zoom).to_pos2();
    (here.distance(front) <= SAME_WINDOW).then_some(front)
}

/// The event to add for an unfocused viewport: the pointer when it is `over` it and has moved,
/// `PointerGone` once when it has left a viewport this hook was feeding.
pub(crate) fn step(followed: Option<Pos2>, over: Option<Pos2>) -> Option<Event> {
    match (followed, over) {
        (Some(old), Some(new)) if old == new => None,
        (_, Some(new)) => Some(Event::PointerMoved(new)),
        (Some(_), None) => Some(Event::PointerGone),
        (None, None) => None,
    }
}

/// The pointer came back to the key window from an unfocused one whose cursor we had set. AppKit
/// doesn't look at the key window's cursor rect again for that (the pointer never left its
/// frame when the other window floats above it), so the arrow AppKit shows over other windows
/// stayed. Until the pointer leaves the key window, its cursor goes to the OS from here as well.
fn handed_key() -> Id {
    Id::new("hover-follow-handed-back")
}
fn handed_back(ctx: &Context) -> bool {
    if !ctx.data(|d| d.get_temp::<bool>(handed_key())).unwrap_or(false) {
        return false;
    }
    let (focused, inner) = ctx.input(|i| (i.viewport().focused == Some(true), i.viewport().inner_rect));
    if !focused {
        return false;
    }
    let here = ctx
        .data(|d| d.get_temp::<Installed>(probe_key()))
        .and_then(|Installed(probe)| probe())
        .zip(inner)
        .and_then(|(p, inner)| over(p, inner.min, ctx.zoom_factor()));
    if here.is_none() {
        ctx.data_mut(|d| d.remove::<bool>(handed_key()));
    }
    here.is_some()
}

pub(crate) struct HoverFollow;

impl egui::plugin::Plugin for HoverFollow {
    fn debug_name(&self) -> &'static str {
        "Prism hover in unfocused windows"
    }
    fn input_hook(&mut self, ctx: &Context, raw: &mut egui::RawInput) {
        let id = raw.viewport_id;
        // Which viewports hold a button, from the events egui is about to see.
        let mut native_pointer = false;
        for e in &raw.events {
            match e {
                Event::PointerButton { pressed, .. } => {
                    native_pointer = true;
                    let pressed = *pressed;
                    ctx.data_mut(|d| {
                        let held = &mut d.get_temp_mut_or_default::<Held>(held_key()).0;
                        held.retain(|v| *v != id);
                        if pressed {
                            held.push(id);
                        }
                    });
                }
                Event::PointerMoved(_) | Event::PointerGone => native_pointer = true,
                Event::WindowFocused(false) => ctx.data_mut(|d| d.get_temp_mut_or_default::<Held>(held_key()).0.retain(|v| *v != id)),
                _ => {}
            }
        }
        let Some(Installed(probe)) = ctx.data(|d| d.get_temp::<Installed>(probe_key())) else { return };
        let followed = ctx.data(|d| d.get_temp::<Followed>(followed_key(id))).unwrap_or_default();
        let forget = |ctx: &Context| ctx.data_mut(|d| d.remove::<Followed>(followed_key(id)));
        let Some(info) = raw.viewports.get(&id) else { return };
        // The key window gets its own motion, as does any window while a click arrives; scripted
        // input (the control channel) is not to be overruled by where the real pointer rests.
        if info.focused != Some(false) || native_pointer || crate::doc_windows::automated_in(ctx, id) {
            if followed.0.is_some() {
                forget(ctx);
            }
            return;
        }
        // Only between Prism's own windows: in the background nothing hovers, as before.
        let active = raw.viewports.values().any(|v| v.focused == Some(true));
        let dragging = ctx.data(|d| d.get_temp::<Held>(held_key())).is_some_and(|h| !h.0.is_empty());
        let at = if active && !dragging { info.inner_rect.and_then(|inner| probe().and_then(|p| over(p, inner.min, ctx.zoom_factor()))) } else { None };
        let Some(event) = step(followed.0, at) else { return };
        // Leaving for another of our windows, that window's cursor follows: the key window's by
        // its cursor rect, an unfocused one's at the end of its pass. Elsewhere nobody would
        // take our cursor back.
        if at.is_none() {
            if probe().is_none() {
                set_cursor(ctx, &HoverCursor::Release);
            } else {
                ctx.data_mut(|d| d.insert_temp(handed_key(), true));
            }
        }
        raw.events.push(event);
        ctx.data_mut(|d| d.insert_temp(followed_key(id), Followed(at)));
    }

    /// After `CursorLifecycle` has settled the pass's cursor.
    fn on_end_pass(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx();
        let fed = ctx.data(|d| d.get_temp::<Followed>(followed_key(ctx.viewport_id()))).is_some_and(|f| f.0.is_some());
        if fed || handed_back(ctx) {
            let cursor = ctx.output(|o| wanted(o.cursor_icon, o.cursor_image.clone()));
            set_cursor(ctx, &cursor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Plugin as _, pos2};

    fn probe(screen: Pos2, window: Pos2) -> HoverProbe {
        HoverProbe { screen, window }
    }

    #[test]
    fn the_pointer_is_over_the_window_both_readings_agree_on() {
        // A window whose content starts at (300, 200) on the desktop, pointer 40, 60 into it.
        let p = probe(pos2(340.0, 260.0), pos2(40.0, 60.0));
        assert_eq!(over(p, pos2(300.0, 200.0), 1.0), Some(pos2(40.0, 60.0)));
        // Another window lies under the pointer elsewhere: the frontmost one isn't this one.
        assert_eq!(over(p, pos2(100.0, 100.0), 1.0), None);
    }

    #[test]
    fn interface_zoom_scales_both_readings() {
        // At 200 % egui's points are twice the OS's: content origin (150, 100) egui points.
        let p = probe(pos2(340.0, 260.0), pos2(40.0, 60.0));
        assert_eq!(over(p, pos2(150.0, 100.0), 2.0), Some(pos2(20.0, 30.0)));
        assert_eq!(over(p, pos2(150.0, 100.0), 0.0), None);
        assert_eq!(over(p, pos2(150.0, 100.0), f32::NAN), None);
    }

    #[test]
    fn follows_then_lets_go_once() {
        let a = pos2(10.0, 10.0);
        let b = pos2(12.0, 10.0);
        assert_eq!(step(None, Some(a)), Some(Event::PointerMoved(a)));
        assert_eq!(step(Some(a), Some(a)), None, "a resting pointer adds nothing");
        assert_eq!(step(Some(a), Some(b)), Some(Event::PointerMoved(b)));
        assert_eq!(step(Some(b), None), Some(Event::PointerGone));
        assert_eq!(step(None, None), None, "hover it never fed is not its to remove");
    }

    /// One pass of `id` with `focused` windows, returning the events egui would see.
    fn pass(ctx: &Context, id: ViewportId, focused: ViewportId, events: Vec<Event>) -> Vec<Event> {
        let mut raw = egui::RawInput { viewport_id: id, events, ..Default::default() };
        for (v, min) in [(ViewportId::ROOT, pos2(0.0, 0.0)), (ViewportId::from_hash_of("doc"), pos2(1000.0, 0.0))] {
            let info = egui::ViewportInfo {
                focused: Some(v == focused),
                inner_rect: Some(egui::Rect::from_min_size(min, egui::vec2(800.0, 600.0))),
                ..Default::default()
            };
            raw.viewports.insert(v, info);
        }
        HoverFollow.input_hook(ctx, &mut raw);
        raw.events
    }

    #[test]
    fn an_unfocused_main_window_hovers_while_a_document_window_is_focused() {
        let ctx = Context::default();
        let doc = ViewportId::from_hash_of("doc");
        let at = Arc::new(std::sync::Mutex::new(Some(probe(pos2(200.0, 300.0), pos2(200.0, 300.0)))));
        let read = at.clone();
        install(&ctx, Arc::new(move || *read.lock().unwrap()));
        // The pointer is over the main window; the document window has the keyboard.
        assert_eq!(pass(&ctx, ViewportId::ROOT, doc, vec![]), vec![Event::PointerMoved(pos2(200.0, 300.0))]);
        assert_eq!(pass(&ctx, doc, doc, vec![]), vec![], "the key window keeps its own motion");
        assert_eq!(pass(&ctx, ViewportId::ROOT, doc, vec![]), vec![], "unmoved");
        // It moves onto the document window (frontmost there): the main window lets go.
        *at.lock().unwrap() = Some(probe(pos2(1100.0, 50.0), pos2(100.0, 50.0)));
        assert_eq!(pass(&ctx, ViewportId::ROOT, doc, vec![]), vec![Event::PointerGone]);
        assert_eq!(pass(&ctx, ViewportId::ROOT, doc, vec![]), vec![]);
        // And the other way round, with the main window focused.
        assert_eq!(pass(&ctx, doc, ViewportId::ROOT, vec![]), vec![Event::PointerMoved(pos2(100.0, 50.0))]);
    }

    #[test]
    fn a_drag_from_the_key_window_does_not_hover_the_others() {
        let ctx = Context::default();
        let doc = ViewportId::from_hash_of("doc");
        install(&ctx, Arc::new(|| Some(probe(pos2(200.0, 300.0), pos2(200.0, 300.0)))));
        let button = |pressed| Event::PointerButton { pos: pos2(5.0, 5.0), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        pass(&ctx, doc, doc, vec![button(true)]);
        assert_eq!(pass(&ctx, ViewportId::ROOT, doc, vec![]), vec![], "a brush stroke leaving its window");
        pass(&ctx, doc, doc, vec![button(false)]);
        assert_eq!(pass(&ctx, ViewportId::ROOT, doc, vec![]), vec![Event::PointerMoved(pos2(200.0, 300.0))]);
    }

    #[test]
    fn the_hovered_window_sets_the_cursor_and_gives_it_back_on_leaving_prism() {
        let ctx = Context::default();
        let doc = ViewportId::from_hash_of("doc");
        let at = Arc::new(std::sync::Mutex::new(Some(probe(pos2(200.0, 300.0), pos2(200.0, 300.0)))));
        let read = at.clone();
        install(&ctx, Arc::new(move || *read.lock().unwrap()));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        install_cursor(&ctx, Arc::new(move |c: &HoverCursor| log.lock().unwrap().push(c.clone())));
        // A pass of the main window (unfocused, under the pointer) whose widget asks for a hand.
        let end = |ctx: &Context| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                HoverFollow.on_end_pass(ui);
            });
            out.textures_delta.clear();
        };
        end(&ctx);
        assert!(seen.lock().unwrap().is_empty(), "a window nobody feeds keeps out of the cursor");
        pass(&ctx, ViewportId::ROOT, doc, vec![]);
        end(&ctx);
        assert_eq!(*seen.lock().unwrap(), vec![HoverCursor::Icon(egui::CursorIcon::PointingHand)]);
        // On to the document window: its own cursor takes over, nothing to give back.
        *at.lock().unwrap() = Some(probe(pos2(1100.0, 50.0), pos2(100.0, 50.0)));
        pass(&ctx, ViewportId::ROOT, doc, vec![]);
        end(&ctx);
        assert_eq!(seen.lock().unwrap().len(), 1);
        // Back, then off to the desktop: the arrow returns.
        *at.lock().unwrap() = Some(probe(pos2(200.0, 300.0), pos2(200.0, 300.0)));
        pass(&ctx, ViewportId::ROOT, doc, vec![]);
        *at.lock().unwrap() = None;
        pass(&ctx, ViewportId::ROOT, doc, vec![]);
        assert_eq!(seen.lock().unwrap().last(), Some(&HoverCursor::Release));
    }

    #[test]
    fn the_key_window_takes_the_cursor_back_until_the_pointer_leaves_it() {
        let ctx = Context::default();
        let doc = ViewportId::from_hash_of("doc");
        let at = Arc::new(std::sync::Mutex::new(Some(probe(pos2(1100.0, 50.0), pos2(100.0, 50.0)))));
        let read = at.clone();
        install(&ctx, Arc::new(move || *read.lock().unwrap()));
        let seen = Arc::new(std::sync::Mutex::new(0usize));
        let count = seen.clone();
        install_cursor(&ctx, Arc::new(move |_: &HoverCursor| *count.lock().unwrap() += 1));
        // The main window (key) ends a pass, its content at the desktop's origin.
        let end = |ctx: &Context| {
            let mut raw = egui::RawInput::default();
            let info = egui::ViewportInfo {
                focused: Some(true),
                inner_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(800.0, 600.0))),
                ..Default::default()
            };
            raw.viewports.insert(ViewportId::ROOT, info);
            let mut out = ctx.run_ui(raw, |ui| HoverFollow.on_end_pass(ui));
            out.textures_delta.clear();
        };
        end(&ctx);
        assert_eq!(*seen.lock().unwrap(), 0, "the key window's cursor is winit's business");
        // Over the unfocused document window, then back onto the main window.
        pass(&ctx, doc, ViewportId::ROOT, vec![]);
        *at.lock().unwrap() = Some(probe(pos2(200.0, 300.0), pos2(200.0, 300.0)));
        assert_eq!(pass(&ctx, doc, ViewportId::ROOT, vec![]), vec![Event::PointerGone]);
        end(&ctx);
        end(&ctx);
        assert_eq!(*seen.lock().unwrap(), 2, "every pass while the pointer stays");
        // Off to the desktop and back in: AppKit sets the cursor rect's cursor itself again.
        *at.lock().unwrap() = None;
        end(&ctx);
        *at.lock().unwrap() = Some(probe(pos2(200.0, 300.0), pos2(200.0, 300.0)));
        end(&ctx);
        assert_eq!(*seen.lock().unwrap(), 2);
    }

    #[test]
    fn a_bitmap_cursor_wins_over_the_icon() {
        let image = egui::CustomCursorImage { rgba: Arc::from(vec![0u8; 4]), size: [1, 1], hotspot: [0, 0] };
        assert_eq!(wanted(egui::CursorIcon::Crosshair, Some(image.clone())), HoverCursor::Image(image));
        assert_eq!(wanted(egui::CursorIcon::None, None), HoverCursor::Icon(egui::CursorIcon::None));
    }

    #[test]
    fn nothing_hovers_without_a_probe_or_in_the_background() {
        let ctx = Context::default();
        let doc = ViewportId::from_hash_of("doc");
        assert_eq!(pass(&ctx, ViewportId::ROOT, doc, vec![]), vec![], "no probe (Windows): the OS delivers motion itself");
        install(&ctx, Arc::new(|| Some(probe(pos2(200.0, 300.0), pos2(200.0, 300.0)))));
        let nobody = ViewportId::from_hash_of("another app");
        assert_eq!(pass(&ctx, ViewportId::ROOT, nobody, vec![]), vec![], "Prism is in the background");
    }
}
