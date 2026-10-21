use prism_color::{ColorMode, SampleType};
use prism_doc::{Document, Layer, PerspectiveRuler, RulerKind, RulerRange, Size, SpecialRuler, SpecialRulers, VanishTarget, VanishingPoint};

fn rulers() -> SpecialRulers {
    let parallel = |a: f64| VanishingPoint { target: VanishTarget::Parallel { angle_deg: a }, handle: [-8_415_698.0, -4_980_577.0], guides: vec![[631.0, 566.0]], ..Default::default() };
    let point = VanishingPoint { snap: false, target: VanishTarget::Point([400.0, -120.5]), handle: [400.0, -120.5], guides: Vec::new() };
    SpecialRulers {
        rulers: vec![
            SpecialRuler {
                snap: true,
                kind: RulerKind::Perspective(PerspectiveRuler { points: vec![parallel(30.6163), parallel(149.3961), point], eye_level: Some([10.0, 20.0]) }),
            },
            SpecialRuler { snap: false, kind: RulerKind::Parallel { angle_deg: 45.0, center: [3.0, 4.0] } },
        ],
        shown: false,
        range: RulerRange::SameFolder,
    }
}

#[test]
fn special_rulers_roundtrip_through_prism() {
    let mut doc = Document::new("Rulers", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("Isometric ruler", doc.pixel_format());
    layer.special_rulers = Some(rulers());
    let mut group = Layer::group("Folder", vec![Layer::raster("Child", doc.pixel_format())]);
    group.special_rulers = Some(SpecialRulers::default());
    doc.layers.push(layer);
    doc.layers.push(group);
    let bytes = prism_format::save_to_bytes(&doc, &Default::default()).unwrap();
    assert_eq!(prism_format::load_from_bytes(&bytes).unwrap(), doc);
}

#[test]
fn out_of_range_special_rulers_are_refused_on_load() {
    let mut doc = Document::new("Bad", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("Ruler", doc.pixel_format());
    let mut r = rulers();
    r.rulers[1].kind = RulerKind::Parallel { angle_deg: 0.0, center: [1e12, 0.0] };
    layer.special_rulers = Some(r);
    doc.layers.push(layer);
    let bytes = prism_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let err = prism_format::load_from_bytes(&bytes).unwrap_err().to_string();
    assert!(err.contains("special ruler"), "{err}");
}
