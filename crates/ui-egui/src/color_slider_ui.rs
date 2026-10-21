//! Clip Studio Paint's Color Slider panel: three sliders in the RGB, HLS or CMY model (the
//! vertical tabs on the left pick it), with the foreground and background chips underneath. The
//! Clip Studio workspace docks it beside a Color panel that shows only the wheel. Like the Color
//! panel it edits the tool colours through `tools.setColors`.

use egui::{Color32, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use super::color_panel_ui::{apply, color, component, current};
use super::{srgb_bytes, srgb_hsva};
use crate::PrismApp;
use crate::state::ColorSliderMode;
use crate::theme::Tokens;

const MODELS: [(ColorSliderMode, &str); 3] = [(ColorSliderMode::Rgb, "RGB"), (ColorSliderMode::Hls, "HLS"), (ColorSliderMode::Cmy, "CMY")];

/// Height of one slider row (`color_panel_ui::component`) and the gap between rows.
const ROW: f32 = 24.0;
const GAP: f32 = 2.0;

pub(crate) fn body(app: &mut PrismApp, ui: &mut egui::Ui) {
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        model_tabs(app, ui);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = GAP;
            sliders(app, ui);
        });
    });
    ui.add_space(8.0);
    chips(app, ui);
}

/// The RGB | HLS | CMY tabs, written bottom to top down the panel's left edge.
fn model_tabs(app: &mut PrismApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let (strip, _) = ui.allocate_exact_size(vec2(18.0, ROW * 3.0 + GAP * 2.0), Sense::hover());
    let tab_h = strip.height() / MODELS.len() as f32;
    for (i, (mode, label)) in MODELS.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(strip.left(), strip.top() + tab_h * i as f32), vec2(strip.width(), tab_h));
        let selected = app.ui.color_panel.slider == mode;
        let resp = ui.interact(r, ui.id().with(("color-slider-model", label)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, label));
        if selected {
            ui.painter().rect_filled(r.shrink2(vec2(0.0, 1.0)), t.radius_sm, t.hover);
        } else if resp.hovered() {
            ui.painter().rect_filled(r.shrink2(vec2(0.0, 1.0)), t.radius_sm, t.hover.gamma_multiply(0.5));
        }
        let ink = if selected { t.text } else { t.text_faint };
        let galley = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(10.0), ink);
        // Turned a quarter anticlockwise: the galley's width runs up the tab, its height across.
        let at = pos2(r.center().x - galley.size().y / 2.0, r.center().y + galley.size().x / 2.0);
        ui.painter().add(egui::epaint::TextShape::new(at, galley, ink).with_angle(-std::f32::consts::FRAC_PI_2));
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            app.ui.color_panel.slider = mode;
        }
    }
}

fn swatch(rgb: [f32; 3]) -> Color32 {
    let [r, g, b] = srgb_bytes([rgb[0], rgb[1], rgb[2], 1.0]);
    Color32::from_rgb(r, g, b)
}

/// Each bar shows the colour the slider would give at that point, the other two kept.
fn sliders(app: &mut PrismApp, ui: &mut egui::Ui) {
    let [r, g, b, _] = color(app);
    let rgb = [r, g, b];
    match app.ui.color_panel.slider {
        ColorSliderMode::Rgb | ColorSliderMode::Cmy => {
            let cmy = app.ui.color_panel.slider == ColorSliderMode::Cmy;
            let labels = if cmy { ["C", "M", "Y"] } else { ["R", "G", "B"] };
            // CMY is RGB's complement on the same 0–255 scale.
            let to_value = |c: f32| if cmy { 255.0 - c * 255.0 } else { c * 255.0 };
            let from_value = |v: f32| if cmy { 1.0 - v / 255.0 } else { v / 255.0 };
            let mut next = rgb;
            let mut changed = false;
            for (i, label) in labels.into_iter().enumerate() {
                let ends = [0.0, 255.0].map(|v| {
                    let mut c = rgb;
                    if let Some(slot) = c.get_mut(i) {
                        *slot = from_value(v);
                    }
                    swatch(c)
                });
                let mut v = to_value(rgb.get(i).copied().unwrap_or(0.0)).round();
                if component(ui, label, &mut v, 0.0..=255.0, "", &ends) {
                    if let Some(slot) = next.get_mut(i) {
                        *slot = from_value(v.clamp(0.0, 255.0));
                    }
                    changed = true;
                }
            }
            if changed {
                let mut hsv = srgb_hsva([next[0], next[1], next[2], 1.0]);
                if hsv.s <= f32::EPSILON || hsv.v <= f32::EPSILON {
                    hsv.h = current(app, ui).h;
                }
                apply(app, ui, hsv);
            }
        }
        ColorSliderMode::Hls => {
            let background = app.ui.color_panel.background;
            let [h, l, s] = current_hls(app, ui, rgb);
            let (mut hv, mut lv, mut sv) = ((h * 360.0).round(), (l * 100.0).round(), (s * 100.0).round());
            let hue: Vec<Color32> = (0..=12).map(|k| swatch(hls_to_rgb(k as f32 / 12.0, l, s))).collect();
            let light = [0.0, 0.5, 1.0].map(|x| swatch(hls_to_rgb(h, x, s)));
            let sat = [0.0, 1.0].map(|x| swatch(hls_to_rgb(h, l, x)));
            let mut changed = component(ui, "H", &mut hv, 0.0..=360.0, "", &hue);
            changed |= component(ui, "L", &mut lv, 0.0..=100.0, "", &light);
            changed |= component(ui, "S", &mut sv, 0.0..=100.0, "", &sat);
            if changed {
                let hls = [(hv / 360.0).rem_euclid(1.0), (lv / 100.0).clamp(0.0, 1.0), (sv / 100.0).clamp(0.0, 1.0)];
                let next = hls_to_rgb(hls[0], hls[1], hls[2]);
                let mut hsv = srgb_hsva([next[0], next[1], next[2], 1.0]);
                if hsv.s <= f32::EPSILON || hsv.v <= f32::EPSILON {
                    hsv.h = hls[0];
                }
                apply(app, ui, hsv);
                ui.data_mut(|d| d.insert_temp(hls_key(background), hls));
            }
        }
    }
}

fn hls_key(background: bool) -> egui::Id {
    egui::Id::new(("color-slider-hls", background))
}

/// The colour as hue, lightness and saturation (each 0–1). Grey, black and white keep the hue and
/// saturation last set here, or the Color panel's hue, so dragging L through them loses nothing.
fn current_hls(app: &PrismApp, ui: &egui::Ui, rgb: [f32; 3]) -> [f32; 3] {
    let stored: Option<[f32; 3]> = ui.data(|d| d.get_temp(hls_key(app.ui.color_panel.background)));
    let bytes = srgb_bytes([rgb[0], rgb[1], rgb[2], 1.0]);
    if let Some(hls) = stored.filter(|h| srgb_bytes(with_alpha(hls_to_rgb(h[0], h[1], h[2]))) == bytes) {
        return hls;
    }
    let mut hls = rgb_to_hls(rgb);
    if hls[2] <= f32::EPSILON || hls[1] <= f32::EPSILON || hls[1] >= 1.0 - f32::EPSILON {
        hls[0] = current(app, ui).h;
    }
    hls
}

fn with_alpha(c: [f32; 3]) -> [f32; 4] {
    [c[0], c[1], c[2], 1.0]
}

/// sRGB-encoded RGB (0–1) to hue (0–1), lightness and saturation (HLS, a.k.a. HSL).
pub(crate) fn rgb_to_hls(c: [f32; 3]) -> [f32; 3] {
    let c = c.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let l = (mx + mn) / 2.0;
    let d = mx - mn;
    let s = if d <= 0.0 { 0.0 } else { d / (1.0 - (2.0 * l - 1.0).abs()).max(f32::EPSILON) };
    let h = crate::color_picker_ui::rgb_to_hsv(c)[0] / 360.0;
    [h.rem_euclid(1.0), l, s.clamp(0.0, 1.0)]
}

/// Hue (0–1), lightness and saturation back to sRGB-encoded RGB (0–1), through HSV.
pub(crate) fn hls_to_rgb(h: f32, l: f32, s: f32) -> [f32; 3] {
    let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
    let (h, l, s) = (finite(h).rem_euclid(1.0), finite(l).clamp(0.0, 1.0), finite(s).clamp(0.0, 1.0));
    let v = l + s * l.min(1.0 - l);
    let sv = if v <= 0.0 { 0.0 } else { 2.0 * (1.0 - l / v) };
    crate::color_picker_ui::hsv_to_rgb(h * 360.0, sv.clamp(0.0, 1.0), v)
}

/// Clip Studio's colour chips under the sliders: the foreground (main) colour, wide, and the
/// background (sub) colour. A click picks the one the sliders edit, ringed; a double-click opens
/// the Color Picker on it.
fn chips(app: &mut PrismApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let width = ui.available_width().max(60.0);
    let (row, _) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
    let gap = 6.0;
    let fg_w = ((width - gap) * 0.62).floor();
    let fg = Rect::from_min_size(row.min + vec2(2.0, 2.0), vec2(fg_w - 4.0, row.height() - 4.0));
    let bg = Rect::from_min_max(pos2(fg.right() + gap, fg.top()), pos2(row.right() - 2.0, fg.bottom()));
    let background = app.ui.color_panel.background;
    for (rect, is_bg, label) in [(fg, false, "Foreground Color"), (bg, true, "Background Color")] {
        let [r, g, b, _] = if is_bg { app.session.tools.background } else { app.session.tools.foreground };
        let radius = rect.height() / 2.0;
        ui.painter().rect_filled(rect, radius, swatch([r, g, b]));
        ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
        if is_bg == background {
            ui.painter().rect_stroke(rect.expand(2.0), radius + 2.0, Stroke::new(1.5, t.accent), StrokeKind::Outside);
        }
        let resp = ui.interact(rect, ui.id().with(("color-slider-chip", is_bg)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, is_bg == background, crate::i18n::t(label)));
        if resp.clicked() || resp.double_clicked() {
            app.ui.color_panel.background = is_bg;
        }
        if resp.double_clicked() {
            crate::color_picker_ui::open(app, if is_bg { "background" } else { "foreground" });
        }
    }
}

#[cfg(test)]
#[path = "color_slider_ui_tests.rs"]
mod tests;
