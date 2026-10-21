//! Native Color panel presentations. These edit the existing tool-colour command; only the
//! selected presentation belongs to UI state. Geometry is bounded and uses logical points.

use egui::{Color32, Mesh, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

use super::{color_readout, field_chips, hsva_srgb, srgb_bytes, srgb_hsva};
use crate::PrismApp;
use crate::state::ColorPanelMode;
use crate::theme::Tokens;

const MODES: [(ColorPanelMode, &str); 4] = [
    (ColorPanelMode::HueCube, "Hue Cube"),
    (ColorPanelMode::BrightnessCube, "Brightness Cube"),
    (ColorPanelMode::ColorWheel, "Color Wheel"),
    (ColorPanelMode::HsbSliders, "HSB Sliders"),
];

pub(crate) fn panel_menu(app: &mut PrismApp, ui: &mut egui::Ui) {
    // Clip Studio keeps its sliders in the Color Slider panel, so the Color panel offers no HSB Sliders.
    let csp = crate::csp_tools::active(app);
    for (mode, label) in MODES {
        if csp && mode == ColorPanelMode::HsbSliders {
            continue;
        }
        let selected = app.ui.color_panel.mode == mode;
        let label = crate::i18n::t(label);
        let response = ui.selectable_label(selected, format!("{} {label}", if selected { "✓" } else { "  " }));
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, label));
        if response.clicked() {
            app.ui.color_panel.mode = mode;
            // Give the wheel room when first switching from a compact cube.
            // User-resized taller groups stay tall; compact windows retain the dock's scrolling.
            if mode == ColorPanelMode::ColorWheel {
                let group = crate::dock::Group::Color;
                let width = crate::dock::last_rects(ui.ctx()).into_iter().find(|(g, _)| *g == group).map_or(290.0, |(_, rect)| rect.width());
                app.ui.dock.heights.insert(group, app.ui.dock.height(group).max((width.min(420.0) + 150.0).max(WHEEL_GROUP_MIN)));
            }
            ui.close();
        }
    }
}

pub(super) fn color(app: &PrismApp) -> [f32; 4] {
    if app.ui.color_panel.background { app.session.tools.background } else { app.session.tools.foreground }
}

fn hsv_key(background: bool) -> egui::Id {
    egui::Id::new(("color-panel-hsva", background))
}

pub(super) fn current(app: &PrismApp, ui: &egui::Ui) -> egui::ecolor::Hsva {
    let background = app.ui.color_panel.background;
    let rgb = color(app);
    let stored: Option<egui::ecolor::Hsva> = ui.data(|d| d.get_temp(hsv_key(background)));
    if let Some(hsv) = stored.filter(|h| srgb_bytes(hsva_srgb(*h)) == srgb_bytes(rgb)) {
        return hsv;
    }
    let mut hsv = srgb_hsva(rgb);
    if hsv.s <= f32::EPSILON || hsv.v <= f32::EPSILON {
        hsv.h = ui.data(|d| d.get_temp(egui::Id::new(("color-field-hue", background)))).unwrap_or(hsv.h);
    }
    hsv
}

fn remember(ui: &egui::Ui, background: bool, hsv: egui::ecolor::Hsva) {
    ui.data_mut(|d| {
        d.insert_temp(hsv_key(background), hsv);
        d.insert_temp(egui::Id::new(("color-field-hue", background)), hsv.h);
    });
}

fn display(hsv: egui::ecolor::Hsva) -> Color32 {
    let [r, g, b] = srgb_bytes(hsva_srgb(hsv));
    Color32::from_rgb(r, g, b)
}

pub(super) fn apply(app: &mut PrismApp, ui: &egui::Ui, hsv: egui::ecolor::Hsva) {
    let background = app.ui.color_panel.background;
    let rgb = hsva_srgb(hsv);
    let hex = crate::color_picker_ui::hex([rgb[0], rgb[1], rgb[2]]);
    let params = if background { json!({"background":hex}) } else { json!({"foreground":hex}) };
    if app.run("tools.setColors", params).is_ok() {
        remember(ui, background, hsv);
        if !background {
            crate::type_tool::foreground_changed(app);
        }
    }
}

pub(crate) fn show_mode(app: &mut PrismApp, ui: &mut egui::Ui, mode: ColorPanelMode) {
    match mode {
        ColorPanelMode::HueCube | ColorPanelMode::BrightnessCube => {
            cube(app, ui, mode);
            return;
        }
        // Clip Studio's Color Wheel panel: the wheel and the chips; the sliders are the separate
        // Color Slider panel (`color_slider_ui`).
        ColorPanelMode::ColorWheel | ColorPanelMode::HsbSliders if crate::csp_tools::active(app) => {
            csp_wheel(app, ui);
            return;
        }
        ColorPanelMode::ColorWheel => {
            slider_header(app, ui);
            wheel(app, ui);
            return;
        }
        ColorPanelMode::HsbSliders => slider_header(app, ui),
    }
    ui.add_space(6.0);
    let mut rgb = color(app);
    let background = app.ui.color_panel.background;
    if color_readout(ui, ui.id().with(("color-field", background)), &mut rgb) {
        let mut hsv = srgb_hsva(rgb);
        if hsv.s <= f32::EPSILON || hsv.v <= f32::EPSILON {
            hsv.h = current(app, ui).h;
        }
        apply(app, ui, hsv);
    }
}

fn slider_header(app: &mut PrismApp, ui: &mut egui::Ui) {
    let width = ui.available_width();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (chips, _) = ui.allocate_exact_size(vec2(38.0, 78.0), Sense::hover());
        field_chips(app, ui, chips);
        ui.vertical(|ui| {
            ui.set_width((width - 46.0).max(140.0));
            ui.spacing_mut().item_spacing.y = 2.0;
            sliders(app, ui);
        });
    });
}

pub(super) fn component(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str, gradient: &[Color32]) -> bool {
    let width = (ui.available_width() - 94.0).max(60.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add_sized(vec2(14.0, 24.0), egui::Label::new(label));
        let slider = ui
            .allocate_ui_with_layout(vec2(width, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                crate::widgets::slider(ui, value, range.clone(), Some(gradient))
            })
            .inner;
        let field = crate::widgets::value_field(ui, value, range, suffix, 68.0);
        slider.changed() || field.changed()
    })
    .inner
}

fn sliders(app: &mut PrismApp, ui: &mut egui::Ui) {
    let mut hsv = current(app, ui);
    let (mut h, mut s, mut v) = (hsv.h * 360.0, hsv.s * 100.0, hsv.v * 100.0);
    let hue = crate::widgets::hue_stops();
    let sat = [egui::ecolor::Hsva::new(hsv.h, 0.0, hsv.v.max(0.2), 1.0), egui::ecolor::Hsva::new(hsv.h, 1.0, hsv.v.max(0.2), 1.0)].map(display);
    let val = [Color32::BLACK, display(egui::ecolor::Hsva::new(hsv.h, hsv.s, 1.0, 1.0))];
    let mut changed = component(ui, "H", &mut h, 0.0..=360.0, "°", &hue);
    changed |= component(ui, "S", &mut s, 0.0..=100.0, "%", &sat);
    changed |= component(ui, "B", &mut v, 0.0..=100.0, "%", &val);
    if changed {
        hsv.h = (h / 360.0).rem_euclid(1.0);
        hsv.s = s / 100.0;
        hsv.v = v / 100.0;
        apply(app, ui, hsv);
    }
}

fn grid(rect: Rect, color: impl Fn(f32, f32) -> Color32) -> Mesh {
    const N: u32 = 36;
    let mut mesh = Mesh::default();
    for j in 0..=N {
        for i in 0..=N {
            let (x, y) = (i as f32 / N as f32, j as f32 / N as f32);
            mesh.colored_vertex(pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height()), color(x, y));
        }
    }
    for j in 0..N {
        for i in 0..N {
            let a = j * (N + 1) + i;
            mesh.add_triangle(a, a + 1, a + N + 2);
            mesh.add_triangle(a, a + N + 2, a + N + 1);
        }
    }
    mesh
}

fn marker(ui: &egui::Ui, p: Pos2, radius: f32) {
    ui.painter().circle_stroke(p, radius, Stroke::new(1.5, Color32::WHITE));
    ui.painter().circle_stroke(p, radius + 1.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
}

fn cube(app: &mut PrismApp, ui: &mut egui::Ui, mode: ColorPanelMode) {
    let t = Tokens::get(ui.ctx());
    let width = ui.available_width().max(80.0);
    // ScrollArea content has an unbounded height. Its clip rect is the actual panel viewport,
    // so both cubes follow the dock splitter instead of a fixed height or square aspect ratio.
    // Leave room for markers at the field edges and the strip's arrow on the right.
    let field_width = (width - 76.0).max(12.0);
    let height = (ui.clip_rect().bottom() - ui.cursor().top() - 12.0).max(1.0);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (chips, _) = ui.allocate_exact_size(vec2(38.0, height), Sense::hover());
        field_chips(app, ui, chips);
        let mut hsv = current(app, ui);
        let bright = mode == ColorPanelMode::BrightnessCube;
        let (field, response) = ui.allocate_exact_size(vec2(field_width, height), Sense::click_and_drag());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, if bright { "Brightness cube field" } else { "Hue cube field" }));
        ui.painter().add(grid(field, |x, y| {
            display(if bright { egui::ecolor::Hsva::new(x, 1.0 - y, hsv.v, 1.0) } else { egui::ecolor::Hsva::new(hsv.h, x, 1.0 - y, 1.0) })
        }));
        ui.painter().rect_stroke(field, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
        let mut changed = false;
        if (response.clicked() || response.dragged())
            && let Some(p) = response.interact_pointer_pos()
        {
            let x = ((p.x - field.left()) / field.width()).clamp(0.0, 1.0);
            let y = ((p.y - field.top()) / field.height()).clamp(0.0, 1.0);
            if bright {
                hsv.h = x.min(0.9999);
                hsv.s = 1.0 - y;
            } else {
                hsv.s = x;
                hsv.v = 1.0 - y;
            }
            changed = true;
        }
        let (x, y) = if bright { (hsv.h, 1.0 - hsv.s) } else { (hsv.s, 1.0 - hsv.v) };
        marker(ui, pos2(field.left() + x * field.width(), field.top() + y * field.height()), 5.0);
        let (strip, response) = ui.allocate_exact_size(vec2(14.0, height), Sense::click_and_drag());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, if bright { "Brightness cube slider" } else { "Hue cube slider" }));
        ui.painter().add(grid(strip, |_, y| {
            display(if bright { egui::ecolor::Hsva::new(hsv.h, hsv.s, 1.0 - y, 1.0) } else { egui::ecolor::Hsva::new(1.0 - y, 1.0, 1.0, 1.0) })
        }));
        if (response.clicked() || response.dragged())
            && let Some(p) = response.interact_pointer_pos()
        {
            let value = 1.0 - ((p.y - strip.top()) / strip.height()).clamp(0.0, 1.0);
            if bright {
                hsv.v = value;
            } else {
                hsv.h = value.min(0.9999);
            }
            changed = true;
        }
        let y = strip.top() + (1.0 - if bright { hsv.v } else { hsv.h }) * strip.height();
        ui.painter().add(egui::Shape::convex_polygon(
            vec![pos2(strip.right() + 1.0, y), pos2(strip.right() + 6.0, y - 4.0), pos2(strip.right() + 6.0, y + 4.0)],
            t.text,
            Stroke::NONE,
        ));
        if changed {
            apply(app, ui, hsv);
        }
    });
}

/// Ring hue proceeds counter-clockwise from red at the right, like the reference colour wheel.
fn direction(hue: f32) -> egui::Vec2 {
    let angle = -hue * std::f32::consts::TAU;
    vec2(angle.cos(), angle.sin())
}

fn triangle(rect: Rect) -> [Pos2; 3] {
    let radius = rect.width().min(rect.height()) * 0.365;
    let center = rect.center();
    [center + vec2(radius, 0.0), center + vec2(-radius * 0.5, -radius * 0.866_025_4), center + vec2(-radius * 0.5, radius * 0.866_025_4)]
}

fn triangle_point(rect: Rect, saturation: f32, brightness: f32) -> Pos2 {
    let [h, w, b] = triangle(rect);
    b + (h - b) * (brightness * saturation) + (w - b) * (brightness * (1.0 - saturation))
}

fn weights(vertices: [Pos2; 3], p: Pos2) -> [f32; 3] {
    let [a, b, c] = vertices;
    let cross = |a: egui::Vec2, b: egui::Vec2| a.x * b.y - a.y * b.x;
    let determinant = cross(a - c, b - c);
    if !determinant.is_finite() || determinant.abs() < f32::EPSILON {
        return [0.0, 0.0, 1.0];
    }
    let h = cross(p - c, b - c) / determinant;
    let w = cross(a - c, p - c) / determinant;
    [h, w, 1.0 - h - w]
}

fn triangle_sv(rect: Rect, p: Pos2) -> [f32; 2] {
    let vertices = triangle(rect);
    let mut coordinates = weights(vertices, p);
    if coordinates.iter().any(|v| *v < 0.0) {
        // Outside drags project onto the nearest edge instead of jumping to another component.
        let [a, b, c] = vertices;
        let mut closest = a;
        let mut distance = f32::INFINITY;
        for (from, to) in [(a, b), (b, c), (c, a)] {
            let edge = to - from;
            let t = ((p - from).dot(edge) / edge.length_sq().max(f32::EPSILON)).clamp(0.0, 1.0);
            let point = from + edge * t;
            let d = (point - p).length_sq();
            if d < distance {
                closest = point;
                distance = d;
            }
        }
        coordinates = weights(vertices, closest);
    }
    let [h, w, _] = coordinates.map(|v| v.clamp(0.0, 1.0));
    let brightness = (h + w).clamp(0.0, 1.0);
    [if brightness > f32::EPSILON { (h / brightness).clamp(0.0, 1.0) } else { 0.0 }, brightness]
}

#[derive(Clone, Copy)]
enum WheelPart {
    Ring,
    Triangle,
}

fn wheel_part(rect: Rect, p: Pos2) -> Option<WheelPart> {
    let radius = rect.width() * 0.5;
    let distance = (p - rect.center()).length();
    if (radius * 0.80..=radius).contains(&distance) {
        Some(WheelPart::Ring)
    } else if weights(triangle(rect), p).iter().all(|v| *v >= -0.0001) {
        Some(WheelPart::Triangle)
    } else {
        None
    }
}

/// The smallest wheel (points, margin included): like Photoshop's Color Wheel, a short panel
/// scrolls rather than shrinking the wheel below a usable size. Only a narrower panel is smaller.
pub(crate) const MIN_WHEEL: f32 = 220.0;
/// The Color group height (tab strip, HSB header and a [`MIN_WHEEL`] wheel) the dock keeps while
/// the wheel shows, so the splitter stops where Photoshop's does instead of squashing the wheel.
pub(crate) const WHEEL_GROUP_MIN: f32 = MIN_WHEEL + 150.0;

/// The least height the dock gives a group whose active tab is the Color panel, when the
/// current mode needs more than the group's own minimum (Photoshop's Color Wheel).
pub(crate) fn group_min_height(app: &PrismApp) -> Option<f32> {
    (app.ui.color_panel.mode == ColorPanelMode::ColorWheel && !crate::csp_tools::active(app)).then_some(WHEEL_GROUP_MIN)
}

/// Space kept under a Clip Studio wheel for the colour chips (points).
const CSP_CHIPS: f32 = 44.0;
/// The smallest Clip Studio wheel (points).
const CSP_MIN_WHEEL: f32 = 80.0;

/// Clip Studio's Color Wheel: the wheel as large as the panel allows and centred in it both ways
/// (a narrow panel leaves equal room above and below, a wide one at the sides), the foreground
/// and background chips at the bottom left.
fn csp_wheel(app: &mut PrismApp, ui: &mut egui::Ui) {
    let top = ui.cursor().top();
    let bottom = ui.clip_rect().bottom().min(ui.max_rect().bottom()).max(top + CSP_MIN_WHEEL + CSP_CHIPS);
    let area = Rect::from_min_max(pos2(ui.max_rect().left(), top), pos2(ui.max_rect().right(), bottom));
    ui.allocate_rect(area, Sense::hover());
    let room = Rect::from_min_max(area.min + vec2(0.0, 4.0), pos2(area.right(), area.bottom() - CSP_CHIPS));
    let size = room.width().min(room.height()).max(CSP_MIN_WHEEL);
    wheel_in(app, ui, Rect::from_center_size(room.center(), vec2(room.width(), size)), size);
    let chips = Rect::from_min_size(pos2(area.left() + 4.0, area.bottom() - CSP_CHIPS + 2.0), vec2(38.0, 38.0));
    field_chips(app, ui, chips);
}

fn wheel(app: &mut PrismApp, ui: &mut egui::Ui) {
    // Use the space below the HSB header, but never less than the minimum wheel.
    let room = (ui.clip_rect().bottom() - ui.cursor().top() - 6.0).max(MIN_WHEEL);
    let size = ui.available_width().min(room).clamp(80.0, 420.0);
    let (outer, _) = ui.allocate_exact_size(vec2(ui.available_width(), size), Sense::hover());
    wheel_in(app, ui, outer, size);
}

/// The wheel, `size` across, centred in `outer` (which takes its pointer input).
fn wheel_in(app: &mut PrismApp, ui: &mut egui::Ui, outer: Rect, size: f32) {
    let response = ui.interact(outer, ui.id().with("color-wheel"), Sense::click_and_drag());
    let rect = Rect::from_center_size(outer.center(), vec2(size - 14.0, size - 14.0));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Color wheel"));
    let mut hsv = current(app, ui);
    let radius = rect.width() * 0.5;
    let mut ring = Mesh::default();
    const N: u32 = 180;
    for k in 0..=N {
        let h = k as f32 / N as f32;
        let d = direction(h);
        let color = display(egui::ecolor::Hsva::new(h, 1.0, 1.0, 1.0));
        ring.colored_vertex(rect.center() + d * radius, color);
        ring.colored_vertex(rect.center() + d * radius * 0.80, color);
    }
    for k in 0..N {
        let a = k * 2;
        ring.add_triangle(a, a + 2, a + 3);
        ring.add_triangle(a, a + 3, a + 1);
    }
    ui.painter().add(ring);
    let [h, w, b] = triangle(rect);
    let mut mesh = Mesh::default();
    mesh.colored_vertex(h, display(egui::ecolor::Hsva::new(hsv.h, 1.0, 1.0, 1.0)));
    mesh.colored_vertex(w, Color32::WHITE);
    mesh.colored_vertex(b, Color32::BLACK);
    mesh.add_triangle(0, 1, 2);
    ui.painter().add(mesh);
    let drag_key = response.id.with("wheel-part");
    if response.drag_started() {
        let origin = ui.ctx().input(|i| i.pointer.press_origin());
        ui.data_mut(|d| {
            d.remove::<WheelPart>(drag_key);
            if let Some(part) = origin.and_then(|p| wheel_part(rect, p)) {
                d.insert_temp(drag_key, part);
            }
        });
    }
    let part = if response.dragged() {
        ui.data(|d| d.get_temp(drag_key))
    } else if response.clicked() {
        response.interact_pointer_pos().and_then(|p| wheel_part(rect, p))
    } else {
        None
    };
    if let Some(part) = part
        && let Some(p) = response.interact_pointer_pos()
    {
        match part {
            WheelPart::Ring => {
                let d = p - rect.center();
                if d.length_sq() > f32::EPSILON {
                    hsv.h = (-d.y.atan2(d.x) / std::f32::consts::TAU).rem_euclid(1.0);
                }
            }
            WheelPart::Triangle => {
                let [s, v] = triangle_sv(rect, p);
                hsv.s = s;
                hsv.v = v;
            }
        }
        apply(app, ui, hsv);
    }
    if response.drag_stopped() {
        ui.data_mut(|d| d.remove::<WheelPart>(drag_key));
    }
    marker(ui, rect.center() + direction(hsv.h) * radius * 0.90, 5.0);
    marker(ui, triangle_point(rect, hsv.s, hsv.v), 5.0);
}

#[cfg(test)]
#[path = "color_panel_ui/tests.rs"]
mod tests;
