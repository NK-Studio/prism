use super::*;
use crate::sut::tests::{leaf_with, number, reference_blob, text};
use prism_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
use prism_paint::TipShape;

// Synthetic CLIP STUDIO folders built from the public SQLite spec; no CELSYS data or assets.
const P: usize = 4096;
type Table<'a> = (&'a str, &'a str, Vec<Vec<Value>>);

fn database(tables: &[Table<'_>], utf16: bool) -> Vec<u8> {
    let mut db = vec![0; (tables.len() + 1) * P];
    db[..16].copy_from_slice(b"SQLite format 3\0");
    db[16..18].copy_from_slice(&(P as u16).to_be_bytes());
    db[56..60].copy_from_slice(&(if utf16 { 2u32 } else { 1 }).to_be_bytes());
    let schema: Vec<_> =
        tables.iter().enumerate().map(|(i, (name, sql, _))| vec![text("table"), text(name), text(name), number(i as f64 + 2.0), text(sql)]).collect();
    leaf_with(&mut db, 1, &schema, utf16);
    for (i, (_, _, rows)) in tables.iter().enumerate() {
        leaf_with(&mut db, i + 2, rows, utf16);
    }
    db
}
fn int(v: i64) -> Value {
    Value::Integer(v)
}
fn png() -> Vec<u8> {
    let im = Image::from_raw(2, 2, ChannelLayout::Rgba, SampleType::U8, vec![0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 0]).unwrap();
    prism_codecs::encode(&im, Format::Png, &EncodeOptions::default()).unwrap()
}

const TIP: &str = ".:01:02:tip:data:material_0.layer";
const MISSING: &str = ".:01:02:missing:data:material_0.layer";

struct Folder(PathBuf);
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn write(path: PathBuf, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}
fn folder(name: &str) -> Folder {
    let root = std::env::temp_dir().join(format!("prism-csp-library-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let modifier = |name: &str, uuid: &str, path_type: i64, path: &str| vec![Value::Null, text(name), text(uuid), int(path_type), text(path)];
    let meta = |id: i64, kind: &str, pending: i64| vec![Value::Null, int(id), text(kind), int(1), int(pending), int(0)];
    let cat = ".:01:02:collection";
    write(
        root.join(CATALOG),
        &database(
            &[
                (
                    "MaterialModifier",
                    "CREATE TABLE MaterialModifier(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, MaterialName TEXT DEFAULT NULL, MaterialUuid TEXT DEFAULT NULL, PathType INTEGER DEFAULT NULL, Path TEXT DEFAULT NULL)",
                    vec![
                        modifier("Collection", "", 0, cat),
                        modifier("Ready brush", "uuid-ready", 0, cat),
                        modifier("Cold brush", "uuid-cold", 0, cat),
                        modifier("Pending brush", "uuid-pending", 0, cat),
                        modifier("An image", "uuid-image", 0, cat),
                        modifier("茂み 브러시", "uuid-library", 0, cat),
                        modifier("Elsewhere", "uuid-remote", 1, "cloud"),
                        modifier("Broken brush", "uuid-broken", 0, cat),
                        modifier("Fill tool", "uuid-fill", 0, ".:03:04:fill"),
                        modifier("An action", "uuid-action", 0, cat),
                    ],
                ),
                (
                    "CatalogMetaData",
                    "CREATE TABLE CatalogMetaData(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, ModifierID INTEGER DEFAULT 0, MaterialKind TEXT DEFAULT NULL, Downloaded INTEGER DEFAULT 0, DownloadNotYet INTEGER DEFAULT 0, NewDownloading INTEGER DEFAULT 0)",
                    vec![
                        meta(1, "catalog", 0),
                        meta(2, "brush", 0),
                        meta(3, "brush", 0),
                        meta(4, "brush", 1),
                        meta(5, "image", 0),
                        meta(6, "brush", 0),
                        meta(7, "brush", 0),
                        meta(8, "brush", 0),
                        meta(9, "other_tool", 0),
                        meta(10, "auto_action", 0),
                    ],
                ),
                (
                    "Tag",
                    "CREATE TABLE Tag(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, Title TEXT DEFAULT NULL)",
                    vec![vec![Value::Null, text("lace")], vec![Value::Null, text(" crystal ")], vec![Value::Null, text("")], vec![Value::Null, text("塗り")]],
                ),
                (
                    "TableTagElementMap",
                    "CREATE TABLE TableTagElementMap(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, TagId INTEGER DEFAULT NULL, ElementId INTEGER DEFAULT NULL)",
                    vec![
                        // Collection: lace, 塗り. Ready brush: crystal, lace (a duplicate and an empty
                        // and an unknown tag are dropped). Fill tool: 塗り.
                        vec![Value::Null, int(1), int(1)],
                        vec![Value::Null, int(4), int(1)],
                        vec![Value::Null, int(2), int(2)],
                        vec![Value::Null, int(1), int(2)],
                        vec![Value::Null, int(2), int(2)],
                        vec![Value::Null, int(3), int(2)],
                        vec![Value::Null, int(99), int(2)],
                        vec![Value::Null, int(4), int(9)],
                    ],
                ),
            ],
            false,
        ),
    );
    let node_sql = |t: &str| {
        format!(
            "CREATE TABLE {t}(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, NodeName TEXT DEFAULT NULL, NodeMaterialUuid TEXT DEFAULT NULL, NodeVariantID INTEGER DEFAULT NULL)"
        )
    };
    let variant_sql = |t: &str| {
        format!(
            "CREATE TABLE {t}(VariantID INTEGER DEFAULT NULL, BrushSize REAL DEFAULT NULL, BrushUsePatternImage INTEGER DEFAULT NULL, BrushPatternImageArray BLOB DEFAULT NULL)"
        )
    };
    let (download_nodes, download_variants) = (node_sql("ToolVariantNode"), variant_sql("ToolVariant"));
    write(
        root.join("CLIPStudioPaintVer1_5_0").join(DOWNLOAD_CACHE),
        &database(
            &[
                (
                    "ToolVariantNode",
                    &download_nodes,
                    vec![
                        vec![Value::Null, text("Ready brush"), text("uuid-ready"), int(10)],
                        vec![Value::Null, text("Broken brush"), text("uuid-broken"), int(11)],
                    ],
                ),
                (
                    "ToolVariant",
                    &download_variants,
                    vec![
                        vec![int(9), number(99.0), int(0), Value::Null],
                        vec![int(10), number(12.0), int(1), Value::Blob(reference_blob(&[TIP]))],
                        vec![int(11), number(5.0), int(1), Value::Blob(reference_blob(&[MISSING]))],
                    ],
                ),
            ],
            false,
        ),
    );
    let (library_nodes, library_variants) = (node_sql("Node"), variant_sql("Variant"));
    write(
        root.join("CLIPStudioPaintVer1_5_0").join(TOOL_LIBRARY),
        &database(
            &[
                ("Node", &library_nodes, vec![vec![Value::Null, text("茂み 브러시"), text("uuid-library"), int(20)]]),
                ("Variant", &library_variants, vec![vec![int(20), number(30.0), int(0), Value::Null]]),
            ],
            true,
        ),
    );
    let tip = root.join(MATERIALS).join("01").join("02").join("tip");
    write(tip.join("data").join("material_0.layer"), b"not a supported C2F layer");
    write(tip.join("thumbnail").join("thumbnail.png"), &png());
    Folder(root)
}
fn files(root: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
    for entry in std::fs::read_dir(root).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push((path.clone(), std::fs::read(path).unwrap()));
        }
    }
}

#[test]
fn rowid_aliases_and_utf16_text_are_read() {
    let db = database(
        &[(
            "T",
            "CREATE TABLE T(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, Name TEXT DEFAULT NULL)",
            vec![vec![Value::Null, text("茂み1")], vec![Value::Null, text("세모")]],
        )],
        true,
    );
    let mut rows = Database::new(&db).unwrap().table("T").unwrap();
    rows.sort_by_key(|r| int_of(r, "_PW_ID"));
    assert_eq!(rows.iter().map(|r| int_of(r, "_PW_ID")).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(rows[0].get("Name").and_then(Value::text), Some("茂み1"));
    let mut big_endian = db.clone();
    big_endian[56..60].copy_from_slice(&3u32.to_be_bytes());
    assert!(Database::new(&big_endian).is_err());
}
#[test]
fn rows_written_before_added_columns_read_declared_defaults() {
    let sql = "CREATE TABLE T(A INTEGER DEFAULT 7, B TEXT DEFAULT 'it''s', C REAL DEFAULT -1.5, D BLOB)";
    let db = database(&[("T", sql, vec![vec![int(1)], vec![int(2), text("set")]])], false);
    let mut rows = Database::new(&db).unwrap().table("T").unwrap();
    rows.sort_by_key(|r| int_of(r, "A"));
    assert_eq!(rows[0].get("B").and_then(Value::text), Some("it's"));
    assert_eq!(rows[0].get("C").and_then(Value::number), Some(-1.5));
    assert!(matches!(rows[0].get("D"), Some(Value::Null)));
    assert_eq!(rows[1].get("B").and_then(Value::text), Some("set"));
    for bad in ["CREATE TABLE T(A INTEGER, B TEXT DEFAULT (1+2))", "CREATE TABLE T(A INTEGER, B TEXT DEFAULT CURRENT_TIME)"] {
        assert!(Database::new(&database(&[("T", bad, vec![vec![int(1)]])], false)).unwrap().table("T").is_err());
    }
    let long = database(&[("T", "CREATE TABLE T(A INTEGER)", vec![vec![int(1), int(2)]])], false);
    assert!(Database::new(&long).unwrap().table("T").is_err());
}
fn int_of(r: &Row, k: &str) -> i64 {
    r.get(k).and_then(Value::integer).unwrap()
}

#[test]
fn lists_downloaded_brushes_with_their_readiness() {
    let f = folder("list");
    let lib = list(&f.0).unwrap();
    let got: Vec<_> = lib.brushes.iter().map(|b| (b.name.as_str(), b.catalog.as_str(), b.status)).collect();
    assert_eq!(lib.brushes.iter().filter(|b| b.kind == "other_tool").map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Fill tool"]);
    assert_eq!(
        got,
        [
            ("Ready brush", "Collection", Status::Ready(CacheSource::DownloadCache)),
            ("Cold brush", "Collection", Status::NoCache),
            ("Pending brush", "Collection", Status::Downloading),
            ("茂み 브러시", "Collection", Status::Ready(CacheSource::ToolLibrary)),
            ("Elsewhere", "", Status::UnsupportedLocation),
            ("Broken brush", "Collection", Status::Ready(CacheSource::DownloadCache)),
            ("Fill tool", "", Status::NoCache),
        ]
    );
    assert!(lib.notices.iter().any(|n| n == LICENSE_NOTICE));
    assert!(!lib.notices.iter().any(|n| n.contains("running")));
    for b in &lib.brushes {
        assert_eq!(b.status.guidance().is_none(), matches!(b.status, Status::Ready(_)));
    }
}

#[test]
fn previews_of_materials_and_collections_resolve_inside_the_material_folder() {
    let f = folder("thumbnails");
    let base = f.0.join(MATERIALS);
    write(base.join("01/02/collection/thumbnail/catalogThumb.png"), &png());
    write(base.join("01/02/collection/uuid-ready/thumbnail/thumbnail.png"), &png());
    write(base.join("03/04/fill/thumbnail/thumbnail.png"), &png());
    let lib = list(&f.0).unwrap();
    let thumb = |uuid: &str| lib.brushes.iter().find(|b| b.uuid == uuid).unwrap().thumbnail.clone();
    // Inside a collection the material has a folder of its own; a loose one is the folder.
    assert_eq!(thumbnail_bytes(&thumb("uuid-ready").unwrap()).unwrap(), png());
    assert!(thumb("uuid-fill").is_some());
    assert_eq!(thumb("uuid-cold"), None);
    assert_eq!(thumb("uuid-remote"), None, "never outside the local material folder");
    assert_eq!(lib.catalogs.len(), 1);
    assert_eq!(lib.catalogs[0].name, "Collection");
    assert!(lib.catalogs[0].thumbnail.as_ref().is_some_and(|p| p.ends_with("catalogThumb.png")));
    // Oversized images are refused instead of read whole.
    let big = base.join("01/02/collection/uuid-cold/thumbnail/thumbnail.png");
    write(big.clone(), &vec![0; THUMBNAIL_LIMIT as usize + 1]);
    assert!(thumbnail_bytes(&big).is_err());
}

#[test]
fn user_tags_of_materials_and_collections_are_listed() {
    let root = folder("tags");
    let lib = list(&root.0).unwrap();
    let tags = |uuid: &str| lib.brushes.iter().find(|b| b.uuid == uuid).unwrap().tags.clone();
    assert_eq!(tags("uuid-ready"), ["crystal", "lace"]);
    assert_eq!(tags("uuid-fill"), ["塗り"]);
    assert!(tags("uuid-cold").is_empty());
    assert_eq!(lib.catalogs[0].tags, ["lace", "塗り"]);
}

#[test]
fn catalogs_without_tag_tables_list_untagged() {
    let root = folder("no-tags");
    let modifiers = "CREATE TABLE MaterialModifier(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, MaterialName TEXT DEFAULT NULL, MaterialUuid TEXT DEFAULT NULL, PathType INTEGER DEFAULT NULL, Path TEXT DEFAULT NULL)";
    let meta = "CREATE TABLE CatalogMetaData(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, ModifierID INTEGER DEFAULT 0, MaterialKind TEXT DEFAULT NULL, Downloaded INTEGER DEFAULT 0, DownloadNotYet INTEGER DEFAULT 0, NewDownloading INTEGER DEFAULT 0)";
    write(
        root.0.join(CATALOG),
        &database(
            &[
                ("MaterialModifier", modifiers, vec![vec![Value::Null, text("Brush"), text("uuid-a"), int(0), text(".:01:02:a")]]),
                ("CatalogMetaData", meta, vec![vec![Value::Null, int(1), text("brush"), int(1), int(0), int(0)]]),
            ],
            false,
        ),
    );
    let lib = list(&root.0).unwrap();
    assert_eq!(lib.brushes.len(), 1);
    assert!(lib.brushes[0].tags.is_empty());
}

#[test]
fn downloaded_container_is_ready_without_paint_cache_for_both_folder_layouts() {
    let f = folder("raw-container");
    let bytes = crate::sut::tests::tool_fixture();
    for rel in ["01/02/collection/uuid-cold/data/material_0.tool", "03/04/fill/data/material_0.tool"] {
        write(f.0.join(MATERIALS).join(rel), &bytes);
    }
    let lib = list(&f.0).unwrap();
    for uuid in ["uuid-cold", "uuid-fill"] {
        assert_eq!(lib.brushes.iter().find(|b| b.uuid == uuid).unwrap().status, Status::Ready(CacheSource::MaterialFile));
        let (_, imp) = import(&f.0, uuid, None, &prism_raster::Interrupt::NONE).unwrap();
        assert_eq!(imp.presets.len(), 1);
    }
}

#[test]
fn imports_from_the_caches_without_touching_any_file() {
    let f = folder("import");
    let mut before = Vec::new();
    files(&f.0, &mut before);
    let (group, imp) = import(&f.0, "uuid-ready", None, &prism_raster::Interrupt::NONE).unwrap();
    assert_eq!(group, "Collection");
    let p = &imp.presets[0];
    assert_eq!((p.name.as_str(), p.brush.size), ("Ready brush", 12.0));
    let TipShape::Sampled(tip) = &p.brush.tip else { panic!("tip expected") };
    assert_eq!((tip.width, tip.height), (2, 2));
    assert!(imp.notes.iter().any(|w| w.contains("download cache")));
    assert!(imp.warnings.iter().any(|w| w.contains("using its thumbnail")), "{:?}", imp.warnings);

    let (_, imp) = import(&f.0, "uuid-library", Some("Mine"), &prism_raster::Interrupt::NONE).unwrap();
    assert_eq!((imp.presets[0].group.as_str(), imp.presets[0].brush.size), ("Mine", 30.0));
    assert!(imp.notes.iter().any(|w| w.contains("sub-tool library")));

    let mut after = Vec::new();
    files(&f.0, &mut after);
    assert_eq!(before, after);
}

#[test]
fn brushes_that_are_not_ready_explain_what_to_do() {
    let f = folder("not-ready");
    let none = &prism_raster::Interrupt::NONE;
    assert!(import(&f.0, "uuid-cold", None, none).unwrap_err().contains("has not prepared"));
    assert!(import(&f.0, "uuid-pending", None, none).unwrap_err().contains("not finished"));
    assert!(import(&f.0, "uuid-remote", None, none).unwrap_err().contains("not stored"));
    assert!(import(&f.0, "uuid-image", None, none).is_err());
    assert!(import(&f.0, "", None, none).is_err());
    assert!(import(&f.0, "uuid-broken", None, none).unwrap_err().contains("not available locally"));
}

#[test]
fn unsaved_database_changes_are_reported() {
    let f = folder("wal");
    let mut wal = f.0.join(CATALOG).into_os_string();
    wal.push("-wal");
    write(PathBuf::from(wal), b"pending");
    assert!(list(&f.0).unwrap().notices.iter().any(|n| n.contains("running")));
}

#[test]
fn missing_or_corrupt_folders_fail_without_panicking() {
    let f = folder("corrupt");
    assert!(list(&f.0.join("absent")).is_err());
    let empty = f.0.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(list(&empty).unwrap_err().contains("catalog"));
    let catalog = std::fs::read(f.0.join(CATALOG)).unwrap();
    for cut in [0, 16, 100, P, catalog.len() - 1] {
        std::fs::write(f.0.join(CATALOG), &catalog[..cut]).unwrap();
        assert!(list(&f.0).is_err());
    }
    std::fs::write(f.0.join(CATALOG), &catalog).unwrap();
    // A corrupt sub-tool cache degrades to a notice, never an error or panic.
    std::fs::write(f.0.join("CLIPStudioPaintVer1_5_0").join(DOWNLOAD_CACHE), b"garbage").unwrap();
    let lib = list(&f.0).unwrap();
    assert!(lib.notices.iter().any(|n| n.contains("could not be read")));
    assert_eq!(lib.brushes[0].status, Status::NoCache);
}

#[test]
fn material_references_cannot_leave_the_material_folder() {
    let f = folder("escape");
    let root = std::fs::canonicalize(f.0.join(MATERIALS)).unwrap();
    write(f.0.join("secret.layer"), b"outside");
    for bad in
        ["", ".", "x:01:02", ".:..:..:secret.layer", ".:01/02:tip", ".:01\\02:tip", ".:01::tip", ".:a", ".:01:02:tip:data", &format!(".{}", ":a".repeat(20))]
    {
        assert!(resolve(&root, bad).is_err(), "{bad:?}");
    }
    assert!(resolve(&root, TIP).is_ok());
    assert!(material_file(&root, MISSING).is_err());
}

#[test]
fn packaged_materials_are_valid_ustar() {
    let tar = ustar(&[("data/material_0.layer".into(), vec![7; 700]), ("thumbnail/thumbnail.png".into(), png())]);
    assert_eq!(tar.len() % 512, 0);
    assert_eq!(&tar[257..262], b"ustar");
    let sum: u32 = tar[..512].iter().enumerate().map(|(i, b)| if (148..156).contains(&i) { 32 } else { u32::from(*b) }).sum();
    assert_eq!(u32::from_str_radix(std::str::from_utf8(&tar[148..154]).unwrap(), 8).unwrap(), sum);
}

/// Local, user-owned CLIP STUDIO data; never bundled or committed. Read-only: verifies the
/// catalog and both sub-tool databases are byte-identical afterwards.
#[test]
#[ignore = "reads the local CLIP STUDIO user data folder (PRISM_TEST_CSP_ROOT or the default)"]
fn local_library_lists_and_imports_every_ready_brush() {
    let root = std::env::var_os("PRISM_TEST_CSP_ROOT").map(PathBuf::from).or_else(default_root).expect("no CLIP STUDIO user data folder");
    let paint = paint_folder(&root).expect("no CLIP STUDIO PAINT folder");
    let watched = [root.join(CATALOG), paint.join(DOWNLOAD_CACHE), paint.join(TOOL_LIBRARY)];
    let before: Vec<_> = watched.iter().map(|p| std::fs::read(p).ok()).collect();
    let lib = list(&root).unwrap();
    for b in &lib.brushes {
        match import(&root, &b.uuid, None, &prism_raster::Interrupt::NONE) {
            Ok((group, imp)) => {
                let thumbs = imp.warnings.iter().filter(|w| w.contains("thumbnail")).count();
                let tips = match &imp.presets[0].brush.tip {
                    TipShape::Round => 0,
                    TipShape::Sampled(_) => 1,
                    TipShape::Sequence(t) => t.len(),
                };
                println!("ok   {} [{group}] {:?} tips={tips} thumbnail-fallbacks={thumbs} warnings={}", b.name, b.status, imp.warnings.len());
            }
            Err(e) => println!("skip {} {:?}: {e}", b.name, b.status),
        }
    }
    println!("notices: {:#?}", lib.notices);
    let after: Vec<_> = watched.iter().map(|p| std::fs::read(p).ok()).collect();
    assert!(before == after, "a CLIP STUDIO database changed");
}
