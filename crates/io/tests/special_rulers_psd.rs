use prism_color::{ColorMode, SampleType};
use prism_doc::{Document, Layer, RulerKind, Size, SpecialRuler, SpecialRulers};

#[test]
fn psd_export_reports_special_rulers_it_drops() {
    let mut doc = Document::new("Rulers", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
    let mut ruled = Layer::raster("Ruled", doc.pixel_format());
    ruled.special_rulers = Some(SpecialRulers { rulers: vec![SpecialRuler { snap: true, kind: RulerKind::Parallel { angle_deg: 30.0, center: [0.0, 0.0] } }], ..Default::default() });
    let mut empty = Layer::raster("Empty set", doc.pixel_format());
    empty.special_rulers = Some(SpecialRulers::default());
    doc.layers.push(ruled);
    doc.layers.push(empty);
    let (_, warnings) = prism_io::document_to_psd_with(&doc, &Default::default());
    let ruler_warnings: Vec<_> = warnings.iter().filter(|w| w.contains("special rulers")).collect();
    assert_eq!(ruler_warnings.len(), 1, "{warnings:?}");
    assert!(ruler_warnings[0].contains("Ruled"));
}
