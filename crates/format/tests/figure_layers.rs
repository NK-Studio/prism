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

#[test]
fn character_models_roundtrip_as_blobs() {
    use prism_doc::figure::FigureModel;
    use std::sync::Arc;
    let mut doc = Document::new("Characters", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("3D Figure 1", doc.pixel_format());
    let mut character = standing_figure();
    let file: Vec<u8> = (0..70_000u32).map(|i| (i * 7 % 251) as u8).collect();
    character.model = Some(FigureModel { file_name: "girl.vrm".into(), bytes: Arc::new(file.clone()) });
    // A mannequin, the character, and the same character again (one blob for both).
    let scene = FigureScene { figures: vec![standing_figure(), character.clone(), character], ..Default::default() };
    layer.figure = Some(scene);
    doc.layers.push(layer);
    let bytes = prism_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let back = prism_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(back, doc);
    let figs = &back.layers[0].figure.as_ref().unwrap().figures;
    assert!(figs[0].model.is_none());
    assert_eq!(figs[1].model.as_ref().unwrap().bytes.as_slice(), file.as_slice());
    assert_eq!(figs[2].model.as_ref().unwrap().file_name, "girl.vrm");
    // The bytes are not in the manifest's JSON.
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("\"bytes\""));
}
