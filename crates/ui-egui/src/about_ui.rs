//! Prism About card, with the studio wordmark rendered by our native type engine.
use std::sync::{Arc, Mutex, PoisonError};

use egui::{Rect, RichText, Sense, vec2};
use prism_doc::{
    TextLayer,
    text::{CharStyle, TextRun},
};

#[cfg(test)]
use crate::PrismApp;
use crate::theme;

pub(crate) fn palette() -> theme::Tokens {
    theme::Tokens::for_kind(theme::ThemeKind::StudioLight)
}

// Studio philosophy is published in Korean and English. Other locales use the existing
// English fallback until translations are available; menu/UI translation coverage is unchanged.
const PHILOSOPHY: [&str; 4] = [
    "Where intuition meets logic, new expression begins.",
    "Art gives imagination form, and programming brings it to life. Making games means connecting these two languages to build a world.",
    "Prism began at this meeting point. Just as light unfolds into many colors through a prism, we hope every creator's ideas unfold into an expression of their own.",
    "A tool for drawing imagination and creating possibilities.",
];

// Named About fonts keep studio typography separate from all editor UI fonts.
// Assets are optional external craft-fonts build inputs, never committed font files.
pub(crate) fn install_fonts(fonts: &mut egui::FontDefinitions) {
    install_fonts_with(fonts, prism_text::CRAFT_FONTS);
}

fn install_fonts_with(fonts: &mut egui::FontDefinitions, faces: &[prism_text::craft_fonts::CraftFont]) {
    for (name, family, style, fallback) in [
        ("about-serif", "MaruBuri", "Regular", egui::FontFamily::Proportional),
        ("about-serif-semibold", "MaruBuri", "SemiBold", egui::FontFamily::Name("semibold".into())),
        ("about-sans", "Pretendard", "Regular", egui::FontFamily::Proportional),
        ("about-sans-bold", "Pretendard", "Bold", egui::FontFamily::Name("semibold".into())),
    ] {
        let mut stack = Vec::new();
        if let Some(face) = faces.iter().find(|f| f.family == family && f.style == style && !f.bytes.is_empty()) {
            fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_static(face.bytes)));
            stack.push(name.to_string());
        }
        stack.extend(fonts.families.get(&fallback).into_iter().flatten().cloned());
        fonts.families.insert(egui::FontFamily::Name(name.into()), stack);
    }
}

fn font(size: f32, family: &str) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(family.into()))
}

pub(crate) fn compact(ctx: &egui::Context) -> bool {
    ctx.content_rect().width() < 900.0 || ctx.content_rect().height() < 700.0
}

pub(crate) fn margins(ctx: &egui::Context) -> egui::Margin {
    if compact(ctx) { egui::Margin::same(24) } else { egui::Margin { left: 52, right: 52, top: 44, bottom: 44 } }
}

pub(crate) fn width(ctx: &egui::Context) -> f32 {
    (ctx.content_rect().width() - 48.0 - f32::from(margins(ctx).left) * 2.0).clamp(240.0, 636.0)
}

pub(crate) fn header(ui: &mut egui::Ui) -> Rect {
    ui.horizontal(|ui| {
        if let Some(icon) = logo(ui.ctx()) {
            ui.add(egui::Image::new(&icon).fit_to_exact_size(vec2(60.0, 60.0)).alt_text("Prism logo"));
        }
        ui.add_space(18.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(tl!("Prism")).font(font(32.0, "about-sans-bold")).color(palette().text));
            ui.add_space(6.0);
            ui.label(
                RichText::new(crate::i18n::fmt(tl!("Version {version}"), &[("version", prism_engine::build_info::VERSION)]))
                    .font(font(12.0, "about-sans"))
                    .color(palette().text_dim),
            );
        });
    })
    .response
    .rect
}

fn logo(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let id = egui::Id::new("prism-about-logo");
    if let Some(texture) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(texture);
    }
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/app-icon/prism-1024.png"))
        .inspect_err(|e| log::warn!("Prism About logo: {e}"))
        .ok()?;
    let image = egui::ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
    let texture = ctx.load_texture("prism-about-logo", image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    Some(texture)
}

// Prefer spaces over egui's character-level CJK breaks, retaining authored sentence breaks.
fn reading_lines(ctx: &egui::Context, text: &str, font: &egui::FontId, width: f32) -> String {
    let mut rows = Vec::new();
    for line in text.lines() {
        let mut row = String::new();
        for word in line.split_whitespace() {
            let candidate = if row.is_empty() { word.to_string() } else { format!("{row} {word}") };
            let fits = ctx.fonts_mut(|f| f.layout_no_wrap(candidate.clone(), font.clone(), palette().text).size().x <= width);
            if !fits && !row.is_empty() {
                rows.push(std::mem::take(&mut row));
                row = word.to_string();
            } else {
                row = candidate;
            }
        }
        rows.push(row);
    }
    rows.join("\n")
}

fn philosophy_label(ui: &mut egui::Ui, text: &str, font: egui::FontId, line_height: f32) {
    let mut job = egui::text::LayoutJob::simple(reading_lines(ui.ctx(), text, &font, ui.available_width()), font, palette().text, ui.available_width());
    for section in &mut job.sections {
        section.format.line_height = Some(line_height);
    }
    let response = ui.add(egui::Label::new(job).wrap());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
}

pub(crate) fn body(ui: &mut egui::Ui) {
    let small = compact(ui.ctx());
    // Reserve header, frame, fixed footer and dismissal row before sizing the scroll viewport.
    let budget = if small { 251.0 } else { 355.0 };
    egui::ScrollArea::vertical()
        .id_salt("prism-about-philosophy")
        .max_height((ui.ctx().content_rect().height() - budget).max(40.0))
        .min_scrolled_height(0.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            philosophy_label(ui, crate::i18n::t(PHILOSOPHY[0]), font(if small { 24.0 } else { 28.0 }, "about-serif-semibold"), if small { 36.0 } else { 44.0 });
            ui.add_space(if small { 18.0 } else { 24.0 });
            philosophy_label(ui, crate::i18n::t(PHILOSOPHY[1]), font(if small { 15.0 } else { 17.0 }, "about-serif"), if small { 26.0 } else { 31.0 });
            ui.add_space(if small { 22.0 } else { 28.0 });
            philosophy_label(ui, crate::i18n::t(PHILOSOPHY[2]), font(if small { 15.0 } else { 17.0 }, "about-serif"), if small { 26.0 } else { 31.0 });
        });
    ui.add_space(if small { 12.0 } else { 26.0 });
    let (rule, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment([rule.left_center(), rule.right_center()], egui::Stroke::new(0.75, palette().separator));
    ui.add_space(if small { 14.0 } else { 22.0 });
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), if small { 58.0 } else { 48.0 }), Sense::hover());
    let brand_width = 128.0_f32.min(row.width() * 0.32);
    let left = Rect::from_min_max(row.min, egui::pos2(row.right() - brand_width - 16.0, row.bottom()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(left));
    child.add(
        egui::Label::new(RichText::new(format!("© 2026 NK Studio. {}", tl!("All rights reserved."))).font(font(11.5, "about-sans")).color(palette().text_dim))
            .wrap(),
    );
    child.add_space(6.0);
    child.add(egui::Label::new(RichText::new(crate::i18n::t(PHILOSOPHY[3])).font(font(12.0, "about-sans")).color(palette().text_dim)).wrap());
    paint_wordmark(ui, Rect::from_min_max(egui::pos2(row.right() - brand_width, row.top()), row.max));
}

pub(crate) fn ok_button(ui: &mut egui::Ui) -> bool {
    let t = palette();
    let label = tl!("OK");
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font(14.0, "about-sans"), t.primary_text);
    let size = vec2(94.0_f32.max(galley.size().x + 32.0), 34.0_f32.max(galley.size().y + 12.0));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    if ui.is_rect_visible(rect) {
        // Explicit state colours avoid inherited editor widget styling and hover expansion.
        let fill = if response.is_pointer_button_down_on() {
            t.primary_bg.lerp_to_gamma(t.card, 0.04)
        } else if response.hovered() {
            t.primary_bg.lerp_to_gamma(t.text_dim, 0.3)
        } else {
            t.primary_bg
        };
        ui.painter().rect_filled(rect, rect.height() / 2.0, fill);
        if response.has_focus() {
            ui.painter().rect_stroke(rect.expand(3.0), rect.height() / 2.0 + 3.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Outside);
        }
        // Centre the actual glyph meshes, independently of the parent's top-aligned layout
        // and the face's ascender/descender whitespace. The position never changes on hover.
        let bounds = if galley.mesh_bounds.is_finite() && !galley.mesh_bounds.is_negative() { galley.mesh_bounds } else { galley.rect };
        let pixels_per_point = ui.ctx().pixels_per_point().max(0.1);
        let position = rect.center() - bounds.center().to_vec2();
        let position = ((position.to_vec2() * pixels_per_point).round() / pixels_per_point).to_pos2();
        ui.painter().galley(position, galley, t.primary_text);
    }
    response.clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter))
}

fn wordmark_layer() -> TextLayer {
    let text = "NK Studio".to_string();
    let style = CharStyle {
        font_family: "Pretendard".into(),
        font_style: "Bold".into(),
        weight: 700,
        size_pt: 24.0,
        color: prism_color::Color::WHITE,
        ..Default::default()
    };
    let mut layer = TextLayer { runs: vec![TextRun { len: text.len(), style }], text, ..Default::default() };
    layer.sync_summary();
    layer
}

#[derive(Default)]
struct WordmarkCache {
    generation: Option<(u64, u32)>,
    texture: Option<egui::TextureHandle>,
}

fn wordmark_image(scale: f32) -> Option<egui::ColorImage> {
    let mut engine = prism_text::shared().lock().unwrap_or_else(PoisonError::into_inner);
    let rendered = engine.render(&wordmark_layer(), 72.0 * scale, prism_color::PixelFormat::RGBA8).1;
    let rect = rendered.surface.content_bounds();
    let width = usize::try_from(rect.width()).ok()?;
    let height = usize::try_from(rect.height()).ok()?;
    let count = width.checked_mul(height)?;
    if count == 0 || count > 1_048_576 {
        return None;
    }
    let mut rgba = vec![[0.0; 4]; count];
    rendered.surface.read_rgba_into(rect, &mut rgba);
    Some(egui::ColorImage::new(
        [width, height],
        rgba.into_iter().map(|p| egui::Color32::from_white_alpha((p[3].clamp(0.0, 1.0) * 255.0).round() as u8)).collect(),
    ))
}

fn paint_wordmark(ui: &mut egui::Ui, rect: Rect) {
    let response = ui.interact(rect, ui.id().with("nk-studio-wordmark"), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "NK Studio"));
    let cache = ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<Arc<Mutex<WordmarkCache>>>(egui::Id::new("about-wordmark-cache")).clone());
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    let scale = ui.ctx().pixels_per_point().clamp(0.5, 4.0);
    let generation = (prism_engine::type_font_cmds::revision(), scale.to_bits());
    if cache.generation != Some(generation) {
        cache.texture = wordmark_image(scale).map(|image| ui.ctx().load_texture("nk-studio-pretendard-bold", image, egui::TextureOptions::LINEAR));
        cache.generation = Some(generation);
    }
    if let Some(texture) = &cache.texture {
        let size = texture.size_vec2() / scale;
        let size = size * (rect.width() / size.x).min(1.0);
        let at = Rect::from_min_size(egui::pos2(rect.right() - size.x, rect.center().y - size.y / 2.0), size);
        ui.painter().image(texture.id(), at, Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), palette().text);
    } else {
        ui.painter().text(rect.right_center(), egui::Align2::RIGHT_CENTER, "NK Studio", theme::semibold(24.0), palette().text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DialogKind;
    use egui_kittest::{Harness, kittest::Queryable};

    fn app() -> PrismApp {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("file.new", serde_json::json!({"width": 640, "height": 480, "name": "Working document"})).unwrap();
        app.run("layer.new.layer", serde_json::json!({})).unwrap();
        app.sync_views();
        app.integrated_titlebar = cfg!(target_os = "macos");
        app
    }

    fn draw(ui: &mut egui::Ui, app: &mut PrismApp) {
        let _lang = crate::i18n::language_scope(crate::i18n::Lang::from_code("ko").unwrap());
        if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
            return;
        }
        crate::panels::title_bar(app, ui);
        egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        crate::dialogs::show(app, ui.ctx());
    }

    #[test]
    fn about_card_shows_only_prism_and_places_studio_at_bottom_right() {
        for (size, scale) in [
            (vec2(480.0, 360.0), 1.0),
            (vec2(480.0, 360.0), 2.0),
            (vec2(640.0, 480.0), 1.0),
            (vec2(640.0, 480.0), 2.0),
            (vec2(960.0, 640.0), 1.0),
            (vec2(1200.0, 800.0), 2.0),
        ] {
            let mut app = app();
            app.ui.open_dialog(DialogKind::About, Default::default());
            let mut h = Harness::builder().with_size(size).with_pixels_per_point(scale).build_ui_state(draw, app);
            PrismApp::setup_context(&h.ctx, theme::ThemeKind::ProMedium);
            h.run_steps(5);
            let frame = h.ctx.memory(|m| m.area_rect(egui::Id::new(("dialog", h.state().ui.dialogs[0].id)))).unwrap();
            assert!(frame.left() >= 0.0 && frame.right() <= size.x && frame.bottom() <= size.y, "{frame:?}");
            if size.y >= 800.0 {
                assert!((frame.center().y - size.y / 2.0).abs() <= 1.0, "centred after font/layout changes: {frame:?}");
            }
            let logo = h.get_by_label("NK Studio").rect();
            let copyright = h.get_by_label("© 2026 NK Studio. 모든 권리 보유.").rect();
            assert!(logo.left() > copyright.right());
            assert!(logo.bottom() > frame.center().y);
            assert!(h.query_by_label("감각과 논리 사이,\n새로운 표현을 만듭니다.").is_some());
            let tagline = h.get_by_label("상상을 그리고, 가능성을 만드는 도구.").rect();
            let ok = h.get_by_label("확인").rect();
            assert!(tagline.bottom() < ok.top() && ok.bottom() < frame.bottom());
            // In compact windows the paragraphs scroll, while the footer stays outside the clip.
            let lang = crate::i18n::Lang::from_code("ko").unwrap();
            // Authored breaks survive wide cards; compact cards reflow at spaces.
            for source in PHILOSOPHY {
                let text = crate::i18n::tr(lang, source);
                let id = font(15.0, "about-serif");
                let wrapped = reading_lines(&h.ctx, text, &id, 260.0);
                assert_eq!(wrapped.split_whitespace().collect::<Vec<_>>(), text.split_whitespace().collect::<Vec<_>>());
                for line in wrapped.lines() {
                    let galley = h.ctx.fonts_mut(|f| f.layout_no_wrap(line.into(), id.clone(), palette().text));
                    assert!(galley.size().x <= 260.0, "whole Korean words fit at {scale}x: {line}");
                }
            }
            let first = h.get_by_label(crate::i18n::tr(lang, PHILOSOPHY[1])).rect();
            let last = h.get_by_label(crate::i18n::tr(lang, PHILOSOPHY[2])).rect();
            assert!(first.top() < last.top());
            assert!(last.width() <= frame.width());
            assert!(h.query_by_label(&format!("버전 {}", prism_engine::build_info::VERSION)).is_some());
            for removed in ["정보", "기여자", "모델", "GitHub", "MIT / Apache-2.0", "PhotoCraft"] {
                assert!(h.query_by_label(removed).is_none(), "{removed} must not appear");
            }
            // Legacy stored dialog fields cannot bring back the removed tabs.
            h.state_mut().ui.dialogs[0].fields.insert("tab".into(), serde_json::json!("contributors"));
            h.run_steps(3);
            assert!(h.query_by_label("감각과 논리 사이,\n새로운 표현을 만듭니다.").is_some());
            if last.bottom() >= copyright.top() {
                h.hover_at(h.get_by_label("감각과 논리 사이,\n새로운 표현을 만듭니다.").rect().center());
                h.run_steps(2);
                h.event(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, -500.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                });
                h.run_steps(15);
                let scrolled = h.get_by_label(crate::i18n::tr(lang, PHILOSOPHY[2])).rect();
                assert!(scrolled.bottom() < copyright.top(), "the full ending can be read above the fixed footer: {scrolled:?}");
                assert_eq!(h.get_by_label("확인").rect(), ok, "scrolling must not move OK");
            }
            if size.y >= 800.0 {
                h.set_size(vec2(480.0, 360.0));
                h.run_steps(8);
                let resized = h.ctx.memory(|m| m.area_rect(egui::Id::new(("dialog", h.state().ui.dialogs[0].id)))).unwrap();
                assert!(resized.left() >= 0.0 && resized.right() <= 480.0 && resized.top() >= 0.0 && resized.bottom() <= 360.0, "{resized:?}");
                assert!(h.get_by_label("확인").rect().bottom() < 360.0, "OK remains accessible after resizing an open card");
            }
            h.get_by_label("확인").click();
            h.run_steps(3);
            assert!(h.state().ui.dialogs.is_empty());
        }
    }

    #[test]
    fn philosophy_uses_korean_copy_and_english_fallback_for_other_locales() {
        let ko = crate::i18n::Lang::from_code("ko").unwrap();
        for (source, expected) in PHILOSOPHY.into_iter().zip([
            "감각과 논리 사이,\n새로운 표현을 만듭니다.",
            "아트는 상상에 모습을 주고, 프로그래밍은 그 상상이 움직이게 합니다.\n게임을 만드는 일은 두 언어를 이어 하나의 세계를 완성하는 과정입니다.",
            "프리즘은 그 만남에서 출발했습니다.\n빛이 프리즘을 지나 다양한 색으로 펼쳐지듯,\n창작자의 아이디어가 자신만의 표현으로 펼쳐지기를 바랍니다.",
            "상상을 그리고, 가능성을 만드는 도구.",
        ]) {
            assert_eq!(crate::i18n::tr(ko, source), expected);
            for lang in crate::i18n::Lang::all().filter(|l| *l != ko) {
                assert_eq!(crate::i18n::tr(lang, source), source, "{}", lang.code());
            }
        }
    }

    #[test]
    fn ok_button_centres_glyphs_and_has_stable_hover_and_pressed_states() {
        for scale in [1.0, 2.0] {
            let mut h = Harness::builder().with_size(vec2(240.0, 100.0)).with_pixels_per_point(scale).build_ui(|ui| {
                let _lang = crate::i18n::language_scope(crate::i18n::Lang::from_code("ko").unwrap());
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("about-sans".into()))) {
                    return;
                }
                // This is the About footer's inherited layout, which must not top-align text.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    ok_button(ui);
                });
            });
            PrismApp::setup_context(&h.ctx, theme::ThemeKind::ProMedium);
            h.run_steps(4);
            let rect = h.get_by_label("확인").rect();
            let inspect = |h: &Harness<'_>| {
                let glyphs = h
                    .output()
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Text(t) if t.galley.job.text == "확인" => Some(t.galley.mesh_bounds.translate(t.pos.to_vec2())),
                        _ => None,
                    })
                    .unwrap();
                assert!((glyphs.center().x - rect.center().x).abs() <= 0.5 / scale);
                assert!((glyphs.center().y - rect.center().y).abs() <= 0.5 / scale);
                let fill = h
                    .output()
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Rect(r) if r.rect == rect => Some(r.fill),
                        _ => None,
                    })
                    .unwrap();
                (glyphs, fill)
            };
            let (normal_text, normal_fill) = inspect(&h);
            assert_eq!(normal_fill, palette().primary_bg);
            h.hover_at(rect.center());
            h.run_steps(3);
            let (hover_text, hover_fill) = inspect(&h);
            assert_eq!(h.get_by_label("확인").rect(), rect);
            assert_eq!(hover_text, normal_text);
            assert_ne!(hover_fill, normal_fill);
            assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::PointingHand);
            h.event(egui::Event::PointerButton { pos: rect.center(), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
            h.run_steps(2);
            let (pressed_text, pressed_fill) = inspect(&h);
            assert_eq!(pressed_text, normal_text);
            assert_ne!(pressed_fill, hover_fill);
            assert_ne!(pressed_fill, normal_fill);
            assert_eq!(pressed_fill.a(), 255);
            assert_eq!(h.get_by_label("확인").rect(), rect);
            h.event(egui::Event::PointerButton { pos: rect.center(), button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            h.hover_at(egui::pos2(1.0, 1.0));
            h.run_steps(3);
            assert_eq!(inspect(&h), (normal_text, normal_fill));
            h.get_by_label("확인").focus();
            h.run_steps(2);
            assert!(h.output().shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.stroke.color == palette().accent)));
        }
    }

    #[test]
    fn about_dismisses_with_enter_and_escape() {
        for key in [egui::Key::Enter, egui::Key::Escape] {
            let mut app = app();
            app.ui.open_dialog(DialogKind::About, Default::default());
            let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).build_ui_state(draw, app);
            PrismApp::setup_context(&h.ctx, theme::ThemeKind::StudioLight);
            h.run_steps(5);
            h.event(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
            h.run_steps(3);
            assert!(h.state().ui.dialogs.is_empty(), "dismissal with {key:?}");
        }
    }

    #[test]
    fn studio_fonts_are_scoped_and_work_without_optional_assets() {
        for faces in [&[][..], prism_text::CRAFT_FONTS] {
            let mut definitions = egui::FontDefinitions::default();
            let editor = definitions.families[&egui::FontFamily::Proportional].clone();
            definitions.families.insert(egui::FontFamily::Name("semibold".into()), editor.clone());
            install_fonts_with(&mut definitions, faces);
            assert_eq!(definitions.families[&egui::FontFamily::Proportional], editor);
            for (name, family, style) in [
                ("about-serif", "MaruBuri", "Regular"),
                ("about-serif-semibold", "MaruBuri", "SemiBold"),
                ("about-sans", "Pretendard", "Regular"),
                ("about-sans-bold", "Pretendard", "Bold"),
            ] {
                let stack = &definitions.families[&egui::FontFamily::Name(name.into())];
                if let Some(face) = faces.iter().find(|f| f.family == family && f.style == style) {
                    assert_eq!(stack.first().map(String::as_str), Some(name));
                    assert_eq!(definitions.font_data[name].font.as_ref(), face.bytes);
                } else {
                    assert_eq!(stack, &editor);
                }
            }
        }
    }

    #[test]
    fn about_uses_the_current_prism_app_logo_and_caches_it() {
        let ctx = egui::Context::default();
        let first = logo(&ctx).unwrap();
        assert_eq!(first.size(), [1024, 1024]);
        assert_eq!(first.id(), logo(&ctx).unwrap().id());
    }

    #[test]
    fn titlebar_keeps_the_app_name_when_a_document_is_open() {
        let mut h = Harness::builder().with_size(vec2(1800.0, 500.0)).build_ui_state(draw, app());
        PrismApp::setup_context(&h.ctx, theme::ThemeKind::ProMedium);
        h.run_steps(5);
        let title = h.get_by_label("프리즘").rect();
        if crate::panels::BRAND_MARK {
            assert!(title.left() < 20.0 && title.width() < 30.0, "Windows: the labelled logo leads the bar, no centred name: {title:?}");
        } else {
            assert!((title.center().x - 900.0).abs() < 1.0, "centred on the window, including the macOS traffic-light inset");
        }
        assert!(h.state().session.active().unwrap().is_dirty());
    }

    #[test]
    fn studio_wordmark_requests_pretendard_bold_and_rasterizes_at_both_scales() {
        let layer = wordmark_layer();
        assert_eq!(layer.font_family, "Pretendard");
        assert_eq!(layer.char_runs()[0].style.weight, 700);
        for scale in [1.0, 2.0] {
            let image = wordmark_image(scale).unwrap();
            assert!(image.size[0] > 100 && image.size[1] > 10);
            assert!(image.pixels.iter().any(|p| p.a() == 255));
        }
    }

    #[test]
    #[ignore = "writes About-card and title-bar GPU visual evidence"]
    fn about_brand_visual_evidence() {
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/about-card");
        std::fs::create_dir_all(&out).unwrap();
        assert!(
            prism_text::shared().lock().unwrap().fonts.faces("Pretendard").iter().any(|f| f.weight == 700.0 && !f.italic),
            "installed Pretendard Bold fixture"
        );
        #[cfg(target_os = "macos")]
        {
            let bold = std::fs::read(std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join("Library/Fonts/Pretendard-Bold.otf")).unwrap();
            let layout = prism_text::shared().lock().unwrap().layout(&wordmark_layer(), 72.0);
            assert!(!layout.faces.is_empty());
            assert!(
                layout.faces.iter().all(|f| f.font.data.as_ref() == bold.as_slice() && !f.embolden),
                "actual installed Bold face, with no synthetic weight or fallback"
            );
        }
        for kind in [theme::ThemeKind::ProMedium, theme::ThemeKind::StudioLight] {
            for scale in [1.0, 2.0] {
                let mut app = app();
                app.ui.open_dialog(DialogKind::About, Default::default());
                let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_pixels_per_point(scale).wgpu().build_ui_state(draw, app);
                PrismApp::setup_context(&h.ctx, kind);
                h.run_steps(5);
                h.render().unwrap().save(out.join(format!("{kind:?}-{scale}x.png"))).unwrap();
            }
        }
    }
}
