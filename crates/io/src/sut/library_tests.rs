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

#[test]
fn folders_favourites_and_own_materials_are_listed_but_installed_defaults_are_not() {
    let root = folder("folders");
    let modifiers = "CREATE TABLE MaterialModifier(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, MaterialName TEXT DEFAULT NULL, MaterialUuid TEXT DEFAULT NULL, PathType INTEGER DEFAULT NULL, Path TEXT DEFAULT NULL)";
    let meta = "CREATE TABLE CatalogMetaData(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, ModifierID INTEGER DEFAULT 0, MaterialKind TEXT DEFAULT NULL, Downloaded INTEGER DEFAULT 0, DownloadNotYet INTEGER DEFAULT 0, NewDownloading INTEGER DEFAULT 0, Favorite INTEGER DEFAULT 0)";
    let m = |name: &str, uuid: &str, path: &str| vec![Value::Null, text(name), text(uuid), int(0), text(path)];
    let d = |id: i64, kind: &str, downloaded: i64, favorite: i64| vec![Value::Null, int(id), text(kind), int(downloaded), int(0), int(0), int(favorite)];
    let long = "x".repeat(400);
    write(
        root.0.join(CATALOG),
        &database(
            &[
                (
                    "MaterialModifier",
                    modifiers,
                    vec![m("Ruler", "uuid-ruler", ".:05:06:ruler"), m("Mine", "uuid-own", ".:07:08:uuid-own"), m("Default", "uuid-default", ".:Install:Paint001:uuid-default")],
                ),
                ("CatalogMetaData", meta, vec![d(1, "image", 1, 0), d(2, "image", 0, 1), d(3, "image", 0, 0)]),
                (
                    "FolderTag",
                    "CREATE TABLE FolderTag(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, Title TEXT DEFAULT NULL)",
                    vec![vec![Value::Null, text("ダウンロード:Isometric ruler and grid-1758445:Isometric ruler and grid")], vec![Value::Null, text("自作ブラシ:茂み")], vec![Value::Null, text(&long)]],
                ),
                (
                    "TableFolderTagElementMap",
                    "CREATE TABLE TableFolderTagElementMap(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, TagId INTEGER DEFAULT NULL, ElementId INTEGER DEFAULT NULL)",
                    vec![vec![Value::Null, int(1), int(1)], vec![Value::Null, int(2), int(2)], vec![Value::Null, int(3), int(2)], vec![Value::Null, int(2), int(3)]],
                ),
            ],
            false,
        ),
    );
    let lib = list(&root.0).unwrap();
    let names: Vec<&str> = lib.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Ruler", "Mine"], "installed defaults have no pixels locally");
    assert_eq!(lib.layers[0].folders, ["ダウンロード:Isometric ruler and grid-1758445:Isometric ruler and grid"]);
    assert!(!lib.layers[0].favorite && lib.layers[1].favorite);
    assert_eq!(lib.layers[1].folders[0], "自作ブラシ:茂み");
    assert_eq!(lib.layers[1].folders[1].chars().count(), MAX_FOLDER_CHARS);
    assert!(lib.layers.iter().all(|l| l.status == Status::Missing), "no files were written");
}

#[test]
fn auto_actions_are_listed_and_read_by_id() {
    let f = folder("actions");
    let lib = list(&f.0).unwrap();
    let got: Vec<_> = lib.actions.iter().map(|a| (a.name.as_str(), a.catalog.as_str(), a.status)).collect();
    assert_eq!(got, [("An action", "Collection", Status::Missing)]);
    assert!(action_bytes(&f.0, "uuid-action").unwrap_err().contains("missing"));
    let file = f.0.join(MATERIALS).join("01").join("02").join("collection").join("uuid-action").join("data").join("material_0.laf");
    write(file, b"laf bytes");
    let lib = list(&f.0).unwrap();
    assert_eq!(lib.actions[0].status, Status::Ready(CacheSource::MaterialFile));
    assert_eq!(action_bytes(&f.0, "uuid-action").unwrap(), ("An action".to_string(), b"laf bytes".to_vec()));
    // Only auto actions: a brush or image id is not one.
    for uuid in ["uuid-ready", "uuid-image", "", "../uuid-action"] {
        assert!(action_bytes(&f.0, uuid).is_err(), "{uuid}");
    }
    assert!(lib.brushes.iter().all(|b| b.kind != "auto_action"));
}

/// A gradient blob (16-byte header): opaque `(rgb, flags, position)` stops, no curves.
fn gradient(stops: &[([u32; 3], u32, u32)]) -> Vec<u8> {
    let mut b: Vec<u8> = [16u32, 28, stops.len() as u32, 16].into_iter().flat_map(u32::to_be_bytes).collect();
    for (rgb, flags, pos) in stops {
        b.extend([rgb[0], rgb[1], rgb[2], u32::MAX, *flags, *pos, 0].into_iter().flat_map(u32::to_be_bytes));
    }
    b
}

#[test]
fn a_gradient_sub_tool_reads_its_node_names_and_gradation_data() {
    use crate::sut::gradient_material::read;
    let red_white = gradient(&[([u32::MAX, 0, 0], 0, 0), ([u32::MAX; 3], 0, 0x8000)]);
    let inner = database(
        &[
            ("Node", "CREATE TABLE Node (NodeName TEXT, NodeVariantID INTEGER)", vec![vec![text("Warm"), int(7)], vec![text("Pen"), int(8)]]),
            ("Variant", "CREATE TABLE Variant (VariantID INTEGER, GradationData BLOB)", vec![vec![int(7), Value::Blob(red_white)], vec![int(8), Value::Null]]),
        ],
        false,
    );
    let outer = database(&[("Material", "CREATE TABLE Material (MaterialName TEXT, MaterialToolVariantManager BLOB)", vec![vec![text("Warm"), Value::Blob(inner.clone())]])], false);
    let m = read(&crate::csp_container::plain_fixture(&outer), &prism_raster::Interrupt::NONE).unwrap();
    assert_eq!(m.groups.len(), 1);
    assert_eq!(m.groups[0].name, None);
    assert_eq!(m.groups[0].gradients.len(), 1);
    assert_eq!(m.groups[0].gradients[0].0, "Warm");
    assert_eq!(m.groups[0].gradients[0].1.stops[0].color, prism_paint::csp_gradient::CspStopColor::Rgb([1.0, 0.0, 0.0]));
    assert!(m.warnings.iter().any(|w| w.contains("Pen")), "a variant without gradient data is reported");
    // Malformed inputs are errors, never panics.
    let empty = database(&[("Material", "CREATE TABLE Material (MaterialName TEXT, MaterialToolVariantManager BLOB)", vec![])], false);
    let broken = database(&[("Variant", "CREATE TABLE Variant (VariantID INTEGER, GradationData BLOB)", vec![vec![int(7), Value::Blob(vec![0, 0, 0, 16])]])], false);
    for bytes in [empty, outer[..100].to_vec(), b"not a material".to_vec(), Vec::new(), broken] {
        assert!(read(&bytes, &prism_raster::Interrupt::NONE).is_err());
    }
}

#[test]
fn a_colour_set_reads_its_colours_in_palette_order() {
    use crate::sut::color_set::read;
    const SQL: &str = "CREATE TABLE colorsetmaterial(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, colorsetitemname TEXT DEFAULT NULL, colorsetitemred INTEGER DEFAULT NULL, colorsetitemgreen INTEGER DEFAULT NULL, colorsetitemblue INTEGER DEFAULT NULL, colorsetitemtransparent INTEGER DEFAULT NULL)";
    let full = i64::from(u32::MAX);
    let row = |id: i64, name: &str, rgb: [i64; 3], transparent: i64| vec![int(id), text(name), int(rgb[0]), int(rgb[1]), int(rgb[2]), int(transparent)];
    let db = database(
        &[(
            "colorsetmaterial",
            SQL,
            vec![
                row(2, "", [0, full, 0], 0),
                row(1, "Sky", [0xDCDC_DCDC, 0, full], 0),
                row(3, "", [0, 0, 0], 1),
                row(4, "", [-5, 0, 0], 0),
            ],
        )],
        false,
    );
    let set = read(&crate::csp_container::plain_fixture(&db), &prism_raster::Interrupt::NONE).unwrap();
    assert_eq!(set.colors.len(), 2, "{set:?}");
    assert_eq!(set.colors[0].name, "Sky", "palette order is _PW_ID");
    assert!((set.colors[0].rgb[0] - 220.0 / 255.0).abs() < 1e-6);
    assert_eq!(set.colors[1].rgb, [0.0, 1.0, 0.0]);
    assert_eq!(set.transparent, 1);
    assert_eq!(set.warnings.len(), 2, "transparent and unreadable entries are reported: {:?}", set.warnings);
    assert_eq!(read(&db, &prism_raster::Interrupt::NONE).unwrap(), set, "plain SQLite reads too");
    let empty = database(&[("colorsetmaterial", SQL, vec![row(1, "", [0, 0, 0], 1)])], false);
    let other = database(&[("Material", "CREATE TABLE Material (MaterialName TEXT)", vec![vec![text("x")]])], false);
    for bytes in [empty, other, db[..100].to_vec(), b"not a colour set".to_vec(), Vec::new()] {
        assert!(read(&bytes, &prism_raster::Interrupt::NONE).is_err());
    }
}

#[test]
fn a_pose_material_reads_its_figure() {
    use crate::sut::pose::read;
    let pep = r#"<?xml version="1.0" encoding="UTF-8"?><celsysdocument><pose version="1.0.14"><node name="root_group_" rotquat="1 0 0 0" trans="1 2 3"><node name="hips_bb_" rotquat="1 0 0 0" trans="0 96 0"/></node></pose></celsysdocument>"#;
    let db = database(
        &[
            ("Material", "CREATE TABLE Material(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, MaterialName TEXT DEFAULT NULL)", vec![vec![int(1), text("Pose A")]]),
            ("File", "CREATE TABLE File(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, MainId INTEGER DEFAULT NULL, FileName TEXT DEFAULT NULL, FileData BLOB DEFAULT NULL)", vec![vec![int(1), int(1), text("a.pep"), Value::Blob(pep.as_bytes().to_vec())]]),
        ],
        false,
    );
    let f = read(&crate::csp_container::plain_fixture(&db), "fallback", &prism_raster::Interrupt::NONE).unwrap();
    assert_eq!(f.name, "Pose A");
    assert_eq!(f.root.offset, [1.0, 2.0, 3.0]);
    assert_eq!(f.root.find("hips_bb_").unwrap().offset, [0.0, 96.0, 0.0]);
    let no_file = database(&[("Material", "CREATE TABLE Material(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, MaterialName TEXT DEFAULT NULL)", vec![vec![int(1), text("x")]])], false);
    assert!(read(&no_file, "x", &prism_raster::Interrupt::NONE).is_err());
}

/// Render every local pose collection into one PNG per collection, with Prism's standing figure
/// for comparison: `PRISM_LOOK_OUT=<dir> cargo test -p prism-io --lib local_poses -- --ignored`.
#[test]
#[ignore = "reads the local CLIP STUDIO folder and writes PNGs"]
fn local_poses_render() {
    use prism_algo::figure3d::{RenderOptions, frame, render};
    use prism_doc::figure::{FigureScene, standing_figure};
    let out = std::path::PathBuf::from(std::env::var_os("PRISM_LOOK_OUT").unwrap_or_else(|| "target/csp-poses".into()));
    std::fs::create_dir_all(&out).unwrap();
    let root = std::env::var_os("PRISM_TEST_CSP_ROOT").map(std::path::PathBuf::from).or_else(default_root).unwrap();
    let lib = list(&root).unwrap();
    let mut by_catalog: Vec<(String, Vec<prism_doc::figure::Figure>)> = Vec::new();
    for p in &lib.poses {
        let (catalog, figure) = import_pose(&root, &p.uuid, &prism_raster::Interrupt::NONE).unwrap_or_else(|e| panic!("{}: {e}", p.name));
        eprintln!("{} [{}]: {} joints", p.name, catalog, figure.root.walk().len());
        match by_catalog.iter_mut().find(|(c, _)| *c == catalog) {
            Some((_, v)) => v.push(figure),
            None => by_catalog.push((catalog, vec![figure])),
        }
    }
    // The first collection from several sides, to compare with CLIP STUDIO's thumbnail.
    if let Some((_, figures)) = by_catalog.first().cloned() {
        for f in &figures {
            let posed = prism_algo::figure3d::skeleton::pose(f);
            let hips = prism_algo::figure3d::skeleton::find(&posed, "hips_bb_").map(|h| h.pos);
            eprintln!("{}: facing {:?} hips {:?}", f.name, prism_algo::figure3d::facing(f), hips);
        }
        for (k, yaw) in [-60.0f64, -30.0, 30.0, 60.0, 120.0, 180.0].into_iter().enumerate() {
            let mut scene = FigureScene { figures: figures.clone(), ..Default::default() };
            frame(&mut scene, [300.0, 225.0], 400.0, true).unwrap();
            prism_algo::figure3d::orbit(&mut scene, yaw, 10.0);
            frame(&mut scene, [300.0, 225.0], 400.0, false).unwrap();
            let s = render(&scene, prism_color::PixelFormat::RGBA8, prism_geom::Rect::new(0, 0, 600, 450), RenderOptions { draft: true }, &prism_raster::Interrupt::NONE).unwrap();
            let data: Vec<u8> = s.read_region(prism_geom::Rect::new(0, 0, 600, 450)).iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
            let im = prism_codecs::Image::from_raw(600, 450, prism_codecs::ChannelLayout::Rgba, prism_codecs::SampleType::U8, data).unwrap();
            std::fs::write(out.join(format!("view{k}.png")), prism_codecs::encode(&im, prism_codecs::Format::Png, &prism_codecs::EncodeOptions::default()).unwrap()).unwrap();
        }
    }
    by_catalog.push(("standing".into(), vec![standing_figure()]));
    for (i, (name, figures)) in by_catalog.into_iter().enumerate() {
        let mut scene = FigureScene { figures, ..Default::default() };
        frame(&mut scene, [300.0, 225.0], 400.0, true).unwrap();
        let s = render(&scene, prism_color::PixelFormat::RGBA8, prism_geom::Rect::new(0, 0, 600, 450), RenderOptions::default(), &prism_raster::Interrupt::NONE).unwrap();
        let r = prism_geom::Rect::new(0, 0, 600, 450);
        let data: Vec<u8> = s.read_region(r).iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
        let im = prism_codecs::Image::from_raw(600, 450, prism_codecs::ChannelLayout::Rgba, prism_codecs::SampleType::U8, data).unwrap();
        let png = prism_codecs::encode(&im, prism_codecs::Format::Png, &prism_codecs::EncodeOptions::default()).unwrap();
        std::fs::write(out.join(format!("pose{i}.png")), png).unwrap();
        eprintln!("pose{i}.png = {name}");
    }
}

/// Local look at Prism's hand setup shapes (no CLIP STUDIO data): each shape on its own left hand,
/// fingers up, from the back, the thumb side and three-quarters, one strip per shape in
/// `$PRISM_LOOK_OUT` (default `target/hand-shapes`).
#[test]
#[ignore = "writes PNGs to look at"]
fn hand_shapes_render() {
    use prism_algo::figure3d::hand::{Closed, HandSetup, Side, set_hand};
    use prism_doc::figure::standing_figure;
    let out = std::path::PathBuf::from(std::env::var_os("PRISM_LOOK_OUT").unwrap_or_else(|| "target/hand-shapes".into()));
    std::fs::create_dir_all(&out).unwrap();
    let shapes: Vec<(&str, [f64; 3], Closed)> = vec![
        ("flat", [1.0, 0.0, 0.0], Closed::Fist),
        ("spread", [0.0, 1.0, 0.0], Closed::Fist),
        ("fist", [0.0, 0.0, 1.0], Closed::Fist),
        ("catPaw", [0.0, 0.0, 1.0], Closed::CatPaw),
        ("tube", [0.0, 0.0, 1.0], Closed::Tube),
        ("grip", [0.0, 0.0, 1.0], Closed::Grip),
        ("middle", [1.0, 1.0, 1.0], Closed::Fist),
    ];
    for (name, weights, closed) in shapes {
        let mut f = standing_figure();
        set_hand(&mut f, Side::Left, &HandSetup { weights, closed, locked: [false; 5] }).unwrap();
        hand_strip(&f, "left", &out.join(format!("{name}.png")));
    }
}

/// One hand of `f` (`side` "left" or "right"), fingers up, from three sides, as a PNG strip.
fn hand_strip(f: &prism_doc::figure::Figure, side: &str, path: &std::path::Path) {
    use prism_algo::figure3d::{RenderOptions, frame, orbit, render};
    use prism_doc::figure::{Figure, FigureScene, Joint};
    let (w, h) = (300u32, 300u32);
    // The hand under a short forearm and a root that turns it fingers up (the mannequin draws no
    // bone off the tree root).
    let mut hand = f.root.find(&format!("{side}hand_bb_")).unwrap().clone();
    hand.offset = [0.0, -4.0, 0.0];
    let arm = Joint { name: format!("{side}forearm_bb_"), rotation: [1.0, 0.0, 0.0, 0.0], offset: [0.0; 3], children: vec![hand] };
    let root = Joint { name: "root_group_".into(), rotation: [0.0, 0.0, 0.0, 1.0], offset: [0.0; 3], children: vec![arm] };
    let figure = Figure { name: String::new(), source: None, root };
    let mut strip = vec![0u8; (w * 3 * h * 4) as usize];
    for (k, yaw) in [0.0f64, 90.0, 45.0].into_iter().enumerate() {
        let mut scene = FigureScene { figures: vec![figure.clone()], ..Default::default() };
        frame(&mut scene, [150.0, 150.0], 260.0, false).unwrap();
        orbit(&mut scene, yaw, 0.0);
        frame(&mut scene, [150.0, 150.0], 260.0, false).unwrap();
        let r = prism_geom::Rect::new(0, 0, w as i32, h as i32);
        let s = render(&scene, prism_color::PixelFormat::RGBA8, r, RenderOptions::default(), &prism_raster::Interrupt::NONE).unwrap();
        let px: Vec<u8> = s.read_region(r).iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
        for y in 0..h as usize {
            for x in 0..w as usize {
                let (src, dst) = ((y * w as usize + x) * 4, (y * 3 * w as usize + k * w as usize + x) * 4);
                // On white, so the outline reads like CLIP STUDIO's canvas.
                let a = f32::from(px[src + 3]) / 255.0;
                for c in 0..3 {
                    strip[dst + c] = (f32::from(px[src + c]) * a + 255.0 * (1.0 - a)).round() as u8;
                }
                strip[dst + 3] = 255;
            }
        }
    }
    let im = prism_codecs::Image::from_raw(w * 3, h, prism_codecs::ChannelLayout::Rgba, prism_codecs::SampleType::U8, strip).unwrap();
    std::fs::write(path, prism_codecs::encode(&im, prism_codecs::Format::Png, &prism_codecs::EncodeOptions::default()).unwrap()).unwrap();
}

/// Local look at CLIP STUDIO's own hand poses (its installed `handsetup_*.pep`, read in place and
/// never copied) put on Prism's standing figure: each on both hands, as strips in
/// `$PRISM_LOOK_OUT` (default `target/csp-hand-poses`). Set `PRISM_TEST_CSP_PAINT_3D` to the
/// folder holding them when CLIP STUDIO is installed elsewhere.
#[test]
#[ignore = "reads CLIP STUDIO's installed hand poses and writes PNGs to look at"]
fn local_hand_poses_render() {
    use prism_algo::figure3d::hand::{Side, apply_hand_pose};
    use prism_doc::figure::standing_figure;
    let dir = std::env::var_os("PRISM_TEST_CSP_PAINT_3D").map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\Program Files\CELSYS\CLIP STUDIO 1.5\CLIP STUDIO PAINT\Settings\PAINT\3D".into());
    let out = std::path::PathBuf::from(std::env::var_os("PRISM_LOOK_OUT").unwrap_or_else(|| "target/csp-hand-poses".into()));
    std::fs::create_dir_all(&out).unwrap();
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let path = entry.path();
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        if !stem.starts_with("handsetup_") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let super::super::pose::Pose::Hand(h) = super::super::pose::read_any(&bytes, &stem, &prism_raster::Interrupt::NONE).unwrap() else { panic!("{stem}: not a hand pose") };
        let from = if h.left { Side::Left } else { Side::Right };
        let mut f = standing_figure();
        for side in [Side::Left, Side::Right] {
            apply_hand_pose(&mut f, side, &h.hand, from, [false; 5]).unwrap();
        }
        hand_strip(&f, "left", &out.join(format!("{stem}_left.png")));
        hand_strip(&f, "right", &out.join(format!("{stem}_right.png")));
        eprintln!("{stem}: {} hand, {} fingers", if h.left { "left" } else { "right" }, h.hand.children.len());
        seen += 1;
    }
    assert!(seen > 0, "no handsetup_*.pep in {}", dir.display());
}

/// Local check of downloaded hand pose materials: every listed pose with its hand flag, and each
/// hand pose put on Prism's standing figure (both hands), as strips in `$PRISM_LOOK_OUT`
/// (default `target/csp-hand-materials`).
#[test]
#[ignore = "reads the local CLIP STUDIO folder and writes PNGs to look at"]
fn local_hand_pose_materials() {
    use prism_algo::figure3d::hand::{Side, apply_hand_pose};
    use prism_doc::figure::standing_figure;
    let out = std::path::PathBuf::from(std::env::var_os("PRISM_LOOK_OUT").unwrap_or_else(|| "target/csp-hand-materials".into()));
    std::fs::create_dir_all(&out).unwrap();
    let root = std::env::var_os("PRISM_TEST_CSP_ROOT").map(std::path::PathBuf::from).or_else(default_root).unwrap();
    let lib = list(&root).unwrap();
    let mut hands = 0;
    for (i, p) in lib.poses.iter().enumerate() {
        let (catalog, pose) = import_pose_any(&root, &p.uuid, &prism_raster::Interrupt::NONE).unwrap_or_else(|e| panic!("{}: {e}", p.name));
        match pose {
            super::super::pose::Pose::Body(f) => eprintln!("{i:2} body  {} [{catalog}] listed hand={:?} joints={}", p.name, p.hand, f.root.walk().len()),
            super::super::pose::Pose::Hand(h) => {
                hands += 1;
                let names: Vec<String> = h.hand.walk().into_iter().map(|(_, j)| j.name.clone()).collect();
                eprintln!("{i:2} hand  {} [{catalog}] listed hand={:?} left={} joints={} first={:?}", p.name, p.hand, h.left, names.len(), names.get(1));
                if std::env::var_os("PRISM_DUMP_HANDS").is_some() {
                    for (d, j) in h.hand.walk() {
                        let a = 2.0 * j.rotation[0].clamp(-1.0, 1.0).acos().to_degrees();
                        eprintln!("    {}{} rot {:?} ({a:.1} deg) off {:?}", "  ".repeat(d), j.name, j.rotation.map(|v| (v * 1000.0).round() / 1000.0), j.offset.map(|v| (v * 100.0).round() / 100.0));
                    }
                }
                assert_eq!(p.hand, Some(h.left), "{}: the listing tells hand poses apart", p.name);
                let from = if h.left { Side::Left } else { Side::Right };
                let mut f = standing_figure();
                for side in [Side::Left, Side::Right] {
                    apply_hand_pose(&mut f, side, &h.hand, from, [false; 5]).unwrap();
                }
                let side = if h.left { "left" } else { "right" };
                hand_strip(&f, side, &out.join(format!("hand{i:02}.png")));
            }
        }
    }
    eprintln!("{hands} hand poses of {}", lib.poses.len());
}
