//! Visible-row font previews, bounded per context and invalidated on DPI/import changes.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

#[derive(Default)]
struct Cache {
    generation: (u64, u32),
    textures: HashMap<String, Option<egui::TextureHandle>>,
}

pub(crate) fn paint(ui: &mut egui::Ui, family: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(220.0_f32.min(ui.available_width()), 32.0), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let id = egui::Id::new("font-preview-cache");
    let cache = ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<Arc<Mutex<Cache>>>(id).clone());
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    let scale = ui.ctx().pixels_per_point().clamp(0.5, 4.0);
    let generation = (prism_engine::type_font_cmds::revision(), scale.to_bits());
    if cache.generation != generation {
        cache.textures.clear();
        cache.generation = generation;
    }
    if !cache.textures.contains_key(family) {
        // At most 64 RGBA previews (under 29 MiB even at 4x), and only visible rows enter.
        if cache.textures.len() >= 64 {
            cache.textures.clear();
        }
        let source = prism_text::shared().lock().unwrap_or_else(PoisonError::into_inner).fonts.preview_source(family);
        let texture = source.and_then(|s| s.render(scale)).map(|p| {
            let pixels = p.coverage.into_iter().map(|a| egui::Color32::from_white_alpha((a.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
            ui.ctx().load_texture(format!("font-preview-{family}"), egui::ColorImage::new([p.width, p.height], pixels), egui::TextureOptions::LINEAR)
        });
        cache.textures.insert(family.to_owned(), texture);
    }
    if let Some(Some(texture)) = cache.textures.get(family) {
        ui.painter().image(texture.id(), rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), ui.visuals().text_color());
    } else {
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, tl!("No preview"), egui::FontId::proportional(12.0), ui.visuals().weak_text_color());
    }
}
