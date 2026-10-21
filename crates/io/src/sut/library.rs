//! Read-only import from an installed CLIP STUDIO PAINT's downloaded material library.
//!
//! Reads the local material catalog (`CatalogMaterial.cmdb`), the sub-tool caches the app keeps
//! for downloaded tools (`EditImageToolDownloadTool.dtdb`, then `EditImageTool.todb`) and the
//! tip material files those settings reference. Nothing is written, no server is contacted and
//! downloaded `.tool` containers are read when no cache exists. Layout evidence: docs/sut-analysis.md.
use super::{
    Row, active_variant, read_tool, reference_paths,
    sqlite::{Database, Value},
};
use super::layer_material::{INFO_LIMIT, LayerMaterial, MaterialInfo, parse_info};
use crate::abr_map::AbrImport;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, String>;
const DB_LIMIT: u64 = 64 << 20;
const MATERIAL_LIMIT: u64 = 64 << 20;
/// Path segments of one material reference (`.:<bucket>:<bucket>:<uuid>:data:<file>`).
const MAX_SEGMENTS: usize = 16;
/// User tags kept per material, and the longest tag kept (characters).
const MAX_TAGS: usize = 64;
const MAX_TAG_CHARS: usize = 64;

const CATALOG: &str = "CLIPStudioCommon/MaterialDB/CatalogMaterial.cmdb";
const MATERIALS: &str = "CLIPStudioCommon/Material";
const DOWNLOAD_CACHE: &str = "Tool/EditImageToolDownloadTool.dtdb";
const TOOL_LIBRARY: &str = "Tool/EditImageTool.todb";

/// Shown with every listing: local access is not a licence to use every material in Prism.
pub const LICENSE_NOTICE: &str = "Downloaded CLIP STUDIO ASSETS materials can be licensed for use in CLIP STUDIO software only; check each material's terms before using it in Prism, and never redistribute it";

/// Which CLIP STUDIO PAINT cache holds a brush's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheSource {
    /// Downloaded material container, without starting PAINT or creating a tool cache.
    MaterialFile,
    /// The cache PAINT keeps for downloaded sub-tools (closest to the download).
    DownloadCache,
    /// PAINT's current sub-tool library; reflects any edits made in PAINT.
    ToolLibrary,
}

impl CacheSource {
    pub fn id(self) -> &'static str {
        match self {
            Self::MaterialFile => "materialFile",
            Self::DownloadCache => "downloadCache",
            Self::ToolLibrary => "toolLibrary",
        }
    }
}

/// Whether a downloaded brush can be imported now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ready(CacheSource),
    /// The catalog still marks the download as pending or in progress.
    Downloading,
    /// No sub-tool cache yet: PAINT has not processed this material.
    NoCache,
    /// The material is not stored below the local material folder.
    UnsupportedLocation,
    /// A layer material whose file is not on disk.
    Missing,
}

impl Status {
    pub fn id(self) -> &'static str {
        match self {
            Self::Ready(_) => "ready",
            Self::Downloading => "downloading",
            Self::NoCache => "noCache",
            Self::UnsupportedLocation => "unsupportedLocation",
            Self::Missing => "missing",
        }
    }
    /// What the user can do about a brush that is not ready.
    pub fn guidance(self) -> Option<&'static str> {
        match self {
            Self::Ready(_) => None,
            Self::Downloading => Some("The download has not finished in CLIP STUDIO; wait for it, then refresh"),
            Self::NoCache => {
                Some("CLIP STUDIO PAINT has not prepared this material yet; add it as a sub-tool in CLIP STUDIO PAINT once (or restart PAINT), then refresh")
            }
            Self::UnsupportedLocation => Some("This material is not stored in the local CLIP STUDIO material folder and cannot be read"),
            Self::Missing => Some("The material's file is missing; download it again in CLIP STUDIO, then refresh"),
        }
    }
}

/// The catalog's material kinds that are CLIP STUDIO PAINT sub-tools: brushes, and other
/// sub-tools (fill, selection, …) that import for the Clip Studio workspace.
pub const TOOL_KINDS: [&str; 2] = ["brush", "other_tool"];
/// The catalog's material kinds that are layer materials (images, special rulers).
pub const LAYER_KINDS: [&str; 1] = ["image"];

/// One downloaded sub-tool material.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryBrush {
    pub uuid: String,
    pub name: String,
    /// The catalog's material kind, one of [`TOOL_KINDS`].
    pub kind: String,
    /// The downloaded collection it belongs to (empty when unknown).
    pub catalog: String,
    pub status: Status,
    /// CLIP STUDIO's preview image of the material (`thumbnail/thumbnail.png`), resolved inside
    /// the material folder; read it with [`thumbnail_bytes`].
    pub thumbnail: Option<PathBuf>,
    /// The material's own user tags (CLIP STUDIO's "User tag"), in catalog order. Materials in a
    /// collection often have none; the collection's are on its [`Catalog`].
    pub tags: Vec<String>,
}

/// One downloaded layer material: an image, or a layer carrying special rulers.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryLayer {
    pub uuid: String,
    pub name: String,
    /// The downloaded collection it belongs to (empty when unknown).
    pub catalog: String,
    pub status: Status,
    pub thumbnail: Option<PathBuf>,
    pub tags: Vec<String>,
    /// From the material's `icedata/layerData.xml`.
    pub info: MaterialInfo,
}

/// One downloaded collection (catalog) and its cover image (`thumbnail/catalogThumb.png`).
#[derive(Clone, Debug, PartialEq)]
pub struct Catalog {
    pub name: String,
    pub thumbnail: Option<PathBuf>,
    /// The collection's user tags.
    pub tags: Vec<String>,
}

/// The downloaded brushes of one CLIP STUDIO user data folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Library {
    pub root: PathBuf,
    pub brushes: Vec<LibraryBrush>,
    /// Downloaded layer materials, in catalog order.
    pub layers: Vec<LibraryLayer>,
    /// The collections the brushes and layer materials belong to, in catalog order.
    pub catalogs: Vec<Catalog>,
    /// Folder-wide notices (license terms, stale snapshot, missing PAINT data).
    pub notices: Vec<String>,
}

/// The observed per-user CLIP STUDIO data folder (`%APPDATA%\CELSYSUserData\CELSYS` on
/// Windows). Other platforms are unverified: pass the folder explicitly.
pub fn default_root() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let root = PathBuf::from(std::env::var_os("APPDATA")?).join("CELSYSUserData").join("CELSYS");
    root.is_dir().then_some(root)
}

/// List the downloaded brushes below `root` (a CLIP STUDIO user data folder).
pub fn list(root: &Path) -> Result<Library> {
    let snap = Snapshot::read(root)?;
    let materials = std::fs::canonicalize(&snap.materials).ok();
    let image = |reference: String| materials.as_deref().and_then(|m| resolve(m, &reference).ok());
    let brushes = snap
        .entries
        .iter()
        .map(|e| LibraryBrush {
            uuid: e.uuid.clone(),
            name: e.name.clone(),
            kind: e.kind.clone(),
            catalog: e.catalog.clone(),
            status: snap.status(e),
            thumbnail: image(format!("{}:thumbnail:thumbnail.png", e.folder())),
            tags: e.tags.clone(),
        })
        .collect();
    let layers = snap
        .layer_entries
        .iter()
        .map(|e| {
            let reference = |file: &str| format!("{}:{file}", e.folder());
            let info = materials
                .as_deref()
                .and_then(|m| resolve(m, &reference("icedata:layerData.xml")).ok())
                .and_then(|p| read_limited(&p, INFO_LIMIT).ok())
                .map(|b| parse_info(&String::from_utf8_lossy(&b)))
                .unwrap_or_default();
            LibraryLayer {
                uuid: e.uuid.clone(),
                name: e.name.clone(),
                catalog: e.catalog.clone(),
                status: snap.layer_status(e),
                thumbnail: image(reference("thumbnail:thumbnail.png")),
                tags: e.tags.clone(),
                info,
            }
        })
        .collect();
    let mut catalogs: Vec<Catalog> = Vec::new();
    for e in snap.entries.iter().chain(&snap.layer_entries).filter(|e| !e.catalog.is_empty()) {
        if !catalogs.iter().any(|c| c.name == e.catalog) {
            catalogs.push(Catalog {
                name: e.catalog.clone(),
                thumbnail: image(format!("{}:thumbnail:catalogThumb.png", e.path)),
                tags: e.catalog_tags.clone(),
            });
        }
    }
    Ok(Library { root: root.to_path_buf(), brushes, layers, catalogs, notices: snap.notices })
}

/// Largest preview image read by [`thumbnail_bytes`].
pub const THUMBNAIL_LIMIT: u64 = 4 << 20;

/// The bytes of a preview image from [`list`] (at most [`THUMBNAIL_LIMIT`]).
pub fn thumbnail_bytes(path: &Path) -> Result<Vec<u8>> {
    read_limited(path, THUMBNAIL_LIMIT)
}

/// Import one downloaded brush by material UUID. `group` defaults to its collection name.
/// Returns the preset and the parser's fidelity warnings (thumbnail fallbacks included).
pub fn import(root: &Path, uuid: &str, group: Option<&str>, ctl: &prism_raster::Interrupt) -> Result<(String, AbrImport)> {
    ctl.check().map_err(|e| e.to_string())?;
    let snap = Snapshot::read(root)?;
    let entry = snap.entries.iter().find(|e| e.uuid == uuid).ok_or_else(|| format!("no downloaded CLIP STUDIO brush has the id {uuid}"))?;
    let status = snap.status(entry);
    let Status::Ready(source) = status else {
        return Err(format!("{}: {}", entry.name, status.guidance().unwrap_or("not ready")));
    };
    if source == CacheSource::MaterialFile {
        let path = snap.raw_tool(entry).ok_or("the downloaded tool file is no longer available; refresh and try again")?;
        let bytes = read_limited(&path, MATERIAL_LIMIT)?;
        let group = group.unwrap_or(if entry.catalog.is_empty() { "CLIP STUDIO" } else { &entry.catalog }).to_string();
        let mut imp = super::read_sut(&bytes, &group, ctl)?;
        imp.notes.insert(0, "Imported directly from the downloaded CLIP STUDIO material file".into());
        return Ok((group, imp));
    }
    let (node, variant) = snap.tool(source, uuid).ok_or("the brush's CLIP STUDIO cache changed while reading; refresh and try again")?;
    let group = group.map(str::to_string).unwrap_or_else(|| if entry.catalog.trim().is_empty() { "CLIP STUDIO".into() } else { entry.catalog.clone() });
    let materials = snap.materials(variant)?;
    ctl.check().map_err(|e| e.to_string())?;
    let mut imp = read_tool(node, variant, &|| Ok(materials.clone()), &group, ctl)?;
    imp.notes.insert(
        0,
        match source {
            CacheSource::MaterialFile => "Imported directly from the downloaded CLIP STUDIO material file",
            CacheSource::DownloadCache => "Imported from CLIP STUDIO PAINT's download cache, not from a SUT export",
            CacheSource::ToolLibrary => {
                "Imported from CLIP STUDIO PAINT's current sub-tool library; any edits made there are included and it can differ from the original download"
            }
        }
        .into(),
    );
    Ok((group, imp))
}

/// Import one downloaded layer material by material UUID: its name, the decoded layers and
/// what its `layerData.xml` says.
pub fn import_layer(root: &Path, uuid: &str, ctl: &prism_raster::Interrupt) -> Result<(String, LayerMaterial, MaterialInfo)> {
    ctl.check().map_err(|e| e.to_string())?;
    let snap = Snapshot::read(root)?;
    let entry = snap.layer_entries.iter().find(|e| e.uuid == uuid).ok_or_else(|| format!("no downloaded CLIP STUDIO layer material has the id {uuid}"))?;
    let status = snap.layer_status(entry);
    if !matches!(status, Status::Ready(_)) {
        return Err(format!("{}: {}", entry.name, status.guidance().unwrap_or("not ready")));
    }
    let path = snap.layer_file(entry).ok_or("the downloaded layer material file is no longer available; refresh and try again")?;
    let (material, info) = read_layer_file(&path, ctl)?;
    Ok((entry.name.clone(), material, info))
}

/// Read a layer material file (`data/material_0.layer`), or the material folder holding one.
/// The material's `icedata/layerData.xml` beside `data/` is read when present.
pub fn read_layer_file(path: &Path, ctl: &prism_raster::Interrupt) -> Result<(LayerMaterial, MaterialInfo)> {
    let file = if path.is_dir() { path.join("data").join("material_0.layer") } else { path.to_path_buf() };
    let bytes = read_limited(&file, MATERIAL_LIMIT)?;
    let material = super::layer_material::read(&bytes, ctl).map_err(|e| format!("{}: {e}", file.display()))?;
    let info = file
        .parent()
        .and_then(Path::parent)
        .map(|folder| folder.join("icedata").join("layerData.xml"))
        .filter(|p| p.is_file())
        .and_then(|p| read_limited(&p, INFO_LIMIT).ok())
        .map(|b| parse_info(&String::from_utf8_lossy(&b)))
        .unwrap_or_default();
    Ok((material, info))
}

/// A downloaded catalog brush or layer material.
struct Entry {
    uuid: String,
    name: String,
    kind: String,
    catalog: String,
    path: String,
    local: bool,
    pending: bool,
    tags: Vec<String>,
    catalog_tags: Vec<String>,
}

impl Entry {
    /// The material's own folder reference: inside a collection it sits below the collection.
    fn folder(&self) -> String {
        if self.catalog.is_empty() { self.path.clone() } else { format!("{}:{}", self.path, self.uuid) }
    }
}

/// One consistent in-memory read of the catalog and both sub-tool caches.
struct Snapshot {
    materials: PathBuf,
    entries: Vec<Entry>,
    layer_entries: Vec<Entry>,
    caches: Vec<(CacheSource, Vec<Row>, Vec<Row>)>,
    notices: Vec<String>,
}

fn text<'a>(r: &'a Row, k: &str) -> &'a str {
    r.get(k).and_then(Value::text).unwrap_or("")
}
fn int(r: &Row, k: &str) -> i64 {
    r.get(k).and_then(Value::integer).unwrap_or(0)
}

/// Read a whole file, refusing anything larger than `limit`.
fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{}: file too large", path.display()));
    }
    Ok(bytes)
}

/// An uncommitted SQLite write-ahead log or rollback journal: the main file can be behind.
fn has_pending_writes(db: &Path) -> bool {
    ["-wal", "-journal"].iter().any(|suffix| {
        let mut name = db.as_os_str().to_owned();
        name.push(suffix);
        std::fs::metadata(PathBuf::from(name)).is_ok_and(|m| m.len() > 0)
    })
}

impl Snapshot {
    fn read(root: &Path) -> Result<Self> {
        if !root.is_dir() {
            return Err(format!("CLIP STUDIO user data folder not found: {}", root.display()));
        }
        let mut notices = vec![LICENSE_NOTICE.to_string()];
        let mut stale = false;
        let catalog_path = root.join(CATALOG);
        if !catalog_path.is_file() {
            return Err(format!("no CLIP STUDIO material catalog in {}; start CLIP STUDIO once and download a brush", root.display()));
        }
        stale |= has_pending_writes(&catalog_path);
        let bytes = read_limited(&catalog_path, DB_LIMIT)?;
        let db = Database::new(&bytes).map_err(|e| format!("CLIP STUDIO material catalog: {e}"))?;
        let modifiers = db.table("MaterialModifier").map_err(|e| format!("CLIP STUDIO material catalog: {e}"))?;
        let meta = db.table("CatalogMetaData").map_err(|e| format!("CLIP STUDIO material catalog: {e}"))?;
        // Tags are optional: a catalog without (or with unreadable) tag tables lists untagged.
        let tags = match (db.table("Tag"), db.table("TableTagElementMap")) {
            (Ok(tags), Ok(map)) => Tags::new(&tags, &map),
            _ => Tags::default(),
        };
        drop(bytes);
        let entries = catalog_entries(&modifiers, &meta, &tags, &TOOL_KINDS);
        let layer_entries = catalog_entries(&modifiers, &meta, &tags, &LAYER_KINDS);

        let mut caches = Vec::new();
        match paint_folder(root) {
            Some(paint) => {
                for (source, rel, node_table, variant_table) in [
                    (CacheSource::DownloadCache, DOWNLOAD_CACHE, "ToolVariantNode", "ToolVariant"),
                    (CacheSource::ToolLibrary, TOOL_LIBRARY, "Node", "Variant"),
                ] {
                    let path = paint.join(rel);
                    if !path.is_file() {
                        continue;
                    }
                    stale |= has_pending_writes(&path);
                    let read = || -> Result<(Vec<Row>, Vec<Row>)> {
                        let bytes = read_limited(&path, DB_LIMIT)?;
                        let db = Database::new(&bytes)?;
                        Ok((db.table(node_table)?, db.table(variant_table)?))
                    };
                    match read() {
                        Ok((nodes, variants)) => caches.push((source, nodes, variants)),
                        Err(e) => notices.push(format!("CLIP STUDIO PAINT sub-tool cache {rel} could not be read ({e})")),
                    }
                }
            }
            None => notices.push("No CLIP STUDIO PAINT sub-tool cache was found; only self-contained downloaded tool files can be imported".into()),
        }
        if stale {
            notices.push("CLIP STUDIO appears to be running or has unsaved database changes; the newest downloads may be missing until it is closed".into());
        }
        Ok(Self { materials: root.join(MATERIALS), entries, layer_entries, caches, notices })
    }

    fn tool(&self, source: CacheSource, uuid: &str) -> Option<(&Row, &Row)> {
        let (_, nodes, variants) = self.caches.iter().find(|(s, _, _)| *s == source)?;
        nodes.iter().filter(|n| text(n, "NodeMaterialUuid") == uuid).find_map(|n| active_variant(n, variants).ok().map(|v| (n, v)))
    }

    fn status(&self, e: &Entry) -> Status {
        if !e.local {
            return Status::UnsupportedLocation;
        }
        if e.pending {
            return Status::Downloading;
        }
        // The download cache comes first: the current library can hold edited settings.
        [CacheSource::DownloadCache, CacheSource::ToolLibrary]
            .into_iter()
            .find(|s| self.tool(*s, &e.uuid).is_some())
            .or_else(|| self.raw_tool(e).map(|_| CacheSource::MaterialFile))
            .map_or(Status::NoCache, Status::Ready)
    }

    fn layer_status(&self, e: &Entry) -> Status {
        if !e.local {
            return Status::UnsupportedLocation;
        }
        if e.pending {
            return Status::Downloading;
        }
        self.layer_file(e).map_or(Status::Missing, |_| Status::Ready(CacheSource::MaterialFile))
    }

    fn layer_file(&self, e: &Entry) -> Option<PathBuf> {
        let root = std::fs::canonicalize(&self.materials).ok()?;
        resolve(&root, &format!("{}:data:material_0.layer", e.folder())).ok()
    }

    fn raw_tool(&self, e: &Entry) -> Option<PathBuf> {
        let root = std::fs::canonicalize(&self.materials).ok()?;
        let reference = format!("{}:data:material_0.tool", e.folder());
        resolve(&root, &reference).ok()
    }

    /// `MaterialFile`-shaped rows for every tip and texture the variant references. Missing
    /// textures are left out (the parser drops the texture with a warning); a missing tip fails.
    fn materials(&self, variant: &Row) -> Result<Vec<Row>> {
        let root = std::fs::canonicalize(&self.materials).map_err(|e| format!("CLIP STUDIO material folder: {e}"))?;
        let mut rows: Vec<Row> = Vec::new();
        for (key, required) in [("BrushPatternImageArray", int(variant, "BrushUsePatternImage") != 0), ("TextureImage", false)] {
            let Some(refs) = variant.get(key).and_then(Value::blob) else { continue };
            // Unreadable reference blobs are reported by the parser itself.
            let Ok(paths) = reference_paths(refs, "materials") else { continue };
            for path in paths {
                if rows.iter().any(|r| text(r, "OriginalPath") == path) {
                    continue;
                }
                match material_file(&root, &path) {
                    Ok(data) => rows.push(Row::from([("OriginalPath".into(), Value::Text(path)), ("FileData".into(), Value::Blob(data))])),
                    Err(e) if required => {
                        return Err(format!(
                            "a brush tip material is not available locally ({e}); open the brush once in CLIP STUDIO PAINT so it finishes downloading"
                        ));
                    }
                    Err(_) => {}
                }
            }
        }
        Ok(rows)
    }
}

/// User tags by material modifier id (`TableTagElementMap.ElementId` is `MaterialModifier._PW_ID`).
#[derive(Default)]
struct Tags(std::collections::HashMap<i64, Vec<String>>);

impl Tags {
    fn new(tags: &[Row], map: &[Row]) -> Self {
        let titles: std::collections::HashMap<i64, &str> =
            tags.iter().map(|t| (int(t, "_PW_ID"), text(t, "Title").trim())).filter(|(_, title)| !title.is_empty()).collect();
        let mut ordered: Vec<&Row> = map.iter().collect();
        ordered.sort_by_key(|m| int(m, "_PW_ID"));
        let mut out: std::collections::HashMap<i64, Vec<String>> = std::collections::HashMap::new();
        for m in ordered {
            let Some(title) = titles.get(&int(m, "TagId")) else { continue };
            let list = out.entry(int(m, "ElementId")).or_default();
            let title: String = title.chars().take(MAX_TAG_CHARS).collect();
            if list.len() < MAX_TAGS && !list.contains(&title) {
                list.push(title);
            }
        }
        Self(out)
    }
    fn of(&self, id: i64) -> Vec<String> {
        self.0.get(&id).cloned().unwrap_or_default()
    }
}

/// Downloaded sub-tools, joined to their collection names, in catalog order.
fn catalog_entries(modifiers: &[Row], meta: &[Row], tags: &Tags, kinds: &[&str]) -> Vec<Entry> {
    let meta_of = |id: i64| meta.iter().find(|m| int(m, "ModifierID") == id);
    let collection = |path: &str| {
        modifiers
            .iter()
            .find(|m| {
                text(m, "Path") == path && text(m, "MaterialUuid").is_empty() && meta_of(int(m, "_PW_ID")).is_some_and(|d| text(d, "MaterialKind") == "catalog")
            })
            .map(|m| (text(m, "MaterialName").to_string(), tags.of(int(m, "_PW_ID"))))
            .unwrap_or_default()
    };
    let mut ordered: Vec<&Row> = modifiers.iter().collect();
    ordered.sort_by_key(|m| int(m, "_PW_ID"));
    let mut out: Vec<Entry> = Vec::new();
    for m in ordered {
        let Some(d) = meta_of(int(m, "_PW_ID")) else { continue };
        let uuid = text(m, "MaterialUuid");
        let kind = text(d, "MaterialKind");
        if !kinds.contains(&kind) || int(d, "Downloaded") == 0 || uuid.is_empty() || out.iter().any(|e| e.uuid == uuid) {
            continue;
        }
        let path = text(m, "Path");
        let (catalog, catalog_tags) = collection(path);
        out.push(Entry {
            uuid: uuid.to_string(),
            name: text(m, "MaterialName").to_string(),
            kind: kind.to_string(),
            catalog,
            catalog_tags,
            tags: tags.of(int(m, "_PW_ID")),
            path: path.to_string(),
            local: int(m, "PathType") == 0 && path.starts_with(".:"),
            pending: int(d, "DownloadNotYet") != 0 || int(d, "NewDownloading") != 0,
        });
    }
    out
}

/// The newest `CLIPStudioPaintVer*` folder that holds sub-tool data.
fn paint_folder(root: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .take(4096)
        .filter(|e| e.file_name().to_string_lossy().starts_with("CLIPStudioPaintVer"))
        .map(|e| e.path())
        .filter(|p| p.join("Tool").is_dir())
        .collect();
    found.sort();
    found.pop()
}

/// Resolve a material reference (`.:a:b:…:data:material_0.layer`) inside the canonical
/// material root, rejecting traversal and links that leave it.
fn resolve(root: &Path, reference: &str) -> Result<PathBuf> {
    let invalid = || format!("invalid material reference {reference:?}");
    let mut parts = reference.split(':');
    if parts.next() != Some(".") {
        return Err(invalid());
    }
    let mut path = root.to_path_buf();
    let mut count = 0;
    for part in parts {
        count += 1;
        if count > MAX_SEGMENTS || part.is_empty() || part == "." || part == ".." || part.contains(['/', '\\', '\0']) {
            return Err(invalid());
        }
        path.push(part);
    }
    if count < 2 {
        return Err(invalid());
    }
    let canonical = std::fs::canonicalize(&path).map_err(|e| format!("{reference}: {e}"))?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(invalid());
    }
    Ok(canonical)
}

/// The SUT `MaterialFile` payload for one local material: a ustar package of the original
/// layer file and, when present, its thumbnail (the parser's explicit fallback).
fn material_file(root: &Path, reference: &str) -> Result<Vec<u8>> {
    let layer_path = resolve(root, reference)?;
    let layer = read_limited(&layer_path, MATERIAL_LIMIT)?;
    let file_name = layer_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| n.ends_with(".layer") && n.len() <= 80)
        .unwrap_or_else(|| "material_0.layer".into());
    let mut entries = vec![(format!("data/{file_name}"), layer)];
    if let Some(folder) = layer_path.parent().and_then(Path::parent) {
        let thumb = folder.join("thumbnail").join("thumbnail.png");
        if let Ok(canonical) = std::fs::canonicalize(&thumb)
            && canonical.starts_with(root)
            && let Ok(bytes) = read_limited(&canonical, 16 << 20)
        {
            entries.push(("thumbnail/thumbnail.png".into(), bytes));
        }
    }
    Ok(ustar(&entries))
}

/// A minimal POSIX ustar archive (regular files, short names).
fn ustar(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data) in entries {
        let mut h = [0u8; 512];
        let put = |h: &mut [u8; 512], at: usize, s: &[u8]| {
            for (dst, src) in h.iter_mut().skip(at).zip(s) {
                *dst = *src;
            }
        };
        put(&mut h, 0, name.as_bytes().get(..99).unwrap_or(name.as_bytes()));
        put(&mut h, 100, b"0000644\0");
        put(&mut h, 108, b"0000000\0");
        put(&mut h, 116, b"0000000\0");
        put(&mut h, 124, format!("{:011o}\0", data.len()).as_bytes());
        put(&mut h, 136, b"00000000000\0");
        put(&mut h, 148, b"        ");
        h[156] = b'0';
        put(&mut h, 257, b"ustar\0");
        put(&mut h, 263, b"00");
        let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
        put(&mut h, 148, format!("{sum:06o}\0 ").as_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    out
}

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;
