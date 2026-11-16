//! Icon rendering. SVGs are embedded (see `icon_data.rs`), recoloured to white, rasterized by the
//! egui_extras SVG loader at the exact on-screen pixel size (crisp at any DPI), and tinted per use.
//!
//! (History: the first shell used Unicode glyphs; half of them rendered as tofu boxes because the
//! bundled fonts lacked them. Vector icons fix that for good.)

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use egui::{Color32, Rect, Response, Sense, Vec2};

use crate::icon_data::ICONS;
use crate::state::Tool;
use crate::theme::Tokens;

fn white_icons() -> &'static HashMap<&'static str, Arc<[u8]>> {
    static MAP: OnceLock<HashMap<&'static str, Arc<[u8]>>> = OnceLock::new();
    MAP.get_or_init(|| {
        ICONS
            .iter()
            .map(|(name, bytes)| {
                let svg = String::from_utf8_lossy(bytes).replace("currentColor", "#ffffff");
                // Lucide's 2 px stroke is heavy at toolbar sizes; the tool glyphs carry their own weights.
                let svg = if name.starts_with("tool-") || name.starts_with("opt-") { svg } else { svg.replace("stroke-width=\"2\"", "stroke-width=\"1.75\"") };
                (*name, Arc::from(svg.into_bytes().into_boxed_slice()))
            })
            .collect()
    })
}

pub fn exists(name: &str) -> bool {
    white_icons().contains_key(name)
}

/// An egui image for an icon, tinted.
pub fn image(name: &str, size: f32, tint: Color32) -> egui::Image<'static> {
    let bytes = white_icons().get(name).or_else(|| white_icons().get("square")).cloned().unwrap_or_default();
    egui::Image::from_bytes(format!("bytes://icons/{name}.svg"), egui::load::Bytes::Shared(bytes)).fit_to_exact_size(Vec2::splat(size)).tint(tint)
}

/// Paint an icon centred in `rect`.
pub fn paint(ui: &egui::Ui, rect: Rect, name: &str, size: f32, tint: Color32) {
    let r = Rect::from_center_size(rect.center(), Vec2::splat(size));
    image(name, size, tint).paint_at(ui, r);
}

/// An icon as the pointer, above every window: `name` with its hotspot `hot` (a fraction of the
/// icon box) on `p`, white with a dark outline so it reads on any image. The caller hides the OS
/// cursor (`CursorIcon::None`).
pub fn cursor(ctx: &egui::Context, name: &str, p: egui::Pos2, hot: Vec2, size: f32) {
    let rect = Rect::from_min_size(p - hot * size, Vec2::splat(size));
    let area = egui::Area::new(egui::Id::new("pc-icon-cursor")).order(egui::Order::Tooltip).fixed_pos(rect.min).constrain(false).interactable(false);
    area.show(ctx, |ui| {
        let outline = image(name, size, Color32::from_black_alpha(200));
        for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0)] {
            outline.paint_at(ui, rect.translate(egui::vec2(dx, dy)));
        }
        image(name, size, Color32::WHITE).paint_at(ui, rect);
    });
}

/// The toolbar glyph of a tool: Prism's own solid artwork (`assets/icons/tools/`), one per tool.
pub fn tool_icon(t: Tool) -> &'static str {
    match t {
        Tool::Move => "tool-move",
        Tool::RectMarquee => "tool-marquee-rect",
        Tool::EllipseMarquee => "tool-marquee-ellipse",
        Tool::Brush => "tool-brush",
        Tool::Pencil => "tool-pencil",
        Tool::MixerBrush => "tool-mixer-brush",
        Tool::Eraser => "tool-eraser",
        Tool::BackgroundEraser => "tool-eraser-background",
        Tool::MagicEraser => "tool-eraser-magic",
        Tool::Eyedropper => "tool-eyedropper",
        Tool::Ruler => "tool-ruler",
        Tool::Note => "tool-note",
        Tool::Count => "tool-count",
        Tool::Lasso => "tool-lasso",
        Tool::PolygonLasso => "tool-lasso-polygon",
        Tool::MagneticLasso => "tool-lasso-magnetic",
        Tool::MagicWand => "tool-magic-wand",
        Tool::Crop => "tool-crop",
        Tool::Slice => "tool-slice",
        Tool::SliceSelect => "tool-slice-select",
        Tool::Gradient => "tool-gradient",
        Tool::PaintBucket => "tool-paint-bucket",
        Tool::Type => "tool-type",
        Tool::VerticalType => "tool-type-vertical",
        Tool::Hand => "tool-hand",
        Tool::RotateView => "tool-rotate-view",
        Tool::Zoom => "tool-zoom",
        Tool::SpotHealing => "tool-spot-healing",
        Tool::Healing => "tool-healing",
        Tool::Patch => "tool-patch",
        Tool::ContentAwareMove => "tool-content-aware-move",
        Tool::RedEye => "tool-red-eye",
        Tool::CloneStamp => "tool-clone-stamp",
        Tool::PatternStamp => "tool-pattern-stamp",
        Tool::HistoryBrush => "tool-history-brush",
        Tool::Blur => "tool-blur",
        Tool::Sharpen => "tool-sharpen",
        Tool::Smudge => "tool-smudge",
        Tool::Dodge => "tool-dodge",
        Tool::Burn => "tool-burn",
        Tool::Sponge => "tool-sponge",
        Tool::QuickSelection => "tool-quick-selection",
        Tool::ObjectSelection => "tool-object-selection",
        Tool::Pen => "tool-pen",
        Tool::PathSelection => "tool-path-selection",
        Tool::DirectSelection => "tool-direct-selection",
        Tool::Rectangle => "tool-rectangle",
        Tool::EllipseShape => "tool-ellipse",
        Tool::Triangle => "tool-triangle",
        Tool::Polygon => "tool-polygon",
        Tool::Line => "tool-line",
        Tool::Arrow => "tool-arrow",
        Tool::CustomShape => "tool-custom-shape",
    }
}

/// Selected and hover fill shared by square icon buttons. Returns the icon tint.
pub fn button_chrome(ui: &egui::Ui, rect: Rect, selected: bool, hovered: bool) -> Color32 {
    let t = Tokens::get(ui.ctx());
    if selected {
        ui.painter().rect_filled(rect, t.radius_sm, t.accent_soft);
        ui.painter().rect_stroke(rect, t.radius_sm, egui::Stroke::new(1.0, t.accent_border), egui::StrokeKind::Inside);
    } else if hovered {
        ui.painter().rect_filled(rect, t.radius_sm, t.hover);
    }
    if selected {
        t.accent_text
    } else if hovered {
        t.text
    } else {
        t.icon
    }
}

/// Square icon button: transparent until hovered; `selected` gets the accent treatment.
pub fn button(ui: &mut egui::Ui, name: &str, box_size: f32, selected: bool, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(box_size), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let tint = if ui.is_enabled() { button_chrome(ui, rect, selected, resp.hovered()) } else { Tokens::get(ui.ctx()).text_faint };
    // The solid tool glyphs fill more of their button than the line icons, as in Photoshop's toolbar.
    // The options-bar glyphs (`opt-*`) are detailed drawings and need the most room.
    let fraction = if name.starts_with("opt-") {
        0.75
    } else if name.starts_with("tool-") {
        0.6
    } else {
        0.52
    };
    paint(ui, rect, name, (box_size * fraction).round(), tint);
    if tooltip.is_empty() { resp } else { resp.on_hover_text(tl!(tooltip)) }
}

/// Rail toggle: "on" gets a quiet filled background and full-strength icon (no accent).
pub fn rail_button(ui: &mut egui::Ui, name: &str, box_size: f32, on: bool, tooltip: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(box_size), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if on {
        ui.painter().rect_filled(rect, t.radius_sm, t.card);
        ui.painter().rect_stroke(rect, t.radius_sm, egui::Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, t.radius_sm, t.hover);
    }
    let tint = if on || resp.hovered() { t.text } else { t.text_faint };
    paint(ui, rect, name, (box_size * 0.52).round(), tint);
    resp.on_hover_text(tl!(tooltip))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_an_icon() {
        for t in Tool::ALL {
            assert!(exists(tool_icon(t)), "{t:?}");
        }
    }

    #[test]
    fn options_bar_glyphs_exist() {
        for name in ["airbrush", "angle", "brush-settings", "pressure-opacity", "pressure-size", "smoothing-options", "symmetry"] {
            assert!(exists(&format!("opt-{name}")), "{name}");
        }
    }

    #[test]
    fn every_tool_has_its_own_glyph() {
        let mut seen = std::collections::HashSet::new();
        for t in Tool::ALL {
            let name = tool_icon(t);
            assert!(name.starts_with("tool-"), "{t:?} uses Prism's toolbar artwork");
            assert!(seen.insert(name), "{t:?} shares {name} with another tool");
        }
    }

    /// Run explicitly for visual QA: every tool glyph at toolbar, flyout and enlarged sizes.
    #[test]
    #[ignore = "writes target/tool-icons/*.png for visual inspection"]
    fn write_tool_icon_sheet() {
        use egui_kittest::Harness;
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tool-icons");
        std::fs::create_dir_all(&out).unwrap();
        for kind in [crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::StudioLight] {
            let mut h = Harness::builder().with_size(egui::vec2(760.0, 560.0)).with_pixels_per_point(2.0).wgpu().build_ui(|ui| {
                let t = Tokens::get(ui.ctx());
                ui.painter().rect_filled(ui.max_rect(), 0.0, t.chrome);
                for (i, tool) in Tool::ALL.into_iter().enumerate() {
                    let cell = Rect::from_min_size(egui::pos2(8.0 + (i % 9) as f32 * 83.0, 8.0 + (i / 9) as f32 * 90.0), egui::vec2(80.0, 86.0));
                    paint(ui, Rect::from_min_size(cell.min, Vec2::splat(30.0)), tool_icon(tool), 16.0, t.icon);
                    paint(ui, Rect::from_min_size(cell.min + egui::vec2(0.0, 30.0), Vec2::splat(30.0)), tool_icon(tool), 14.0, t.icon);
                    paint(ui, Rect::from_min_size(cell.min + egui::vec2(30.0, 4.0), Vec2::splat(48.0)), tool_icon(tool), 48.0, t.text);
                    ui.painter().text(
                        cell.left_bottom() - egui::vec2(0.0, 12.0),
                        egui::Align2::LEFT_TOP,
                        tool_icon(tool).trim_start_matches("tool-"),
                        egui::FontId::proportional(9.0),
                        t.text_dim,
                    );
                }
            });
            crate::PrismApp::setup_context(&h.ctx, kind);
            h.run_steps(5);
            h.render().unwrap().save(out.join(format!("{kind:?}.png"))).unwrap();
        }
    }

    #[test]
    fn icons_are_recoloured() {
        let m = white_icons();
        assert!(m.len() >= 60);
        for (name, b) in m.iter() {
            let s = std::str::from_utf8(b).unwrap();
            assert!(!s.contains("currentColor"), "{name}");
        }
    }
}
