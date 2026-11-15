use prism_color::{ColorMode, SampleType};
use prism_doc::figure::{FigureScene, standing_figure};
use prism_doc::{Document, Layer, Size};

#[test]
fn psd_export_keeps_figure_pixels_and_reports_the_scene_it_drops() {
    let mut doc = Document::new("Figures", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
    let mut posed = Layer::raster("Posed", doc.pixel_format());
    posed.figure = Some(FigureScene { figures: vec![standing_figure()], ..Default::default() });
    doc.layers.push(posed);
    doc.layers.push(Layer::raster("Plain", doc.pixel_format()));
    let (_, warnings) = prism_io::document_to_psd_with(&doc, &Default::default());
    let figure_warnings: Vec<_> = warnings.iter().filter(|w| w.contains("3D figures")).collect();
    assert_eq!(figure_warnings.len(), 1, "{warnings:?}");
    assert!(figure_warnings[0].contains("Posed"));
}
