use super::*;
use crate::sut::material::tests::tile_fixture_color;
use crate::sut::tests::{leaf, number, text};

// Synthetic layer materials built from the public SQLite spec; no CLIP STUDIO assets.
fn int(v: i64) -> Value {
    Value::Integer(v)
}

/// One table: name, columns, rows.
type Table = (&'static str, Vec<&'static str>, Vec<Vec<Value>>);

fn database(tables: Vec<Table>) -> Vec<u8> {
    const P: usize = 4096;
    let mut db = vec![0u8; (tables.len() + 1) * P];
    db[..16].copy_from_slice(b"SQLite format 3\0");
    db[16..18].copy_from_slice(&4096u16.to_be_bytes());
    db[56..60].copy_from_slice(&1u32.to_be_bytes());
    let schema: Vec<Vec<Value>> = tables
        .iter()
        .enumerate()
        .map(|(i, (name, columns, _))| {
            let sql = format!("CREATE TABLE {name}({})", columns.iter().map(|c| format!("{c} BLOB DEFAULT NULL")).collect::<Vec<_>>().join(", "));
            vec![text("table"), text(name), text(name), int(i as i64 + 2), text(&sql)]
        })
        .collect();
    leaf(&mut db, 1, &schema);
    for (i, (_, _, rows)) in tables.iter().enumerate() {
        leaf(&mut db, i + 2, rows);
    }
    db
}

fn vanish(id: i64, next: i64, angle: f64, guides: &[[f64; 2]]) -> Vec<Value> {
    let blob: Vec<u8> = guides.iter().flat_map(|g| [g[0].to_be_bytes(), g[1].to_be_bytes()].concat()).collect();
    vec![int(id), int(3), int(next), int(0x11), number(-8.4e6), number(-4.9e6), number(angle), int(guides.len() as i64), int(16), Value::Blob(blob)]
}

/// A 1280 × 720 material: a ruler layer (three parallel vanishing points, one concentric
/// circle ruler Prism skips) over a 2 × 2 colour layer.
fn tables() -> Vec<Table> {
    let (attr, blocks) = tile_fixture_color(2, 2, &[255, 0, 128, 255], &[[0, 0, 255], [0, 0, 0], [255, 0, 0], [0, 255, 0]]);
    vec![
        ("Canvas", vec!["MainId", "CanvasWidth", "CanvasHeight", "CanvasRootFolder"], vec![vec![int(1), int(1280), int(720), int(2)]]),
        (
            "Layer",
            vec![
                "MainId",
                "CanvasId",
                "LayerName",
                "LayerFolder",
                "LayerFirstChildIndex",
                "LayerNextIndex",
                "LayerVisibility",
                "LayerOpacity",
                "LayerOffsetX",
                "LayerOffsetY",
                "LayerRenderMipmap",
                "ResizableOriginalMipmap",
                "SpecialRulerManager",
                "RulerRange",
            ],
            vec![
                vec![int(2), int(0), text(""), int(1), int(3), int(0), int(1), int(256), int(0), int(0), int(0), int(0), int(0), Value::Null],
                vec![int(3), int(0), text("Isometric ruler"), int(0), int(0), int(4), int(5), int(256), int(0), int(0), int(0), int(0), int(1), int(0)],
                vec![int(4), int(0), text("Ink"), int(0), int(0), int(0), int(1), int(128), int(10), int(20), int(0), int(5), int(0), Value::Null],
            ],
        ),
        ("Mipmap", vec!["MainId", "CanvasId", "LayerId", "MipmapCount", "BaseMipmapInfo"], vec![vec![int(5), int(0), int(4), int(1), int(6)]]),
        ("MipmapInfo", vec!["MainId", "CanvasId", "LayerId", "ThisScale", "Offscreen", "NextIndex"], vec![vec![int(6), int(0), int(4), int(100), int(7), int(0)]]),
        ("Offscreen", vec!["MainId", "CanvasId", "LayerId", "Attribute", "BlockData"], vec![vec![int(7), int(0), int(4), Value::Blob(attr), Value::Blob(blocks)]]),
        (
            "SpecialRulerManager",
            vec!["MainId", "LayerId", "FirstPerspective", "FirstParallel", "FirstConcentricCircle"],
            vec![vec![int(1), int(3), int(1), int(0), int(1)]],
        ),
        (
            "RulerPerspective",
            vec!["MainId", "LayerId", "NextIndex", "FirstVanishIndex", "Flag", "PerspectiveType", "EyeLevelHandleX", "EyeLevelHandleY"],
            vec![vec![int(1), int(3), int(0), int(1), int(0x70), int(3), number(-2.5e6), number(-3.5e6)]],
        ),
        (
            "RulerVanishPoint",
            vec!["MainId", "LayerId", "NextIndex", "Flag", "VanishPointX", "VanishPointY", "ParallelAngle", "GuideNumber", "GuideDataSize", "Guide"],
            vec![vanish(1, 2, 30.6163181116636, &[[631.0, 566.0], [1056.9, 314.0]]), vanish(2, 3, 149.39613684716832, &[]), vanish(3, 0, 270.0, &[])],
        ),
    ]
}

fn read_tables(tables: Vec<Table>) -> Result<LayerMaterial> {
    read(&crate::csp_container::plain_fixture(&database(tables)), &prism_raster::Interrupt::NONE)
}

fn angles(m: &LayerMaterial) -> Vec<f64> {
    let Some(RulerKind::Perspective(p)) = m.layers[0].rulers.as_ref().map(|r| &r.rulers[0].kind) else { panic!("perspective ruler") };
    p.points
        .iter()
        .map(|vp| match vp.target {
            VanishTarget::Parallel { angle_deg } => angle_deg,
            VanishTarget::Point(_) => panic!("parallel point"),
        })
        .collect()
}

#[test]
fn rulers_and_pixels_are_read_in_layer_order() {
    let m = read_tables(tables()).unwrap();
    assert_eq!((m.width, m.height), (1280, 720));
    assert_eq!(m.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Isometric ruler", "Ink"]);
    let ruler = &m.layers[0];
    assert!(ruler.image.is_none(), "the ruler layer has no pixels");
    let set = ruler.rulers.as_ref().unwrap();
    assert!(set.shown);
    assert_eq!(set.rulers.len(), 1);
    assert_eq!(angles(&m), [30.6163181116636, 149.39613684716832, 270.0]);
    let RulerKind::Perspective(p) = &set.rulers[0].kind else { panic!("perspective") };
    assert_eq!(p.points[0].guides, [[631.0, 566.0], [1056.9, 314.0]]);
    assert_eq!(p.points[0].handle, [-8.4e6, -4.9e6]);
    assert_eq!(p.eye_level, Some([-2.5e6, -3.5e6]));
    assert!(m.warnings.iter().any(|w| w.contains("1 concentric circle ruler(s) skipped")), "{:?}", m.warnings);

    let ink = &m.layers[1];
    assert!(ink.rulers.is_none());
    assert_eq!((ink.offset, ink.opacity), ([10, 20], 0.5));
    let image = ink.image.as_ref().unwrap();
    assert_eq!((image.width, image.height), (2, 2));
    // Stored blue, green, red: the first pixel is opaque red, the second transparent.
    assert_eq!(&image.rgba[..4], &[255, 0, 0, 255]);
    assert_eq!(image.rgba[7], 0);
}

#[test]
fn a_broken_ruler_is_skipped_and_the_rest_still_imports() {
    // A vanishing-point chain that loops.
    let mut t = tables();
    t[7].2[2] = vanish(3, 1, 270.0, &[]);
    let m = read_tables(t).unwrap();
    assert!(m.layers[0].rulers.is_none());
    assert!(m.warnings.iter().any(|w| w.contains("perspective ruler was skipped")), "{:?}", m.warnings);
    assert!(m.layers[1].image.is_some());

    // A guide blob shorter than GuideNumber × 16.
    let mut t = tables();
    if let Value::Blob(b) = &mut t[7].2[0][9] {
        b.truncate(20);
    }
    assert!(read_tables(t).unwrap().layers[0].rulers.is_none());

    // A non-finite angle, a dangling head, and a missing vanishing-point table.
    let mut t = tables();
    t[7].2[1][6] = number(f64::NAN);
    assert!(read_tables(t).unwrap().layers[0].rulers.is_none());
    let mut t = tables();
    t[6].2[0][3] = int(99);
    assert!(read_tables(t).unwrap().layers[0].rulers.is_none());
    let mut t = tables();
    t.truncate(7);
    let m = read_tables(t).unwrap();
    assert!(m.layers[0].rulers.is_none() && !m.warnings.is_empty());
}

#[test]
fn hostile_structure_is_an_error_not_a_panic() {
    // Layers that loop, a missing root and an absurd canvas.
    let mut t = tables();
    t[1].2[2][5] = int(3);
    assert!(read_tables(t).is_err());
    let mut t = tables();
    t[0].2[0][3] = int(42);
    assert!(read_tables(t).is_err());
    let mut t = tables();
    t[0].2[0][1] = int(-5);
    assert!(read_tables(t).is_err());
    // An empty root folder.
    let mut t = tables();
    t[1].2[0][4] = int(0);
    assert!(read_tables(t).is_err());
    // Bad pixels lose the layer's pixels, not the material.
    let mut t = tables();
    t[4].2[0][4] = Value::Blob(vec![1, 2, 3]);
    let m = read_tables(t).unwrap();
    assert!(m.layers[1].image.is_none() && m.warnings.iter().any(|w| w.starts_with("Ink: pixels")));

    let f = crate::csp_container::plain_fixture(&database(tables()));
    for n in (0..f.len()).step_by(97) {
        let _ = read(&f[..n], &prism_raster::Interrupt::NONE);
    }
    assert!(read(b"not a material", &prism_raster::Interrupt::NONE).is_err());
}

#[test]
fn a_tile_drops_only_a_wholly_transparent_last_column_and_row() {
    let px = |a: u8| [0, 0, 0, a];
    // 3 × 2: the last column is clear, the last row is not.
    let im = MaterialImage { width: 3, height: 2, rgba: [px(255), px(9), px(0), px(77), px(0), px(0)].concat() };
    let t = im.tile();
    assert_eq!((t.width, t.height), (2, 2));
    assert_eq!(t.rgba, [px(255), px(9), px(77), px(0)].concat());
    // Both clear: both go.
    let im = MaterialImage { width: 2, height: 2, rgba: [px(255), px(0), px(0), px(0)].concat() };
    assert_eq!((im.tile().width, im.tile().height), (1, 1));
    let solid = MaterialImage { width: 2, height: 2, rgba: px(255).repeat(4) };
    assert_eq!(solid.tile(), solid);
    let dot = MaterialImage { width: 1, height: 1, rgba: px(0).to_vec() };
    assert_eq!(dot.tile(), dot, "never an empty tile");
    let short = MaterialImage { width: 4, height: 4, rgba: vec![0; 5] };
    assert_eq!(short.tile(), short, "inconsistent sizes are left alone");
}

#[test]
fn layer_data_xml_gives_tags_and_tiling() {
    let xml = r#"<infolist><info>
        <datalist key="systemtag"><data>OtherLayer</data><data>Ruler</data><data>SpecialRuler</data><data>PersRuler</data></datalist>
        <datalist key="isTiling"><data>0</data></datalist></info></infolist>"#;
    let info = parse_info(xml);
    assert_eq!(info.tags, ["OtherLayer", "Ruler", "SpecialRuler", "PersRuler"]);
    assert!(info.ruler() && !info.tiling);
    let grid = parse_info(r#"<datalist key="systemtag"><data>Resizable</data></datalist><datalist key="isTiling"><data>1</data></datalist>"#);
    assert!(!grid.ruler() && grid.tiling);
    assert_eq!(parse_info("<garbage <datalist key=\"systemtag\"><data>"), MaterialInfo::default());
}

/// The user's own downloaded "Isometric ruler and grid" (ASSETS 1758445); never bundled.
#[test]
#[ignore = "reads the local CLIP STUDIO material folder (PRISM_TEST_CSP_ROOT or the default)"]
fn local_isometric_ruler_and_grid() {
    let root = std::env::var_os("PRISM_TEST_CSP_ROOT").map(std::path::PathBuf::from).or_else(super::super::library::default_root).unwrap();
    let ctl = &prism_raster::Interrupt::NONE;
    let (name, m, info) = super::super::library::import_layer(&root, "2dedbf35ba-a641-3e9e-79ad-6950ad1a78", ctl).unwrap();
    assert_eq!(name, "Isometric ruler");
    assert!(info.ruler() && !info.tiling);
    assert_eq!((m.width, m.height, m.layers.len()), (1280, 720, 1));
    let a = angles(&m);
    assert!((a[0] - 30.616).abs() < 1e-3 && (a[1] - 149.396).abs() < 1e-3 && a[2] == 270.0, "{a:?}");
    let (name, grid, info) = super::super::library::import_layer(&root, "8f65f2e32e-6d42-c3a8-ddbc-3cd24d4dd0", ctl).unwrap();
    assert_eq!(name, "Isometric grid");
    assert!(info.tiling && grid.layers[0].image.is_some(), "{:?}", grid.warnings);
    let tile = grid.layers[0].image.as_ref().unwrap().tile();
    assert_eq!((grid.width, grid.height, tile.width, tile.height), (429, 259, 428, 258));
}

