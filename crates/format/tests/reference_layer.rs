use prism_color::{ColorMode, SampleType};
use prism_doc::{Document, Layer, Size};

/// The reference-layer flag survives a `.prism` save, on layers and groups, and a file without
/// it (every older file) loads with the flag off.
#[test]
fn reference_flag_roundtrips_through_prism() {
    let mut doc = Document::new("Lines", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
    let mut lines = Layer::raster("Line art", doc.pixel_format());
    lines.reference = true;
    let mut group = Layer::group("Folder", vec![Layer::raster("Child", doc.pixel_format())]);
    group.reference = true;
    doc.layers.push(lines);
    doc.layers.push(Layer::raster("Colour", doc.pixel_format()));
    doc.layers.push(group);
    let bytes = prism_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let loaded = prism_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(loaded, doc);
    assert_eq!(loaded.layers.iter().map(|l| l.reference).collect::<Vec<_>>(), [true, false, true]);

    let plain = Document::new("Plain", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
    let bytes = prism_format::save_to_bytes(&plain, &Default::default()).unwrap();
    assert!(prism_format::load_from_bytes(&bytes).unwrap().layers.iter().all(|l| !l.reference));
}
