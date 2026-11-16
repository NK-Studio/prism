//! Photoshop's Color Picker (Foreground / Background Color) dialog: a 2D colour field and a slider
//! for the selected component (H, S, B, R, G or B radio), new/current swatches, and HSB, RGB, Lab,
//! CMYK and hex fields. The colour lives in the dialog fields (`color` as `#rrggbb`, plus the HSB
//! floats so hue survives greys), so `ui.dialog.set` drives it; OK runs `tools.setColors`.
//! While it is the top dialog the image is its eyedropper, as in Photoshop: the pointer over the
//! canvas is a pipette, and a click or drag there samples into the new colour ([`sample_at`]).
//!
//! Every colour swatch in a panel or another dialog (Layer Style, Fill, Preferences…) opens this
//! same dialog through [`swatch`]: the swatch has no app, so it posts a request the dialog host
//! serves ([`serve_swatches`]), and the picked colour flows back to it live; Cancel restores it.

use egui::{Color32, Mesh, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Map, Value, json};

use crate::PrismApp;
use crate::state::DialogKind;
use crate::theme::Tokens;
use crate::widgets;

/// Radio components, Photoshop order: H, S, B, R, G, B.
pub const MODES: &[(&str, &str)] = &[("h", "H:"), ("s", "S:"), ("v", "B:"), ("r", "R:"), ("g", "G:"), ("b", "B:")];

/// The Lab radios beside them (L, a, b): the field shows the other two Lab components.
pub const LAB_MODES: &[(&str, &str)] = &[("lab-l", "L:"), ("lab-a", "a:"), ("lab-b", "b:")];

/// sRGB of a Lab colour, clipped into the sRGB cube (the field shows out-of-gamut Lab clipped,
/// as Photoshop does).
fn lab_rgb(lab: [f32; 3]) -> [f32; 3] {
    prism_color::convert::lab_to_srgb(lab).map(|c| c.clamp(0.0, 1.0))
}

/// Lab a or b from a 0..1 position, and back.
fn ab(t: f32) -> f32 {
    t * 255.0 - 128.0
}
fn ab_t(v: f32) -> f32 {
    ((v + 128.0) / 255.0).clamp(0.0, 1.0)
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = (h.rem_euclid(360.0)) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

pub fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let d = mx - mn;
    let h = if d <= 0.0 {
        0.0
    } else if mx == c[0] {
        60.0 * ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if mx == c[1] {
        60.0 * ((c[2] - c[0]) / d + 2.0)
    } else {
        60.0 * ((c[0] - c[1]) / d + 4.0)
    };
    [h, if mx <= 0.0 { 0.0 } else { d / mx }, mx]
}

pub fn hex(c: [f32; 3]) -> String {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", q(c[0]), q(c[1]), q(c[2]))
}

pub fn parse_hex(s: &str) -> Option<[f32; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    // `get`, not `[..]`: typed text may be multi-byte, and a byte range can split a character.
    let c = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok().map(|v| f32::from(v) / 255.0);
    Some([c(0)?, c(2)?, c(4)?])
}

/// The colour at field position (x, y) in 0..1 (y down) for `mode` with the slider at `z` (0..1),
/// given the current HSV/RGB (used for the components the field doesn't vary).
pub fn field_color(mode: &str, z: f32, x: f32, y: f32) -> [f32; 3] {
    match mode {
        "lab-l" => lab_rgb([z * 100.0, ab(x), ab(1.0 - y)]),
        "lab-a" => lab_rgb([(1.0 - y) * 100.0, ab(z), ab(x)]),
        "lab-b" => lab_rgb([(1.0 - y) * 100.0, ab(x), ab(z)]),
        "s" => hsv_to_rgb(x * 360.0, z, 1.0 - y),
        "v" => hsv_to_rgb(x * 360.0, 1.0 - y, z),
        "r" => [z, 1.0 - y, x],
        "g" => [1.0 - y, z, x],
        "b" => [x, 1.0 - y, z],
        _ => hsv_to_rgb(z * 360.0, x, 1.0 - y),
    }
}

/// Lab components of field position (x, y) with the slider at `z` in a Lab mode.
fn field_lab(mode: &str, z: f32, x: f32, y: f32) -> Option<[f32; 3]> {
    match mode {
        "lab-l" => Some([z * 100.0, ab(x), ab(1.0 - y)]),
        "lab-a" => Some([(1.0 - y) * 100.0, ab(z), ab(x)]),
        "lab-b" => Some([(1.0 - y) * 100.0, ab(x), ab(z)]),
        _ => None,
    }
}

/// Field position and slider value of a colour in `mode` (inverse of [`field_color`]).
pub fn locate(mode: &str, now: &Components) -> (f32, f32, f32) {
    let &Components { rgb, hsv, lab, .. } = now;
    let h = hsv[0] / 360.0;
    let l = (lab[0] / 100.0).clamp(0.0, 1.0);
    match mode {
        "lab-l" => (ab_t(lab[1]), 1.0 - ab_t(lab[2]), l),
        "lab-a" => (ab_t(lab[2]), 1.0 - l, ab_t(lab[1])),
        "lab-b" => (ab_t(lab[1]), 1.0 - l, ab_t(lab[2])),
        "s" => (h, 1.0 - hsv[2], hsv[1]),
        "v" => (h, 1.0 - hsv[1], hsv[2]),
        "r" => (rgb[2], 1.0 - rgb[1], rgb[0]),
        "g" => (rgb[2], 1.0 - rgb[0], rgb[1]),
        "b" => (rgb[0], 1.0 - rgb[1], rgb[2]),
        _ => (hsv[1], 1.0 - hsv[2], h),
    }
}

/// Colour for slider position `z` (0..1) in `mode`, other components from the current colour.
fn slider_color(mode: &str, z: f32, now: &Components) -> [f32; 3] {
    let &Components { rgb, hsv, lab, .. } = now;
    match mode {
        "lab-l" => lab_rgb([z * 100.0, lab[1], lab[2]]),
        "lab-a" => lab_rgb([lab[0], ab(z), lab[2]]),
        "lab-b" => lab_rgb([lab[0], lab[1], ab(z)]),
        "s" => hsv_to_rgb(hsv[0], z, hsv[2]),
        "v" => hsv_to_rgb(hsv[0], hsv[1], z),
        "r" => [z, rgb[1], rgb[2]],
        "g" => [rgb[0], z, rgb[2]],
        "b" => [rgb[0], rgb[1], z],
        _ => hsv_to_rgb(z * 360.0, 1.0, 1.0),
    }
}

fn c32(c: [f32; 3]) -> Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(q(c[0]), q(c[1]), q(c[2]))
}

/// The fields every picker opens with: `rgb` as the new and current colour, under `label`. The
/// component radio is the one last used, as in Photoshop.
fn fields_for(app: &PrismApp, target: &str, label: &str, rgb: [f32; 3]) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert("__colorPicker".into(), json!(target));
    f.insert("__label".into(), json!(label));
    f.insert("color".into(), json!(hex(rgb)));
    f.insert("__orig".into(), json!(hex(rgb)));
    f.insert("__hsv".into(), json!(rgb_to_hsv(rgb)));
    f.insert("__mode".into(), json!(app.ui.color_picker_mode.clone().unwrap_or_else(|| "h".into())));
    f.insert("__webOnly".into(), json!(false));
    f
}

/// Open the picker for the foreground or background colour.
pub fn open(app: &mut PrismApp, target: &str) -> u64 {
    let c = if target == "background" { app.session.tools.background } else { app.session.tools.foreground };
    let label = if target == "background" { "Color Picker (Background Color)" } else { "Color Picker (Foreground Color)" };
    let target = if target == "background" { "background" } else { "foreground" };
    let f = fields_for(app, target, label, [c[0], c[1], c[2]]);
    app.ui.open_dialog(DialogKind::Command, f)
}

/// Open the picker on `rgb` for something other than the tool colours: OK runs `command` with
/// `params` plus `"color": "#rrggbb"` (e.g. a gradient stop's colour).
pub fn open_for_command(app: &mut PrismApp, label: &str, rgb: [f32; 3], command: &str, params: Value) -> u64 {
    let mut f = fields_for(app, "command", label, rgb);
    f.insert("__command".into(), json!(command));
    f.insert("__params".into(), params);
    app.ui.open_dialog(DialogKind::Command, f)
}

/// A swatch's open picker: which dialog edits which swatch, and what the swatch was last given.
#[derive(Clone, Debug, PartialEq)]
pub struct SwatchSession {
    dialog: u64,
    swatch: u64,
    orig: [f32; 3],
    last: String,
    /// The colour OK confirmed; a session whose dialog closed without one was cancelled.
    done: Option<[f32; 3]>,
}

/// A swatch asking for the picker (posted to egui memory: the swatch has no app).
#[derive(Clone)]
struct SwatchRequest {
    swatch: u64,
    rgb: [f32; 3],
    label: String,
    floats: bool,
}

const SWATCH_REQUESTS: &str = "pc-swatch-picker-requests";

fn swatch_pick(swatch: u64) -> egui::Id {
    egui::Id::new(("pc-swatch-pick", swatch))
}

/// Photoshop's Color Picker for a colour swatch drawn anywhere (a panel, Layer Style, Fill…).
/// `response` is the swatch's (its id names it across frames) and `rgb` its colour, sRGB 0..1.
/// A click opens the picker dialog on the next frame; returns the colour the picker set this
/// frame, live while it is open, the original again on Cancel.
pub fn swatch(ui: &egui::Ui, response: &egui::Response, rgb: [f32; 3], label: &str) -> Option<[f32; 3]> {
    if response.clicked() {
        open_swatch(ui.ctx(), response.id, rgb, label);
    }
    take_swatch(ui.ctx(), response.id)
}

/// Ask for the picker for swatch `id` (see [`swatch`]), e.g. from a menu beside the swatch.
pub fn open_swatch(ctx: &egui::Context, id: egui::Id, rgb: [f32; 3], label: &str) {
    let request = SwatchRequest {
        swatch: id.value(),
        rgb: rgb.map(|c| if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 }),
        label: label.to_string(),
        // Drawn in a window of its own (Layer Style's): the picker gets one too, above it.
        floats: ctx.viewport_id() != egui::ViewportId::ROOT,
    };
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<SwatchRequest>>(egui::Id::new(SWATCH_REQUESTS)).push(request));
    // The dialog host runs in the main window's pass.
    ctx.request_repaint();
    ctx.request_repaint_of(egui::ViewportId::ROOT);
}

/// The colour the picker set for swatch `id` since it last asked (see [`swatch`]).
pub fn take_swatch(ctx: &egui::Context, id: egui::Id) -> Option<[f32; 3]> {
    ctx.data_mut(|d| d.remove_temp::<[f32; 3]>(swatch_pick(id.value())))
}

/// Open the pickers swatches asked for, and hand each open one's colour back to its swatch.
/// Called by the dialog host every frame.
pub fn serve_swatches(app: &mut PrismApp, ctx: &egui::Context) {
    let requests = ctx.data_mut(|d| d.remove_temp::<Vec<SwatchRequest>>(egui::Id::new(SWATCH_REQUESTS))).unwrap_or_default();
    for r in requests {
        let open = app.ui.swatch_pickers.iter().any(|s| s.swatch == r.swatch && app.ui.dialogs.iter().any(|d| d.id == s.dialog));
        if open {
            continue;
        }
        let mut f = fields_for(app, "swatch", &r.label, r.rgb);
        f.insert("__swatch".into(), json!(r.swatch));
        if r.floats {
            f.insert("__floats".into(), json!(true));
        }
        let dialog = app.ui.open_dialog(DialogKind::Command, f);
        app.ui.swatch_pickers.push(SwatchSession { dialog, swatch: r.swatch, orig: r.rgb, last: hex(r.rgb), done: None });
    }
    let mut kept = Vec::new();
    for mut s in std::mem::take(&mut app.ui.swatch_pickers) {
        let live = app.ui.dialogs.iter().find(|d| d.id == s.dialog).and_then(|d| d.fields.get("color")).and_then(Value::as_str).and_then(parse_hex);
        let want = live.or(s.done).unwrap_or(s.orig);
        if hex(want) != s.last {
            s.last = hex(want);
            // The original goes back exactly, not through `#rrggbb`.
            let rgb = if live.is_none() && s.done.is_none() { s.orig } else { want };
            ctx.data_mut(|d| d.insert_temp(swatch_pick(s.swatch), rgb));
            ctx.request_repaint();
        }
        if live.is_some() {
            kept.push(s);
        }
    }
    app.ui.swatch_pickers = kept;
}

pub fn owns(f: &Map<String, Value>) -> bool {
    f.contains_key("__colorPicker")
}

/// A swatch's picker drawn from a window of its own floats in one too ([`swatch`]).
pub fn floats(f: &Map<String, Value>) -> bool {
    owns(f) && f.get("__floats").and_then(Value::as_bool) == Some(true)
}

/// The Color Picker, when it is the top dialog (another dialog opened over it takes the input).
pub fn top(app: &PrismApp) -> Option<u64> {
    app.ui.dialogs.last().filter(|d| owns(&d.fields)).map(|d| d.id)
}

/// Eyedropper: the top Color Picker's new colour becomes the image's composite colour at document
/// point (x, y). Off the image or over transparency nothing changes.
pub fn sample_at(app: &mut PrismApp, x: f64, y: f64) {
    let Some(id) = top(app) else { return };
    let Some(rgb) = crate::canvas::composite_color(app, x, y) else { return };
    if let Some(d) = app.ui.dialog_mut(id) {
        set_rgb(&mut d.fields, rgb, Keep::Nothing);
    }
}

/// The colour in every model the dialog shows.
pub struct Components {
    pub rgb: [f32; 3],
    pub hsv: [f32; 3],
    pub lab: [f32; 3],
    pub cmyk: [f32; 4],
}

/// Component values an edit stores next to the new `color`. sRGB alone loses them: greys have no
/// hue, and Lab or CMYK values outside sRGB are clamped. Typing "60" into L passes through L = 6
/// first; without the stored Lab the clamped colour would change `a` and `b` under the user.
enum Keep {
    Nothing,
    Hsv([f32; 3]),
    Lab([f32; 3]),
    Cmyk([f32; 4]),
}

/// The stored `key` components, if they still produce `rgb`.
fn kept<const N: usize>(f: &Map<String, Value>, key: &str, rgb: [f32; 3], to_rgb: impl Fn([f32; N]) -> [f32; 3]) -> Option<[f32; N]>
where
    [f32; N]: serde::de::DeserializeOwned,
{
    let v: [f32; N] = f.get(key).and_then(|v| serde_json::from_value(v.clone()).ok())?;
    (hex(to_rgb(v)) == hex(rgb)).then_some(v)
}

/// Current colour, keeping the stored HSB, Lab or CMYK values while they still match `color`.
pub fn current(f: &Map<String, Value>) -> Components {
    let rgb = f.get("color").and_then(Value::as_str).and_then(parse_hex).unwrap_or([0.0; 3]);
    Components {
        rgb,
        hsv: kept(f, "__hsv", rgb, |h| hsv_to_rgb(h[0], h[1], h[2])).unwrap_or_else(|| rgb_to_hsv(rgb)),
        lab: kept(f, "__lab", rgb, prism_color::convert::lab_to_srgb).unwrap_or_else(|| prism_color::convert::srgb_to_lab(rgb)),
        cmyk: kept(f, "__cmyk", rgb, prism_color::convert::cmyk_to_rgb).unwrap_or_else(|| prism_color::convert::rgb_to_cmyk(rgb)),
    }
}

fn set_rgb(f: &mut Map<String, Value>, rgb: [f32; 3], keep: Keep) {
    let web = f.get("__webOnly").and_then(Value::as_bool).unwrap_or(false);
    let (rgb, keep) = if web { (rgb.map(|v| (v * 5.0).round() / 5.0), Keep::Nothing) } else { (rgb, keep) };
    f.insert("color".into(), json!(hex(rgb)));
    for key in ["__hsv", "__lab", "__cmyk"] {
        f.remove(key);
    }
    match keep {
        Keep::Nothing => {}
        Keep::Hsv(h) => {
            f.insert("__hsv".into(), json!(h));
        }
        Keep::Lab(l) => {
            f.insert("__lab".into(), json!(l));
        }
        Keep::Cmyk(k) => {
            f.insert("__cmyk".into(), json!(k));
        }
    }
}

fn grid_mesh(rect: Rect, n: usize, color: impl Fn(f32, f32) -> [f32; 3]) -> Mesh {
    let mut m = Mesh::default();
    for j in 0..=n {
        for i in 0..=n {
            let (x, y) = (i as f32 / n as f32, j as f32 / n as f32);
            m.colored_vertex(pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height()), c32(color(x, y)));
        }
    }
    let w = (n + 1) as u32;
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let a = j * w + i;
            m.add_triangle(a, a + 1, a + w + 1);
            m.add_triangle(a, a + w + 1, a + w);
        }
    }
    m
}

/// Room for both component columns, including units, without overflowing the title's width.
pub(crate) const CONTENT_WIDTH: f32 = 624.0;

/// Draw the picker and its top-right actions. `Some(true)` confirms, `Some(false)` cancels.
pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<bool> {
    ui.set_width(CONTENT_WIDTH);
    let mut outcome = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 20.0;
        ui.vertical(|ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                color_field(ui, f);
            });
            ui.add_space(10.0);
            let mut web = f.get("__webOnly").and_then(Value::as_bool).unwrap_or(false);
            if widgets::checkbox(ui, &mut web, tl!("Only Web Colors")).changed() {
                f.insert("__webOnly".into(), json!(web));
                set_rgb(f, current(f).rgb, Keep::Nothing);
            }
            // Under the colour field, not in the action column: the dialog must still fit a
            // 760 x 480 window.
            ui.add_space(6.0);
            if let Some(rgb) = crate::screen_picker::button(ui, ui.id().with("screen-color")) {
                set_rgb(f, rgb, Keep::Nothing);
            }
        });
        ui.vertical(|ui| {
            ui.horizontal_top(|ui| {
                swatches(ui, f);
                let button_width = ui.available_width().max(168.0);
                ui.vertical(|ui| {
                    if widgets::primary_button(ui, tl!("OK"), button_width).clicked() {
                        outcome = Some(true);
                    }
                    ui.add_space(6.0);
                    if widgets::secondary_button(ui, tl!("Cancel"), button_width).clicked() {
                        outcome = Some(false);
                    }
                    ui.add_space(6.0);
                    // Handled by the dialog host, which has the app (`take_add_swatch`).
                    if widgets::secondary_button(ui, tl!("Add to Swatches"), button_width).clicked() {
                        f.insert("__addSwatch".into(), json!(true));
                    }
                });
            });
            ui.add_space(14.0);
            let mode = f.get("__mode").and_then(Value::as_str).unwrap_or("h").to_string();
            fields(ui, f, &mode, &current(f));
        });
    });
    outcome
}

fn color_field(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let mode = f.get("__mode").and_then(Value::as_str).unwrap_or("h").to_string();
    let now = current(f);
    let (rgb, hsv, lab) = (now.rgb, now.hsv, now.lab);
    let (fx, fy, fz) = locate(&mode, &now);
    // Colour field.
    let (field, resp) = ui.allocate_exact_size(vec2(FIELD, FIELD), Sense::click_and_drag());
    ui.painter().add(grid_mesh(field, 32, |x, y| field_color(&mode, fz, x, y)));
    ui.painter().rect_stroke(field, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    let marker = pos2(field.left() + fx * field.width(), field.top() + fy * field.height());
    // Black on light colours, white on dark, as Photoshop's ring.
    let ring = if prism_color::convert::srgb_to_lab(rgb)[0] > 60.0 { Color32::BLACK } else { Color32::WHITE };
    ui.painter().with_clip_rect(field.expand(6.0)).circle_stroke(marker, 5.5, Stroke::new(1.2, ring));
    if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked()) {
        let (x, y) = (((p.x - field.left()) / field.width()).clamp(0.0, 1.0), ((p.y - field.top()) / field.height()).clamp(0.0, 1.0));
        let c = field_color(&mode, fz, x, y);
        let keep = match mode.as_str() {
            "h" => Keep::Hsv([fz * 360.0, x, 1.0 - y]),
            "s" => Keep::Hsv([x * 360.0, fz, 1.0 - y]),
            "v" => Keep::Hsv([x * 360.0, 1.0 - y, fz]),
            m => field_lab(m, fz, x, y).map_or(Keep::Nothing, Keep::Lab),
        };
        set_rgb(f, c, keep);
    }
    // Component slider (hue runs 360° at the top to 0° at the bottom, like Photoshop).
    let (strip, sresp) = ui.allocate_exact_size(vec2(20.0, FIELD), Sense::click_and_drag());
    ui.painter().add(grid_mesh(strip, 32, |_, y| slider_color(&mode, 1.0 - y, &now)));
    ui.painter().rect_stroke(strip, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    let sy = strip.top() + (1.0 - fz) * strip.height();
    // Photoshop's slider: two white arrowheads with a dark rim pointing in at the value.
    for (x, dir) in [(strip.left() - 1.0, 1.0f32), (strip.right() + 1.0, -1.0)] {
        let tri = vec![pos2(x, sy), pos2(x - 7.0 * dir, sy - 5.0), pos2(x - 7.0 * dir, sy + 5.0)];
        ui.painter().add(egui::Shape::convex_polygon(tri, Color32::WHITE, Stroke::new(1.0, Color32::from_black_alpha(160))));
    }
    if let Some(p) = sresp.interact_pointer_pos().filter(|_| sresp.dragged() || sresp.clicked()) {
        let z = 1.0 - ((p.y - strip.top()) / strip.height()).clamp(0.0, 1.0);
        let h = match mode.as_str() {
            "h" => Some([z * 360.0, hsv[1], hsv[2]]),
            "s" => Some([hsv[0], z, hsv[2]]),
            "v" => Some([hsv[0], hsv[1], z]),
            _ => None,
        };
        let l = match mode.as_str() {
            "lab-l" => Some([z * 100.0, lab[1], lab[2]]),
            "lab-a" => Some([lab[0], ab(z), lab[2]]),
            "lab-b" => Some([lab[0], lab[1], ab(z)]),
            _ => None,
        };
        match (h, l) {
            (Some(h), _) => set_rgb(f, hsv_to_rgb(h[0], h[1], h[2]), Keep::Hsv(h)),
            (_, Some(l)) => set_rgb(f, lab_rgb(l), Keep::Lab(l)),
            _ => set_rgb(f, slider_color(&mode, z, &now), Keep::Nothing),
        }
    }
}

/// The colour field's side, in points.
const FIELD: f32 = 256.0;

fn swatches(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let rgb = current(f).rgb;
    let lang = crate::i18n::current();
    ui.allocate_ui_with_layout(vec2(SWATCH.x, SWATCH.y + 40.0), egui::Layout::top_down(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        // "new" over the new colour, "current" under the colour the picker opened with.
        ui.label(egui::RichText::new(crate::i18n::tr_ctx(lang, "colorPicker", "new")).size(12.0).color(t.text_dim));
        let (sw, _) = ui.allocate_exact_size(SWATCH, Sense::hover());
        let half = SWATCH.y / 2.0;
        let orig = f.get("__orig").and_then(Value::as_str).and_then(parse_hex).unwrap_or(rgb);
        ui.painter().rect_filled(Rect::from_min_size(sw.min, vec2(SWATCH.x, half)), 0.0, c32(rgb));
        let cur = Rect::from_min_size(sw.min + vec2(0.0, half), vec2(SWATCH.x, half));
        ui.painter().rect_filled(cur, 0.0, c32(orig));
        ui.painter().rect_stroke(sw, 0.0, Stroke::new(1.0, t.text_faint), StrokeKind::Outside);
        let click_cur = ui.interact(cur, ui.id().with("cp-current"), Sense::click());
        click_cur.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!("Click to restore the current colour")));
        if click_cur.on_hover_text(tl!("Click to restore the current colour")).clicked() {
            set_rgb(f, orig, Keep::Nothing);
        }
        ui.label(egui::RichText::new(crate::i18n::tr_ctx(lang, "colorPicker", "current")).size(12.0).color(t.text_dim));
    });
    web_warning(ui, f, rgb);
}

/// The new / current swatch pair.
const SWATCH: egui::Vec2 = vec2(64.0, 72.0);

/// The nearest web-safe colour (each channel a multiple of 0x33).
pub fn web_safe(rgb: [f32; 3]) -> [f32; 3] {
    rgb.map(|v| (v.clamp(0.0, 1.0) * 5.0).round() / 5.0)
}

/// Photoshop's "not a web-safe colour" cube beside the swatches, over the nearest web-safe
/// colour; a click on either picks that colour. The column is kept when the colour is safe.
fn web_warning(ui: &mut egui::Ui, f: &mut Map<String, Value>, rgb: [f32; 3]) {
    let (column, _) = ui.allocate_exact_size(vec2(18.0, SWATCH.y + 40.0), Sense::hover());
    let safe = web_safe(rgb);
    if hex(safe) == hex(rgb) {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let mid = column.top() + 20.0 + SWATCH.y / 2.0;
    let cube = Rect::from_center_size(pos2(column.center().x, mid - 9.0), vec2(14.0, 14.0));
    let chip = Rect::from_center_size(pos2(column.center().x, mid + 8.0), vec2(13.0, 13.0));
    let tip = tl!("Not a web-safe color: click to use the nearest web-safe color");
    let r = ui.interact(cube.union(chip), ui.id().with("cp-web-safe"), Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tip));
    let ink = if r.hovered() { t.text } else { t.text_dim };
    paint_cube(ui.painter(), cube, ink);
    ui.painter().rect_filled(chip, 0.0, c32(safe));
    ui.painter().rect_stroke(chip, 0.0, Stroke::new(1.0, t.text_faint), StrokeKind::Outside);
    if r.on_hover_text(tip).clicked() {
        set_rgb(f, safe, Keep::Nothing);
    }
}

/// An isometric cube outline in `rect`.
fn paint_cube(p: &egui::Painter, rect: Rect, ink: Color32) {
    let (c, w, h) = (rect.center(), rect.width() / 2.0, rect.height() / 2.0);
    let top = pos2(c.x, rect.top());
    let (ul, ur) = (pos2(c.x - w, c.y - h * 0.5), pos2(c.x + w, c.y - h * 0.5));
    let (ll, lr) = (pos2(c.x - w, c.y + h * 0.5), pos2(c.x + w, c.y + h * 0.5));
    let (mid, bottom) = (pos2(c.x, c.y), pos2(c.x, rect.bottom()));
    let s = Stroke::new(1.2, ink);
    p.add(egui::Shape::closed_line(vec![top, ur, lr, bottom, ll, ul], s));
    p.line_segment([ul, mid], s);
    p.line_segment([ur, mid], s);
    p.line_segment([mid, bottom], s);
}

/// A component radio: Photoshop's ring, filled while selected. Returns the click.
fn radio(ui: &mut egui::Ui, on: bool, label: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, on, label));
    let ring = if resp.hovered() { t.text } else { t.text_dim };
    ui.painter().circle_filled(r.center(), 7.0, t.field);
    ui.painter().circle_stroke(r.center(), 7.0, Stroke::new(1.0, ring));
    if on {
        ui.painter().circle_filled(r.center(), 3.5, t.text);
    }
    resp.clicked()
}

/// Component radios and numeric fields: HSB, RGB, Lab, CMYK and hex.
fn fields(ui: &mut egui::Ui, f: &mut Map<String, Value>, mode: &str, now: &Components) {
    let t = Tokens::get(ui.ctx());
    let &Components { rgb, hsv, lab, cmyk } = now;
    let mut edit: Option<([f32; 3], Keep)> = None;
    let mut new_mode: Option<&str> = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 22.0;
        ui.vertical(|ui| {
            egui::Grid::new("cp-hsb-rgb").min_row_height(0.0).num_columns(4).spacing([6.0, 4.0]).show(ui, |ui| {
                let vals = [hsv[0], hsv[1] * 100.0, hsv[2] * 100.0, rgb[0] * 255.0, rgb[1] * 255.0, rgb[2] * 255.0];
                let units = ["°", "%", "%", "", "", ""];
                let ranges = [0.0..=360.0, 0.0..=100.0, 0.0..=100.0, 0.0..=255.0, 0.0..=255.0, 0.0..=255.0];
                for i in 0..6 {
                    let (key, label) = MODES[i];
                    if radio(ui, mode == key, label) {
                        new_mode = Some(key);
                    }
                    ui.label(egui::RichText::new(tl!(&label)).color(t.text_dim));
                    let mut v = vals[i].round();
                    if widgets::value_field(ui, &mut v, ranges[i].clone(), "", 54.0).changed() {
                        let (mut h, mut c) = (hsv, rgb);
                        if i < 3 {
                            h[i] = if i == 0 { v } else { v / 100.0 };
                            c = hsv_to_rgb(h[0], h[1], h[2]);
                            edit = Some((c, Keep::Hsv(h)));
                        } else {
                            c[i - 3] = v / 255.0;
                            edit = Some((c, Keep::Nothing));
                        }
                    }
                    ui.label(egui::RichText::new(units[i]).color(t.text_faint));
                    ui.end_row();
                    if i == 2 {
                        ui.allocate_exact_size(vec2(0.0, 6.0), Sense::hover());
                        ui.end_row();
                    }
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                // Under the radios' column, as Photoshop's "#".
                ui.add_sized(vec2(16.0, 18.0), egui::Label::new(egui::RichText::new("#").color(t.text_dim)));
                // While the field has focus it shows what is being typed, not the colour: rebuilding it from
                // the colour every frame would throw away every keystroke until six valid digits were in.
                let id = ui.id().with("cp-hex");
                let typing = if ui.memory(|m| m.has_focus(id)) { ui.data(|d| d.get_temp::<String>(id)) } else { None };
                let mut h = typing.unwrap_or_else(|| hex(rgb).trim_start_matches('#').to_string());
                let r = ui.add(egui::TextEdit::singleline(&mut h).id(id).char_limit(6).desired_width(84.0).font(crate::theme::mono(12.0)));
                if r.has_focus() {
                    ui.data_mut(|d| d.insert_temp(id, h.clone()));
                } else {
                    ui.data_mut(|d| d.remove::<String>(id));
                }
                if r.changed()
                    && let Some(c) = parse_hex(&h)
                {
                    edit = Some((c, Keep::Nothing));
                }
            });
        });
        egui::Grid::new("cp-lab-cmyk").min_row_height(0.0).num_columns(4).spacing([6.0, 4.0]).show(ui, |ui| {
            let ranges = [0.0..=100.0, -128.0..=127.0, -128.0..=127.0];
            for (i, &(key, label)) in LAB_MODES.iter().enumerate() {
                if radio(ui, mode == key, label) {
                    new_mode = Some(key);
                }
                ui.label(egui::RichText::new(tl!(&label)).color(t.text_dim));
                let mut x = lab[i].round();
                if widgets::value_field(ui, &mut x, ranges[i].clone(), "", 54.0).changed() {
                    let mut l = lab;
                    l[i] = x;
                    edit = Some((lab_rgb(l), Keep::Lab(l)));
                }
                ui.label("");
                ui.end_row();
            }
            ui.allocate_exact_size(vec2(0.0, 6.0), Sense::hover());
            ui.end_row();
            // CMYK has no radios in Photoshop: its labels line up with Lab's.
            for (i, label) in ["C:", "M:", "Y:", "K:"].into_iter().enumerate() {
                ui.label("");
                ui.label(egui::RichText::new(tl!(&label)).color(t.text_dim));
                let mut x = (cmyk[i] * 100.0).round();
                if widgets::value_field(ui, &mut x, 0.0..=100.0, "", 54.0).changed() {
                    let mut k = cmyk;
                    k[i] = x / 100.0;
                    edit = Some((prism_color::convert::cmyk_to_rgb(k).map(|c| c.clamp(0.0, 1.0)), Keep::Cmyk(k)));
                }
                ui.label(egui::RichText::new("%").color(t.text_faint));
                ui.end_row();
            }
        });
    });
    if let Some(m) = new_mode {
        f.insert("__mode".into(), json!(m));
    }
    if let Some((c, keep)) = edit {
        set_rgb(f, c, keep);
    }
}

/// After the body drew: Add to Swatches adds the picked colour to the Swatches panel, and the
/// component radio is remembered for the next picker, as in Photoshop.
pub fn after_body(app: &mut PrismApp, f: &mut Map<String, Value>) {
    if f.remove("__addSwatch").and_then(|v| v.as_bool()) == Some(true) {
        crate::swatches_ui::add_from_picker(app, f);
    }
    if let Some(mode) = f.get("__mode").and_then(Value::as_str).filter(|m| MODES.iter().chain(LAB_MODES).any(|(k, _)| k == m)) {
        app.ui.color_picker_mode = Some(mode.to_string());
    }
}

/// OK: set the foreground or background colour.
pub fn confirm(app: &mut PrismApp, f: &Map<String, Value>) -> Result<Value, String> {
    if crate::shape_color_ui::owns(f) {
        return crate::shape_color_ui::confirm(app, f);
    }
    let target = f.get("__colorPicker").and_then(Value::as_str).unwrap_or("foreground");
    let color = f.get("color").and_then(Value::as_str).unwrap_or("#000000");
    if target == "swatch" {
        // The swatch takes it on the next frame ([`serve_swatches`]).
        let rgb = parse_hex(color).ok_or("enter a six-digit hex colour")?;
        let swatch = f.get("__swatch").and_then(Value::as_u64);
        if let Some(s) = app.ui.swatch_pickers.iter_mut().find(|s| Some(s.swatch) == swatch) {
            s.done = Some(rgb);
        }
        return Ok(json!({"color": hex(rgb)}));
    }
    if let Some(cmd) = f.get("__command").and_then(Value::as_str) {
        let mut p = f.get("__params").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
        p["color"] = json!(color);
        return app.run(cmd, p);
    }
    let r = app.run("tools.setColors", json!({ target: color }))?;
    if target == "foreground" {
        crate::type_tool::foreground_changed(app);
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    fn picker(color: &str) -> Harness<'static, Map<String, Value>> {
        let mut f = Map::new();
        f.insert("__colorPicker".into(), json!("foreground"));
        f.insert("color".into(), json!(color));
        f.insert("__orig".into(), json!(color));
        let mut h = Harness::builder().with_size(vec2(700.0, 500.0)).build_ui_state(
            |ui, f: &mut Map<String, Value>| {
                if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    body(ui, f);
                }
            },
            f,
        );
        PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(2);
        h
    }

    /// Type `text` into the focused field one key per frame, like a person typing.
    fn type_keys(h: &mut Harness<'static, Map<String, Value>>, text: &str) {
        for ch in text.chars() {
            h.event(egui::Event::Text(ch.to_string()));
            h.run_steps(1);
        }
    }

    fn color(h: &Harness<'static, Map<String, Value>>) -> String {
        h.state().get("color").and_then(Value::as_str).unwrap().to_string()
    }

    #[test]
    fn hex_field_takes_typed_digits() {
        let mut h = picker("#000000");
        h.get_by_role(Role::TextInput).click();
        h.run_steps(1);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(1);
        type_keys(&mut h, "ff88");
        // Partial input stays in the field instead of snapping back to the colour.
        assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("ff88"));
        assert_eq!(color(&h), "#000000");
        type_keys(&mut h, "00");
        assert_eq!(color(&h), "#ff8800");
        h.key_press(egui::Key::Tab);
        h.run_steps(2);
        assert_eq!(color(&h), "#ff8800");
    }

    #[test]
    fn lab_and_cmyk_fields_take_multi_digit_values() {
        let base = parse_hex("#336699").unwrap();
        let lab = prism_color::convert::srgb_to_lab(base);
        let cmyk = prism_color::convert::rgb_to_cmyk(base);
        // Fields in order: H S B R G B, L a b, C M Y K.
        for (field, typed, want) in [
            (6, "60", prism_color::convert::lab_to_srgb([60.0, lab[1], lab[2]])),
            (7, "30", prism_color::convert::lab_to_srgb([lab[0], 30.0, lab[2]])),
            (9, "40", prism_color::convert::cmyk_to_rgb([0.4, cmyk[1], cmyk[2], cmyk[3]])),
        ] {
            let mut h = picker("#336699");
            h.query_all_by_role(Role::SpinButton).nth(field).unwrap().click();
            h.run_steps(1);
            type_keys(&mut h, typed);
            h.key_press(egui::Key::Tab);
            h.run_steps(2);
            assert_eq!(color(&h), hex(want), "field {field} <- {typed}");
        }
    }

    #[test]
    fn picker_places_actions_above_two_columns_and_web_colors_under_the_field() {
        let h = picker("#336699");
        let inputs: Vec<_> = h.query_all_by_role(Role::SpinButton).map(|n| n.rect()).collect();
        assert_eq!(inputs.len(), 13);
        let (hue, red, lightness, cyan, black) = (inputs[0], inputs[3], inputs[6], inputs[9], inputs[12]);
        assert!(lightness.left() > hue.right(), "Lab is beside HSB, not below RGB");
        assert!((lightness.top() - hue.top()).abs() < 1.0);
        assert!((cyan.top() - red.top()).abs() < 1.0, "CMYK and RGB start at the same height");
        let ok = h.get_by_label("OK").rect();
        let cancel = h.get_by_label("Cancel").rect();
        assert!(ok.left() > hue.left() && ok.bottom() < hue.top(), "actions are above the values and beside the preview: {ok:?}, {hue:?}");
        assert!((ok.left() - cancel.left()).abs() < 1.0 && cancel.top() > ok.bottom());
        assert_eq!(h.query_all_by_label("OK").count(), 1);
        let web = h.get_by_label("Only Web Colors").rect();
        assert!(web.left() < hue.left() && web.top() > hue.bottom());
        // The full dialog should fit even in Prism's minimum 760 x 480 window.
        assert!(black.bottom() - ok.top() < 360.0);
        assert!(black.right() - web.left() < 600.0);
        let hex = h.get_by_role(Role::TextInput).rect();
        assert!(hex.left() < lightness.left() && hex.top() > inputs[5].bottom());
    }

    #[test]
    fn web_colors_and_current_swatch_still_update_the_picker() {
        let mut h = picker("#123456");
        h.get_by_label("Only Web Colors").click();
        h.run_steps(2);
        assert_eq!(color(&h), "#003366");
        h.get_by_label("Only Web Colors").click();
        h.run_steps(2);
        h.get_by_label("Click to restore the current colour").click();
        h.run_steps(2);
        assert_eq!(color(&h), "#123456");
    }

    #[test]
    fn side_buttons_and_keyboard_confirm_or_cancel_without_changing_the_other_color() {
        for target in ["foreground", "background"] {
            for action in ["OK", "Cancel", "Enter", "Escape"] {
                let app = PrismApp::new(prism_engine::Session::new(), Default::default());
                let mut h = Harness::builder().with_size(vec2(760.0, 480.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
                PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
                let original_fg = h.state().session.tools.foreground;
                let original_bg = h.state().session.tools.background;
                let id = open(h.state_mut(), target);
                h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("color".into(), json!("#336699"));
                h.run_steps(4);
                let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(760.0, 480.0));
                for node in h.query_all_by_role(Role::SpinButton) {
                    assert!(screen.contains_rect(node.rect()), "numeric inputs fit the minimum window");
                }
                for label in ["OK", "Cancel", "Only Web Colors"] {
                    assert!(screen.contains_rect(h.get_by_label(label).rect()), "{label} fits the minimum window");
                }
                match action {
                    "Enter" => h.key_press(egui::Key::Enter),
                    "Escape" => h.key_press(egui::Key::Escape),
                    _ => h.get_by_label(action).click(),
                }
                h.run_steps(3);
                assert!(h.state().ui.dialogs.is_empty(), "{target}: {action} closes the picker");
                let confirm = matches!(action, "OK" | "Enter");
                let tools = &h.state().session.tools;
                if confirm && target == "foreground" {
                    assert_eq!(hex([tools.foreground[0], tools.foreground[1], tools.foreground[2]]), "#336699");
                } else {
                    assert_eq!(tools.foreground, original_fg);
                }
                if confirm && target == "background" {
                    assert_eq!(hex([tools.background[0], tools.background[1], tools.background[2]]), "#336699");
                } else {
                    assert_eq!(tools.background, original_bg);
                }
            }
        }
    }

    #[test]
    fn parse_hex_rejects_multibyte_text() {
        // Six bytes, three characters: the byte range 0..2 splits "é", which used to panic.
        assert_eq!(parse_hex("aé€"), None);
        assert_eq!(parse_hex("#ff880"), None);
    }

    #[test]
    fn hsv_round_trips() {
        for c in [[1.0, 0.0, 0.0], [0.2, 0.6, 0.4], [0.5, 0.5, 0.5], [0.0, 0.0, 1.0], [0.9, 0.8, 0.1]] {
            let h = rgb_to_hsv(c);
            let back = hsv_to_rgb(h[0], h[1], h[2]);
            for i in 0..3 {
                assert!((back[i] - c[i]).abs() < 1e-5, "{c:?} -> {h:?} -> {back:?}");
            }
        }
        assert_eq!(hex([1.0, 0.5, 0.0]), "#ff8000");
        assert_eq!(parse_hex("#FF8000").map(hex).as_deref(), Some("#ff8000"));
    }

    #[test]
    fn field_and_locate_are_inverse_in_every_mode() {
        let rgb = [0.8, 0.3, 0.2];
        let mut f = Map::new();
        f.insert("color".into(), json!(hex(rgb)));
        let now = current(&f);
        for (m, _) in MODES.iter().chain(LAB_MODES) {
            let (x, y, z) = locate(m, &now);
            let c = field_color(m, z, x, y);
            assert_eq!(hex(c), hex(rgb), "mode {m}");
        }
    }

    #[test]
    fn hue_survives_grey_and_ok_sets_the_colour() {
        let mut f = Map::new();
        f.insert("color".into(), json!("#808080"));
        f.insert("__hsv".into(), json!([200.0, 0.0, 128.0 / 255.0]));
        assert_eq!(current(&f).hsv[0], 200.0);
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        let id = open(&mut app, "background");
        let d = app.ui.dialog_mut(id).unwrap();
        d.fields.insert("color".into(), json!("#3366cc"));
        let fields = d.fields.clone();
        confirm(&mut app, &fields).unwrap();
        assert_eq!(hex([app.session.tools.background[0], app.session.tools.background[1], app.session.tools.background[2]]), "#3366cc");
    }

    fn dialog_color(app: &PrismApp, id: u64) -> &str {
        app.ui.dialogs.iter().find(|d| d.id == id).and_then(|d| d.fields.get("color")).and_then(Value::as_str).unwrap()
    }

    #[test]
    fn the_top_picker_samples_the_image() {
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 40, "height": 20, "background": "transparent"})).unwrap();
        app.run("shape.create", json!({"kind": "rect", "rect": [0, 0, 20, 20], "fill": "#ff0000"})).unwrap();
        app.run("shape.create", json!({"kind": "rect", "rect": [20, 0, 10, 20], "fill": "#00ff00"})).unwrap();
        let id = open(&mut app, "foreground");
        assert_eq!(top(&app), Some(id));
        sample_at(&mut app, 25.5, 5.5);
        assert_eq!(dialog_color(&app, id), "#00ff00");
        assert_eq!(app.ui.dialog_mut(id).unwrap().fields["__orig"], json!("#000000"), "the current colour stays");
        // Off the image or over transparency nothing changes.
        for (x, y) in [(35.0, 5.0), (-3.0, 5.0), (5.0, 20.0), (1e12, -1e12), (f64::NAN, f64::INFINITY)] {
            sample_at(&mut app, x, y);
            assert_eq!(dialog_color(&app, id), "#00ff00", "({x}, {y})");
        }

        // Agents: `ui.pointer` samples into the picker instead of driving the tool.
        app.ui.tool = crate::state::Tool::Brush;
        let rev = app.session.active().unwrap().revision;
        let ctx = egui::Context::default();
        let events = json!([{"kind": "down", "x": 5, "y": 5}, {"kind": "up", "x": 5, "y": 5}]);
        let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"events": events}));
        let _ = crate::control::handle(&mut app, &ctx, &req);
        assert_eq!(dialog_color(&app, id), "#ff0000");
        assert_eq!(app.session.active().unwrap().revision, rev, "the Brush must not paint");
        assert_eq!(app.session.tools.foreground, [0.0, 0.0, 0.0, 1.0], "only OK sets the foreground");

        // A dialog opened over the picker takes the input.
        let about = app.ui.open_dialog(DialogKind::About, Map::new());
        assert_eq!(top(&app), None);
        sample_at(&mut app, 25.0, 5.0);
        assert_eq!(dialog_color(&app, id), "#ff0000");
        app.ui.close_dialog(about);
        sample_at(&mut app, 25.0, 5.0);
        assert_eq!(dialog_color(&app, id), "#00ff00");
    }
}

#[cfg(test)]
mod screen_integration_tests {
    use super::*;
    use crate::screen_picker::{Capture, Pending};
    use egui_kittest::{Harness, kittest::Queryable};
    #[test]
    fn text_swatch_opens_full_dialog_and_screen_pick_waits_for_ok() {
        let mut session = prism_engine::Session::new();
        session.execute("file.new", json!({"width":160,"height":120})).unwrap();
        let id = session.execute("type.create", json!({"text":"Hello world","color":"#000000","size":24,"x":10,"y":60})).unwrap()["layer"].as_u64().unwrap();
        let services = crate::Services {
            screen_pick: Some(Box::new(|_| {
                let (tx, receiver) = std::sync::mpsc::channel();
                tx.send(Ok(Capture::Color(Some([1.0, 136.0 / 255.0, 0.0])))).unwrap();
                Pending { receiver, cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)) }
            })),
            ..Default::default()
        };
        let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
            PrismApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PrismApp::new(session, services);
            app.ui.tool = crate::state::Tool::Type;
            app
        });
        h.run_steps(8);
        let before = h.state().session.active().unwrap().history.entries().len();
        let fg = h.state().session.tools.foreground;
        h.get_by_label("Set the text color").click();
        h.run_steps(6);
        assert_eq!(h.state().ui.dialogs.last().unwrap().fields["__label"], "Color Picker (Text Color)");
        h.get_by_label("Pick screen color").click();
        h.run_steps(6);
        assert_eq!(h.state().ui.dialogs.last().unwrap().fields["color"], "#ff8800");
        assert_eq!(h.state().session.active().unwrap().history.entries().len(), before, "sampling is only a dialog field change");
        assert_eq!(h.state().session.tools.foreground, fg);
        h.get_by_label("OK").click();
        h.run_steps(4);
        let doc = &h.state().session.active().unwrap().doc;
        let layer = doc.layer(prism_doc::LayerId(id)).unwrap();
        let prism_doc::LayerContent::Text(text) = &layer.content else { panic!("not text") };
        assert_eq!(text.color.to_rgba8(), [255, 136, 0, 255]);
        assert_eq!(h.state().session.active().unwrap().history.entries().len(), before + 1);
    }
}
