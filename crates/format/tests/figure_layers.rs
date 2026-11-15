use prism_color::{ColorMode, SampleType};
use prism_doc::figure::{FigureScene, standing_figure};
use prism_doc::{Affine, Document, Layer, Size};

#[test]
fn figure_scenes_roundtrip_through_prism_and_bad_ones_are_refused() {
    let mut doc = Document::new("Figures", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("3D Figure 1", doc.pixel_format());
    let mut scene = FigureScene { figures: vec![standing_figure(), standing_figure()], ..Default::default() };
    scene.camera.yaw = 33.0;
    scene.placement = Affine::translate(4.0, 2.0).mul(&Affine::scale(0.5));
    scene.look.outline = false;
    layer.figure = Some(scene);
    doc.layers.push(layer);
    doc.layers.push(Layer::raster("Plain", doc.pixel_format()));
    let bytes = prism_format::save_to_bytes(&doc, &Default::default()).unwrap();
    assert_eq!(prism_format::load_from_bytes(&bytes).unwrap(), doc);
    // A file whose scene fails validation is reported as corrupt, not loaded.
    let mut bad = doc.clone();
    if let Some(s) = bad.layers[0].figure.as_mut() {
        s.camera.distance = 0.0;
    }
    let bytes = prism_format::save_to_bytes(&bad, &Default::default()).unwrap();
    assert!(prism_format::load_from_bytes(&bytes).is_err());
}
