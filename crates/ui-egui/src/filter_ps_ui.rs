//! Photoshop's classic filter dialog (Filter › Noise › Add Noise…, Median…, and the rest of the
//! one-panel filters): a preview of the layer at real pixels in the top left, with − / zoom / +
//! under it and drag to pan; OK, Cancel and Preview in the right-hand column; the options under
//! the preview, each number as "Label: [value] unit" over a slider, few-choice options as radio
//! buttons where Photoshop has them. Per-filter differences (no preview, no slider, an angle dial,
//! grouped options…) live in [`crate::filter_ps_layout`].
//!
//! The fields are the command's params (plus private `__` keys: `__zoom`, `__centerX`,
//! `__centerY` for the preview), so `ui.dialog.set` / `ui.dialog.confirm` drive it like any
//! dialog. The in-dialog preview runs the engine's own pixel function
//! ([`prism_engine::filters::apply_filter_to_surface`]) on the visible area only, so it matches
//! the result; the on-canvas preview is the usual proxy preview.

use egui::{Color32, Rect as ERect, Sense, Stroke, StrokeKind, vec2};
use prism_color::PixelFormat;
use prism_geom::Rect;
use prism_raster::Surface;
use serde_json::{Map, Value, json};

use crate::PrismApp;
use crate::filter_dialog::{Kind, Param, parse_spec};
use crate::filter_ps_layout::{self as layout, Radio};
use crate::theme::Tokens;
use crate::widgets;

/// Preview box, in points (Photoshop's is about this size).
const PREVIEW: egui::Vec2 = vec2(236.0, 150.0);
const BUTTON_W: f32 = 84.0;
const GAP: f32 = 16.0;
/// The dialog's content width: the preview column, the gap and the button column.
pub const WIDTH: f32 = PREVIEW.x + GAP + BUTTON_W;
const FIELD_W: f32 = 64.0;
/// Zoom steps of the preview, as Photoshop's − / + buttons step them (1 = 100 %).
const ZOOMS: [f32; 10] = [0.0625, 0.125, 0.25, 0.333, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0];
/// The preview never filters more than this many source pixels for one frame.
const MAX_PREVIEW_PIXELS: i64 = 1_500_000;

/// Is this dialog drawn as Photoshop's classic filter dialog? Filter commands whose params are
/// all numbers, choices and checkboxes; plug-ins (their own spec) and filters with grids, files
/// or documents keep the generic form.
pub fn owns(f: &Map<String, Value>) -> bool {
    let Some(cmd) = f.get("__command").and_then(Value::as_str) else { return false };
    if !f.contains_key("__filter") || f.contains_key("__spec") || !cmd.starts_with("filter.") {
        return false;
    }
    let Some(spec) = prism_engine::commands::find(cmd) else { return false };
    let params = parse_spec(spec.params);
    // Colours (`json`) come from the swatches and aren't shown; grids, paths and documents need the
    // generic form.
    !params.is_empty() && params.iter().all(|p| matches!(p.kind, Kind::Range { .. } | Kind::Choice(_) | Kind::Bool(_) | Kind::Int { .. } | Kind::Json))
}

fn enabled(f: &Map<String, Value>, cmd: &str, key: &str) -> bool {
    layout::enabled_when(cmd, key).is_none_or(|(other, value)| f.get(other).and_then(Value::as_str) == Some(value))
}

/// The dialog body with its OK / Cancel / Preview column. `Some(true)` for OK, `Some(false)` for
/// Cancel.
pub fn body(app: &mut PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<bool> {
    let cmd = f.get("__command").and_then(Value::as_str).unwrap_or_default().to_string();
    let params = prism_engine::commands::find(&cmd).map(|c| parse_spec(c.params)).unwrap_or_default();
    let mut outcome = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(PREVIEW.x);
            if layout::has_preview_pane(&cmd) {
                preview(app, ui, f, &cmd);
                ui.add_space(6.0);
            }
            ui.spacing_mut().item_spacing.y = 6.0;
            options(ui, f, &cmd, &params);
            if layout::randomizes(&cmd) {
                ui.add_space(4.0);
                ui.vertical_centered(|ui| {
                    if widgets::secondary_button(ui, tl!("Randomize"), BUTTON_W).clicked() {
                        let next = f.get("seed").and_then(Value::as_u64).unwrap_or(0).wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407) >> 33;
                        f.insert("seed".into(), json!(next as u32));
                    }
                });
            }
        });
        ui.add_space(GAP);
        ui.vertical(|ui| {
            ui.set_width(BUTTON_W);
            ui.spacing_mut().item_spacing.y = 8.0;
            if widgets::primary_button(ui, tl!("OK"), BUTTON_W).clicked() {
                outcome = Some(true);
            }
            // Photoshop: holding Alt turns Cancel into Reset, which puts the defaults back.
            let reset = ui.input(|i| i.modifiers.alt);
            if widgets::secondary_button(ui, if reset { tl!("Reset") } else { tl!("Cancel") }, BUTTON_W).clicked() {
                if reset {
                    reset_params(f, &params);
                } else {
                    outcome = Some(false);
                }
            }
            if layout::has_preview_toggle(&cmd) {
                ui.add_space(6.0);
                let mut on = f.get("__preview").and_then(Value::as_bool).unwrap_or(true);
                if widgets::checkbox(ui, &mut on, tl!("Preview")).changed() {
                    f.insert("__preview".into(), json!(on));
                }
            }
            if let Some((kx, ky)) = layout::center_box(&cmd) {
                ui.add_space(10.0);
                center_box(ui, f, kx, ky);
            }
            if layout::diagram(&cmd) {
                ui.add_space(PREVIEW.y - 40.0);
                diagram(ui, &cmd, &crate::filter_dialog::params_of(f));
            }
        });
    });
    outcome
}

/// The options in order: grouped ones inside their group's frame, joined ones on the row of the
/// option before them.
fn options(ui: &mut egui::Ui, f: &mut Map<String, Value>, cmd: &str, params: &[Param]) {
    let visible: Vec<&Param> = params.iter().filter(|p| p.kind != Kind::Json && !layout::hidden(cmd, &p.key)).collect();
    let mut i = 0;
    while i < visible.len() {
        let p = visible[i];
        if let Some(g) = layout::group_of(cmd, &p.key) {
            let members: Vec<&Param> = visible.iter().copied().filter(|q| g.keys.contains(&q.key.as_str())).collect();
            let header = g.header.strip_prefix('@');
            let mut on = header.is_none_or(|k| f.get(k).and_then(Value::as_bool).unwrap_or(true));
            let t = Tokens::get(ui.ctx());
            match header {
                Some(k) => {
                    if widgets::checkbox(ui, &mut on, &layout::label(cmd, k)).changed() {
                        f.insert(k.into(), json!(on));
                    }
                }
                None => {
                    ui.label(egui::RichText::new(layout::label(cmd, g.header)).color(t.text_dim));
                }
            }
            egui::Frame::new().stroke(Stroke::new(1.0, t.field_border)).corner_radius(t.radius_sm).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.add_enabled_ui(on, |ui| {
                    for q in &members {
                        if header != Some(q.key.as_str()) {
                            let en = enabled(f, cmd, &q.key);
                            ui.add_enabled_ui(en, |ui| option(ui, f, cmd, q, false));
                        }
                    }
                });
            });
            // Skip the rest of the group (and its header checkbox if listed later).
            i += 1;
            while i < visible.len() && (g.keys.contains(&visible[i].key.as_str()) || header == Some(visible[i].key.as_str())) {
                i += 1;
            }
            continue;
        }
        if let Some(text) = layout::heading(cmd, &p.key) {
            ui.label(egui::RichText::new(text).color(Tokens::get(ui.ctx()).text_dim));
        }
        let joined = visible.get(i + 1).filter(|q| layout::joins_previous(cmd, &q.key)).copied();
        let en = enabled(f, cmd, &p.key);
        match joined {
            Some(q) => {
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(en, |ui| option(ui, f, cmd, p, true));
                    ui.add_space(10.0);
                    let en = enabled(f, cmd, &q.key);
                    ui.add_enabled_ui(en, |ui| option(ui, f, cmd, q, true));
                });
                i += 2;
            }
            None => {
                ui.add_enabled_ui(en, |ui| option(ui, f, cmd, p, false));
                i += 1;
            }
        }
    }
}

/// The command's defaults back in the fields (Alt + Cancel).
fn reset_params(f: &mut Map<String, Value>, params: &[Param]) {
    for p in params {
        let v = match &p.kind {
            Kind::Range { default, .. } => json!(default),
            Kind::Choice(_) => json!(p.initial_choice().unwrap_or_default()),
            Kind::Bool(default) => json!(default),
            Kind::Int { default } => json!(default),
            _ => continue,
        };
        f.insert(p.key.clone(), v);
    }
}

/// One option. `inline`: on a shared row (no slider, radio buttons side by side, no label for a
/// joined choice).
fn option(ui: &mut egui::Ui, f: &mut Map<String, Value>, cmd: &str, p: &Param, inline: bool) {
    let t = Tokens::get(ui.ctx());
    let label = layout::label(cmd, &p.key);
    let caption = |ui: &mut egui::Ui| {
        ui.label(egui::RichText::new(format!("{label}:")).color(t.text_dim));
    };
    match &p.kind {
        Kind::Range { min, max, default } => {
            let (min, max) = (*min, *max);
            let mut v = f.get(&p.key).and_then(Value::as_f64).unwrap_or(f64::from(*default)) as f32;
            let dial = layout::dial(cmd, &p.key);
            let row = |ui: &mut egui::Ui, v: &mut f32| {
                ui.spacing_mut().item_spacing.x = 6.0;
                caption(ui);
                widgets::value_field(ui, v, min..=max, "", FIELD_W);
                let u = layout::unit(cmd, &p.key, max);
                if !u.is_empty() {
                    ui.label(egui::RichText::new(u).color(t.text_dim));
                }
                if dial {
                    angle_dial(ui, v, min, max);
                }
            };
            if inline {
                row(ui, &mut v);
            } else {
                ui.horizontal(|ui| row(ui, &mut v));
            }
            if !inline && !dial && layout::slider(cmd, &p.key) {
                if min > 0.0 && max / min > 500.0 {
                    // Wide ranges: a logarithmic slider, as Photoshop's radius sliders feel.
                    let mut lv = v.clamp(min, max).ln();
                    if widgets::slider(ui, &mut lv, min.ln()..=max.ln(), None).changed() {
                        v = lv.exp();
                    }
                } else {
                    widgets::slider(ui, &mut v, min..=max, None);
                }
            }
            let v = if layout::whole(cmd, &p.key) { v.round() } else { (v * 10.0).round() / 10.0 };
            f.insert(p.key.clone(), json!(v.clamp(min, max)));
        }
        Kind::Choice(options) => {
            let first = options.first().cloned().unwrap_or_default();
            let mut cur = f.get(&p.key).and_then(Value::as_str).map_or(first, str::to_string);
            let mut pick = |ui: &mut egui::Ui| {
                for o in options {
                    if radio(ui, cur == *o, &layout::label(cmd, o)).clicked() {
                        cur.clone_from(o);
                    }
                }
            };
            match layout::radio(cmd, &p.key) {
                Some(Radio::Framed) => group(ui, &label, pick),
                Some(Radio::Listed) => {
                    caption(ui);
                    ui.indent(("psflt-radio", &p.key), |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        pick(ui);
                    });
                }
                Some(Radio::Row) => {
                    let mut row = |ui: &mut egui::Ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        if !layout::joins_previous(cmd, &p.key) {
                            caption(ui);
                        }
                        pick(ui);
                    };
                    if inline {
                        row(ui);
                    } else {
                        ui.horizontal(row);
                    }
                }
                None => {
                    ui.horizontal(|ui| {
                        caption(ui);
                        let labels: Vec<String> = options.iter().map(|o| layout::label(cmd, o)).collect();
                        let opts: Vec<(String, &str)> = options.iter().cloned().zip(labels.iter().map(String::as_str)).collect();
                        widgets::dropdown(ui, &format!("psflt-{cmd}-{}", p.key), &mut cur, &opts, PREVIEW.x - 70.0);
                    });
                }
            }
            f.insert(p.key.clone(), json!(cur));
        }
        Kind::Bool(default) => {
            let mut b = f.get(&p.key).and_then(Value::as_bool).unwrap_or(*default);
            widgets::checkbox(ui, &mut b, &label);
            f.insert(p.key.clone(), json!(b));
        }
        Kind::Int { default } => {
            let mut v = f.get(&p.key).and_then(Value::as_f64).unwrap_or(*default as f64) as f32;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                caption(ui);
                widgets::value_field(ui, &mut v, -30000.0..=30000.0, "", FIELD_W);
                let u = layout::unit(cmd, &p.key, 30000.0);
                if !u.is_empty() {
                    ui.label(egui::RichText::new(u).color(t.text_dim));
                }
            });
            f.insert(p.key.clone(), json!(v.round() as i64));
        }
        _ => {}
    }
}

/// Photoshop's angle wheel: a circle with a hand at the angle (0° to the right, counter-
/// clockwise); press or drag on it to point the hand.
fn angle_dial(ui: &mut egui::Ui, v: &mut f32, min: f32, max: f32) {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click_and_drag());
    let c = rect.center();
    if (resp.clicked() || resp.dragged())
        && let Some(at) = resp.interact_pointer_pos()
    {
        let d = at - c;
        if d.length() > 1.0 {
            *v = (-d.y).atan2(d.x).to_degrees().round().clamp(min, max);
        }
    }
    let p = ui.painter();
    p.circle_stroke(c, 11.0, Stroke::new(1.0, t.text_dim));
    let a = v.to_radians();
    p.line_segment([c, c + vec2(a.cos(), -a.sin()) * 9.0], Stroke::new(1.5, t.text));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, "angle"));
}

/// Photoshop's Blur Center box: rays from the centre over a white square; click or drag to move
/// the centre (stored as fractions of the canvas).
fn center_box(ui: &mut egui::Ui, f: &mut Map<String, Value>, kx: &str, ky: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(crate::i18n::tr_ctx(crate::i18n::current(), "filter", "Blur Center")).color(t.text_dim));
    let (rect, resp) = ui.allocate_exact_size(vec2(DIAGRAM, DIAGRAM), Sense::click_and_drag());
    if (resp.clicked() || resp.dragged())
        && let Some(at) = resp.interact_pointer_pos()
    {
        let u = ((at - rect.min) / rect.size()).clamp(vec2(0.0, 0.0), vec2(1.0, 1.0));
        f.insert(kx.into(), json!(u.x));
        f.insert(ky.into(), json!(u.y));
    }
    let fx = f.get(kx).and_then(Value::as_f64).unwrap_or(0.5) as f32;
    let fy = f.get(ky).and_then(Value::as_f64).unwrap_or(0.5) as f32;
    let c = rect.min + vec2(fx, fy) * rect.size();
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, Color32::WHITE);
    for i in 0..24 {
        let a = i as f32 / 24.0 * std::f32::consts::TAU;
        p.line_segment([c, c + vec2(a.cos(), a.sin()) * DIAGRAM * 1.5], Stroke::new(1.0, Color32::from_gray(150)));
    }
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Blur Center"));
}

/// Side of Photoshop's distortion diagram, in points.
const DIAGRAM: f32 = BUTTON_W;

/// Photoshop's little picture of a distortion: a white square ruled with a grey grid, run through
/// the filter with the current settings.
fn diagram(ui: &mut egui::Ui, cmd: &str, params: &Value) {
    let ppp = ui.ctx().pixels_per_point();
    let side = (DIAGRAM * ppp).round().clamp(16.0, 512.0) as i32;
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (cmd, params.to_string(), side).hash(&mut h);
        h.finish()
    };
    let id = ui.id().with("filter-ps-diagram");
    let mut shown: Option<(u64, egui::TextureHandle)> = ui.data(|d| d.get_temp(id));
    if shown.as_ref().is_none_or(|(k, _)| *k != key) {
        let r = Rect::new(0, 0, side, side);
        let cells = 10;
        let mut grid = Vec::with_capacity((side * side * 4) as usize);
        for y in 0..side {
            for x in 0..side {
                let line = (x * cells / side) != ((x + 1) * cells / side) || (y * cells / side) != ((y + 1) * cells / side);
                let v = if line { 0.45 } else { 1.0 };
                grid.extend_from_slice(&[v, v, v, 1.0]);
            }
        }
        let mut surf = Surface::new(PixelFormat::RGBA8);
        surf.write_region(r, &grid);
        let out = prism_engine::filters::apply_filter_to_surface(cmd, params, &surf, r, None, r).unwrap_or(surf);
        let mut px = vec![[0u8; 4]; (side * side) as usize];
        for y in 0..side {
            out.read_rgba8_into(Rect::new(0, y, side, y + 1), &mut px[(y * side) as usize..((y + 1) * side) as usize]);
        }
        let img = egui::ColorImage::new([side as usize, side as usize], px.iter().map(|p| Color32::from_rgb(p[0], p[1], p[2])).collect());
        let tex = match shown.take() {
            Some((_, mut t)) => {
                t.set(img, egui::TextureOptions::LINEAR);
                t
            }
            None => ui.ctx().load_texture("filter-ps-diagram", img, egui::TextureOptions::LINEAR),
        };
        shown = Some((key, tex));
        ui.data_mut(|d| d.insert_temp(id, shown.clone()));
    }
    let (rect, _) = ui.allocate_exact_size(vec2(DIAGRAM, DIAGRAM), Sense::hover());
    if let Some((_, tex)) = &shown {
        ui.painter().image(tex.id(), rect, ERect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    }
}

/// A titled frame around a group of options (Photoshop's "Distribution" box).
fn group(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(title).color(t.text_dim));
    egui::Frame::new().stroke(Stroke::new(1.0, t.field_border)).corner_radius(t.radius_sm).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 6.0;
        add(ui);
    });
}

/// Photoshop-style radio button.
fn radio(ui: &mut egui::Ui, on: bool, label: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (r, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
        ui.painter().circle_stroke(r.center(), 5.5, Stroke::new(1.2, if on { t.accent } else { t.text_faint }));
        if on {
            ui.painter().circle_filled(r.center(), 3.0, t.accent);
        }
        let l = ui.add(egui::Label::new(egui::RichText::new(label).color(t.text)).sense(Sense::click()));
        let resp = resp.union(l);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, on, label));
        resp
    })
    .inner
}

/// The preview's zoom (1 = 100 %, one layer pixel per screen pixel) and centre (document pixels).
fn view(f: &Map<String, Value>, canvas: Rect, sel: Option<Rect>) -> (f32, [f32; 2]) {
    let zoom = f.get("__zoom").and_then(Value::as_f64).map_or(1.0, |z| z as f32);
    let zoom = if zoom.is_finite() { zoom.clamp(ZOOMS[0], ZOOMS[ZOOMS.len() - 1]) } else { 1.0 };
    // Photoshop centres the preview on the selection, else the canvas.
    let frame = sel.filter(|r| !r.is_empty()).unwrap_or(canvas);
    let def = [(frame.x0 + frame.x1) as f32 / 2.0, (frame.y0 + frame.y1) as f32 / 2.0];
    let c = |k: &str, d: f32| f.get(k).and_then(Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite()).unwrap_or(d);
    (zoom, [c("__centerX", def[0]), c("__centerY", def[1])])
}

/// The texture last shown, and what it showed.
#[derive(Clone)]
struct Shown {
    key: u64,
    tex: egui::TextureHandle,
    /// Where the filtered area sits in the preview box (fractions of the box).
    uv: ERect,
}

fn preview(app: &mut PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, cmd: &str) {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(PREVIEW, Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, t.canvas);
    let ppp = ui.ctx().pixels_per_point();
    let source = app.session.active().map(|st| (st.doc.bounds(), st.doc.selection.as_ref().map(Surface::content_bounds), st.revision));
    let Some((canvas, sel, revision)) = source else { return };
    let (mut zoom, mut center) = view(f, canvas, sel);
    let pick = layout::pick_point(cmd);
    // Screen position of a document point, and back.
    let to_screen = |d: [f32; 2], zoom: f32, center: [f32; 2]| rect.center() + vec2(d[0] - center[0], d[1] - center[1]) * zoom / ppp;
    if let Some((kx, ky)) = pick
        && (resp.clicked() || resp.dragged())
        && let Some(at) = resp.interact_pointer_pos()
    {
        // Lens Flare: click or drag in the preview to place the flare's centre.
        let d = (at - rect.center()) * ppp / zoom;
        let (w, h) = (canvas.width().max(1) as f32, canvas.height().max(1) as f32);
        f.insert(kx.into(), json!((((center[0] + d.x) - canvas.x0 as f32) / w).clamp(0.0, 1.0)));
        f.insert(ky.into(), json!((((center[1] + d.y) - canvas.y0 as f32) / h).clamp(0.0, 1.0)));
    } else if resp.dragged() {
        let d = resp.drag_delta() * ppp / zoom;
        center = [center[0] - d.x, center[1] - d.y];
        f.insert("__centerX".into(), json!(center[0]));
        f.insert("__centerY".into(), json!(center[1]));
    }
    // The visible part of the canvas, in document pixels.
    let (hw, hh) = (PREVIEW.x * ppp / zoom / 2.0, PREVIEW.y * ppp / zoom / 2.0);
    let want = Rect::new((center[0] - hw).floor() as i32, (center[1] - hh).floor() as i32, (center[0] + hw).ceil() as i32, (center[1] + hh).ceil() as i32);
    let vis = want.intersect(&canvas);
    let params = crate::filter_dialog::params_of(f);
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (cmd, params.to_string(), vis.x0, vis.y0, vis.x1, vis.y1, zoom.to_bits(), revision).hash(&mut h);
        h.finish()
    };
    let id = ui.id().with("filter-ps-preview");
    let mut shown: Option<Shown> = ui.data(|d| d.get_temp(id));
    if shown.as_ref().is_none_or(|s| s.key != key)
        && !vis.is_empty()
        && let Some(img) = render(app, cmd, &params, vis, zoom)
    {
        {
            let uv = ERect::from_min_max(
                egui::pos2((vis.x0 - want.x0) as f32 / want.width().max(1) as f32, (vis.y0 - want.y0) as f32 / want.height().max(1) as f32),
                egui::pos2((vis.x1 - want.x0) as f32 / want.width().max(1) as f32, (vis.y1 - want.y0) as f32 / want.height().max(1) as f32),
            );
            let tex = match shown.take() {
                Some(mut s) => {
                    s.tex.set(img, egui::TextureOptions::NEAREST);
                    s.tex
                }
                None => ui.ctx().load_texture("filter-ps-preview", img, egui::TextureOptions::NEAREST),
            };
            shown = Some(Shown { key, tex, uv });
            ui.data_mut(|d| d.insert_temp(id, shown.clone()));
        }
    }
    if let Some(s) = &shown {
        let at = ERect::from_min_max(rect.min + s.uv.min.to_vec2() * rect.size(), rect.min + s.uv.max.to_vec2() * rect.size());
        p.image(s.tex.id(), at, ERect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    }
    if let Some((kx, ky)) = pick {
        let fx = f.get(kx).and_then(Value::as_f64).unwrap_or(0.5) as f32;
        let fy = f.get(ky).and_then(Value::as_f64).unwrap_or(0.5) as f32;
        let at = to_screen([canvas.x0 as f32 + fx * canvas.width() as f32, canvas.y0 as f32 + fy * canvas.height() as f32], zoom, center);
        for s in [Stroke::new(3.0, Color32::BLACK), Stroke::new(1.0, Color32::WHITE)] {
            p.line_segment([at - vec2(6.0, 0.0), at + vec2(6.0, 0.0)], s);
            p.line_segment([at - vec2(0.0, 6.0), at + vec2(0.0, 6.0)], s);
        }
    }
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    // − 100 % +, centred under the preview.
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(PREVIEW.x / 2.0 - 64.0);
        let step = ZOOMS.iter().position(|z| (*z - zoom).abs() < 1e-3).unwrap_or(5);
        if crate::icons::button(ui, "minus", 22.0, false, tl!("Zoom Out")).clicked() && step > 0 {
            zoom = ZOOMS[step - 1];
        }
        ui.add_sized(vec2(72.0, 22.0), egui::Label::new(egui::RichText::new(format!("{}%", fmt_zoom(zoom))).color(t.text)));
        if crate::icons::button(ui, "plus", 22.0, false, tl!("Zoom In")).clicked() && step + 1 < ZOOMS.len() {
            zoom = ZOOMS[step + 1];
        }
        // Zooming out never filters more than the preview budget.
        let fits = |z: f32| i64::from((PREVIEW.x * ppp / z) as i32) * i64::from((PREVIEW.y * ppp / z) as i32) <= MAX_PREVIEW_PIXELS;
        while !fits(zoom) && zoom < 1.0 {
            let i = ZOOMS.iter().position(|z| *z >= zoom).unwrap_or(5);
            zoom = ZOOMS[(i + 1).min(5)];
        }
        f.insert("__zoom".into(), json!(zoom));
    });
}

fn fmt_zoom(z: f32) -> String {
    let pct = z * 100.0;
    if (pct - pct.round()).abs() < 0.05 { format!("{}", pct.round() as i64) } else { format!("{pct:.1}") }
}

/// The filtered layer over `vis`, as screen pixels at `zoom` (nearest neighbour), over the
/// transparency checkerboard.
fn render(app: &PrismApp, cmd: &str, params: &Value, vis: Rect, zoom: f32) -> Option<egui::ColorImage> {
    let (_, src, _) = crate::distort_ui::active_pixels(app).ok()?;
    let st = app.session.active()?;
    let canvas = st.doc.bounds();
    // Filter only what shows: the selection (or everything) inside the visible area.
    let mut mask = Surface::new(PixelFormat::GRAY8);
    match st.doc.selection.as_ref() {
        Some(sel) => mask.write_region(vis, &sel.read_region(vis)),
        None => mask.write_region(vis, &vec![1.0; vis.width() as usize * vis.height() as usize]),
    }
    let sel_bounds = st.doc.selection.as_ref().map(Surface::content_bounds).filter(|b| !b.is_empty());
    let bounds = sel_bounds.unwrap_or(canvas);
    let out = prism_engine::filters::apply_filter_to_surface(cmd, params, &src, bounds, Some(&mask), canvas).unwrap_or(src);
    let (w, h) = (((vis.width() as f32) * zoom).round().max(1.0) as usize, ((vis.height() as f32) * zoom).round().max(1.0) as usize);
    let rows: Vec<i32> = (0..h).map(|y| vis.y0 + ((y as f32 + 0.5) / zoom) as i32).collect();
    let cols: Vec<i32> = (0..w).map(|x| vis.x0 + ((x as f32 + 0.5) / zoom) as i32).collect();
    let mut row = vec![[0u8; 4]; vis.width() as usize];
    let mut px = Vec::with_capacity(w * h);
    let alpha = out.format().alpha;
    for (y, sy) in rows.iter().enumerate() {
        out.read_rgba8_into(Rect::new(vis.x0, *sy, vis.x1, sy + 1), &mut row);
        for (x, sx) in cols.iter().enumerate() {
            let s = row.get((sx - vis.x0).max(0) as usize).copied().unwrap_or([0, 0, 0, 0]);
            let check = if ((x / 8) + (y / 8)) % 2 == 0 { 255.0 } else { 204.0 };
            let a = if alpha { f32::from(s[3]) / 255.0 } else { 1.0 };
            let mix = |c: u8| (f32::from(c) * a + check * (1.0 - a)).round() as u8;
            px.push(Color32::from_rgb(mix(s[0]), mix(s[1]), mix(s[2])));
        }
    }
    Some(egui::ColorImage::new([w, h], px))
}

#[cfg(test)]
#[path = "filter_ps_ui_tests.rs"]
mod tests;
