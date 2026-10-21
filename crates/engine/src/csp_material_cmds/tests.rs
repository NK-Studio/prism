use super::*;
use prism_doc::special_ruler::{PerspectiveRuler, RulerKind, SpecialRuler, SpecialRulers, VanishTarget, VanishingPoint};
use prism_io::sut::layer_material::MaterialLayer;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    s
}

fn layer(name: &str) -> MaterialLayer {
    MaterialLayer { name: name.into(), visible: true, opacity: 1.0, offset: [0, 0], image: None, rulers: None }
}

/// The "Isometric ruler" material's shape: a 1280 × 720 canvas, three parallel points.
fn iso() -> LayerMaterial {
    let point = |a: f64, h: [f64; 2]| VanishingPoint { target: VanishTarget::Parallel { angle_deg: a }, handle: h, guides: vec![[631.0, 566.0]], snap: true };
    let ruler = SpecialRuler {
        snap: true,
        kind: RulerKind::Perspective(PerspectiveRuler {
            points: vec![point(30.6163181116636, [-8.4e6, -4.9e6]), point(149.39613684716832, [3.4e6, -2.0e6]), point(270.0, [-223.0, -268.0])],
            eye_level: Some([-2.5e6, -3.5e6]),
        }),
    };
    let rulers = SpecialRulers { rulers: vec![ruler], ..Default::default() };
    LayerMaterial { width: 1280, height: 720, layers: vec![MaterialLayer { rulers: Some(rulers), ..layer("Isometric ruler") }], warnings: vec![] }
}

fn image(w: u32, h: u32) -> MaterialImage {
    MaterialImage { width: w, height: h, rgba: [255, 0, 0, 255].repeat((w * h) as usize) }
}

#[test]
fn bad_params_fail_and_change_nothing() {
    let mut none = Session::new();
    assert!(none.execute(IMPORT, json!({"path": "x.layer"})).is_err(), "needs a document");
    let mut s = session();
    let past = s.active().unwrap().history.past_len();
    for p in [
        json!({}),
        json!({"uuid": "a", "path": "b"}),
        json!({"uuid": 3}),
        json!({"uuid": ""}),
        json!({"path": ["x"]}),
        json!({"path": "/nonexistent/prism/material_0.layer"}),
        json!({"uuid": "id", "root": ""}),
        json!({"uuid": "id", "root": "/nonexistent/prism-csp-root"}),
    ] {
        assert!(s.execute(IMPORT, p.clone()).is_err(), "{p}");
    }
    assert_eq!(s.active().unwrap().history.past_len(), past);
}

#[test]
fn a_ruler_material_adds_one_ruler_layer_in_one_step_and_snaps_strokes() {
    let mut s = session();
    let past = s.active().unwrap().history.past_len();
    let r = place(&mut s, "Isometric ruler", iso(), &MaterialInfo { tags: vec!["SpecialRuler".into()], tiling: false }).unwrap();
    assert_eq!(r["rulers"], 1);
    let id = LayerId(r["layers"][0].as_u64().unwrap());
    let st = s.active().unwrap();
    assert_eq!(st.history.past_len(), past + 1);
    assert_eq!(st.active_layer, Some(id));
    let l = st.doc.layer(id).unwrap();
    assert_eq!(l.name, "Isometric ruler");
    let RulerKind::Perspective(p) = &l.special_rulers.as_ref().unwrap().rulers[0].kind else { panic!("perspective") };
    // The 1280 × 720 material is centred on the 200 × 100 document: moved by (-540, -310).
    assert_eq!(p.points[0].guides, [[91.0, 256.0]]);
    assert_eq!(p.points[2].target, VanishTarget::Parallel { angle_deg: 270.0 });

    // A drag at about 28° lands exactly on the 30.6° line through its start.
    let pts: Vec<Value> = (0..12).map(|i| json!([20.0 + 4.0 * i as f64, 80.0 - 2.1 * i as f64])).collect();
    let out = s.execute("specialRuler.snapPoints", json!({"points": pts})).unwrap();
    let (a, b) = (&out["points"][0], &out["points"][11]);
    let angle = (-(b[1].as_f64().unwrap() - a[1].as_f64().unwrap())).atan2(b[0].as_f64().unwrap() - a[0].as_f64().unwrap()).to_degrees();
    assert!((angle - 30.6163181116636).abs() < 1e-9, "{angle}");

    s.undo();
    assert!(s.active().unwrap().doc.layer(id).is_none(), "one undo removes the material");
}

#[test]
fn images_become_pixels_or_a_tiling_pattern_fill() {
    let mut s = session();
    let mut m = LayerMaterial { width: 100, height: 50, layers: vec![MaterialLayer { image: Some(image(4, 2)), offset: [10, 5], opacity: 0.5, ..layer("Ink") }], warnings: vec![] };
    let r = place(&mut s, "Ink", m.clone(), &MaterialInfo::default()).unwrap();
    let id = LayerId(r["layers"][0].as_u64().unwrap());
    let l = s.active().unwrap().doc.layer(id).unwrap();
    assert_eq!(l.opacity, 0.5);
    let LayerContent::Raster(surface) = &l.content else { panic!("pixels") };
    // Centred: (200 - 100) / 2 + 10 = 60, (100 - 50) / 2 + 5 = 30.
    let mut px = [[0.0f32; 4]; 1];
    surface.read_rgba_into(Rect::new(60, 30, 61, 31), &mut px);
    assert_eq!(px[0], [1.0, 0.0, 0.0, 1.0]);
    surface.read_rgba_into(Rect::new(59, 30, 60, 31), &mut px);
    assert_eq!(px[0][3], 0.0);

    m.layers[0].name = "Grid".into();
    let r = place(&mut s, "Grid", m, &MaterialInfo { tags: vec![], tiling: true }).unwrap();
    let id = LayerId(r["layers"][0].as_u64().unwrap());
    let doc = &s.active().unwrap().doc;
    let LayerContent::Fill(Fill::Pattern { id: pid, phase, .. }) = &doc.layer(id).unwrap().content else { panic!("pattern fill") };
    assert_eq!(*phase, (60.0, 30.0));
    let pattern = doc.patterns.iter().find(|p| &p.id == pid).unwrap();
    assert_eq!((pattern.width, pattern.height), (4, 2));
}

#[test]
fn several_layers_keep_their_order_with_the_first_on_top() {
    let mut s = session();
    let m = LayerMaterial { width: 10, height: 10, layers: vec![layer("Top"), layer("Bottom")], warnings: vec!["note".into()] };
    let r = place(&mut s, "Two", m, &MaterialInfo::default()).unwrap();
    assert_eq!(r["warnings"], json!(["note"]));
    let ids: Vec<LayerId> = r["layers"].as_array().unwrap().iter().map(|v| LayerId(v.as_u64().unwrap())).collect();
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.layer(ids[0]).unwrap().name, "Top");
    assert_eq!(s.active().unwrap().active_layer, Some(ids[0]));
    let bad = MaterialImage { width: 4, height: 4, rgba: vec![0; 3] };
    let m = LayerMaterial { width: 10, height: 10, layers: vec![MaterialLayer { image: Some(bad), ..layer("Bad") }], warnings: vec![] };
    let past = s.active().unwrap().history.past_len();
    assert!(place(&mut s, "Bad", m, &MaterialInfo::default()).is_err());
    assert_eq!(s.active().unwrap().history.past_len(), past);
}

/// The user's own downloaded "Isometric ruler and grid" (ASSETS 1758445); never bundled.
#[test]
#[ignore = "reads the local CLIP STUDIO user data folder (PRISM_TEST_CSP_ROOT or the default)"]
fn local_isometric_ruler_and_grid_import() {
    let mut s = session();
    let mut p = json!({});
    if let Some(root) = std::env::var_os("PRISM_TEST_CSP_ROOT") {
        p["root"] = json!(root.to_string_lossy());
    }
    let listed = s.execute("brush.cspLibrary.list", p.clone()).unwrap();
    let ruler = listed["layers"].as_array().unwrap().iter().find(|l| l["uuid"] == "2dedbf35ba-a641-3e9e-79ad-6950ad1a78").unwrap().clone();
    assert_eq!((ruler["status"].as_str(), ruler["ruler"].as_bool()), (Some("ready"), Some(true)));
    p["uuid"] = json!("2dedbf35ba-a641-3e9e-79ad-6950ad1a78");
    let r = s.execute(IMPORT, p.clone()).unwrap();
    assert_eq!((r["name"].as_str(), r["rulers"].as_u64()), (Some("Isometric ruler"), Some(1)));
    p["uuid"] = json!("8f65f2e32e-6d42-c3a8-ddbc-3cd24d4dd0");
    let r = s.execute(IMPORT, p).unwrap();
    assert_eq!(r["tiling"], true);
    let id = LayerId(r["layers"][0].as_u64().unwrap());
    assert!(matches!(s.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Fill(Fill::Pattern { .. })));
}
