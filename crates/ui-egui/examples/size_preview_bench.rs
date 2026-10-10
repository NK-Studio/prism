//! Bounded Image Size preview on synthetic 24/36 MP layered documents (no GPU required).
//! `cargo run --release -p photocraft-ui-egui --example size_preview_bench`
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use std::{hint::black_box, time::Instant};

fn main() {
    for (w, h) in [(6000, 4000), (7360, 4912)] {
        let mut doc = Document::with_background("Synthetic preview", Size::new(w, h), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut paint = photocraft_raster::Surface::new(doc.pixel_format());
        paint.fill_rect(Rect::from_xywh(0, 0, w / 2, h / 2), &[0.12, 0.3, 0.6, 0.8]);
        doc.layers.push(Layer::new("Synthetic rectangle", LayerContent::Raster(paint)));
        let before = Instant::now();
        let full = photocraft_compose::flatten(&doc);
        black_box(&full);
        let full_ms = before.elapsed().as_secs_f64() * 1000.0;
        drop(full);
        let started = Instant::now();
        let preview = std::sync::Arc::new(photocraft_compose::thumbnail_buffer(&doc, 512));
        let preview_ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut surface = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA32F);
        let pixels: Vec<_> = preview.px.iter().flat_map(|p| p.iter().copied()).collect();
        surface.write_region(preview.rect, &pixels);
        let changed = Instant::now();
        for _ in 0..10 {
            black_box(photocraft_algo::resample::resize_surface_in_canvas(&surface, 1.0, 0.75, photocraft_algo::resample::Resample::Bicubic, preview.rect));
        }
        let changed_ms = changed.elapsed().as_secs_f64() * 100.0;
        let cached = Instant::now();
        for _ in 0..1000 {
            black_box(preview.clone());
        }
        println!(
            "{w}x{h}: full composite {full_ms:.2} ms; initial bounded preview {preview_ms:.2} ms; settings change {changed_ms:.2} ms; cached handle {:.4} ms/frame; pixels {}",
            cached.elapsed().as_secs_f64(),
            preview.px.len()
        );
    }
}
