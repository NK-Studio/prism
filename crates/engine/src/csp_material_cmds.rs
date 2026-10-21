//! CLIP STUDIO layer materials (images and special rulers) added to the open document, like
//! dragging one from PAINT's Material palette onto the canvas. Read-only on the CLIP STUDIO
//! side: see `prism_io::sut::library` and `prism_io::sut::layer_material`.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use prism_doc::{Fill, Layer, LayerContent, LayerId, pattern::Pattern};
use prism_geom::Rect;
use prism_io::sut::layer_material::{LayerMaterial, MaterialImage, MaterialInfo};
use prism_io::sut::library;
use prism_raster::Surface;
use serde_json::{Value, json};
use std::path::PathBuf;

const IMPORT: &str = "layer.cspMaterial.import";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: IMPORT.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// A straight RGBA8 image as a surface in `format`, its top-left at `at`.
fn surface(image: &MaterialImage, at: (i32, i32), format: prism_color::PixelFormat) -> Result<Surface> {
    let (w, h) = (image.width as usize, image.height as usize);
    if image.rgba.len() != w.saturating_mul(h).saturating_mul(4) {
        return Err(EngineError::Other("CLIP STUDIO material pixels have the wrong size".into()));
    }
    let data: Vec<f32> = image.rgba.iter().map(|v| f32::from(*v) / 255.0).collect();
    let mut s = Surface::new(prism_color::PixelFormat::RGBA8);
    let x1 = at.0.checked_add(image.width as i32).ok_or_else(|| bad("material placement is out of range"))?;
    let y1 = at.1.checked_add(image.height as i32).ok_or_else(|| bad("material placement is out of range"))?;
    s.write_region(Rect::new(at.0, at.1, x1, y1), &data);
    Ok(if format == s.format() { s } else { s.convert(format) })
}

/// Add `material`'s layers above the active layer as one history step; the first layer ends on
/// top. The material canvas is centred on the document, at its own pixel size.
fn place(s: &mut Session, name: &str, material: LayerMaterial, info: &MaterialInfo) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let size = st.doc.size;
    let dx = (i64::from(size.width) - i64::from(material.width)) / 2;
    let dy = (i64::from(size.height) - i64::from(material.height)) / 2;
    let format = st.doc.pixel_format();
    let mut layers: Vec<Layer> = Vec::new();
    let mut patterns: Vec<Pattern> = Vec::new();
    let mut rulers = 0;
    for l in material.layers.iter().rev() {
        let shift = |o: i32, d: i64| i32::try_from(i64::from(o) + d).map_err(|_| bad("material placement is out of range"));
        let origin = (shift(l.offset[0], dx)?, shift(l.offset[1], dy)?);
        let content = match &l.image {
            // A tiling image repeats across the canvas: a pattern fill whose tile starts where
            // the material's pixels do.
            Some(image) if info.tiling => {
                let image = image.tile();
                let tile = surface(&image, (0, 0), format)?;
                let pattern = Pattern::new(l.name.clone(), tile, image.width, image.height);
                let fill = Fill::Pattern { name: pattern.name.clone(), scale: 1.0, id: pattern.id.clone(), angle: 0.0, link: false, phase: (origin.0 as f32, origin.1 as f32) };
                patterns.push(pattern);
                LayerContent::Fill(fill)
            }
            Some(image) => LayerContent::Raster(surface(image, origin, format)?),
            None => LayerContent::Raster(Surface::new(format)),
        };
        let mut layer = Layer::new(l.name.clone(), content);
        layer.visible = l.visible;
        layer.opacity = l.opacity;
        if let Some(mut r) = l.rulers.clone() {
            r.translate([dx as f64, dy as f64]);
            r.validate().map_err(EngineError::Other)?;
            rulers += r.rulers.len();
            layer.special_rulers = Some(r);
        }
        layers.push(layer);
    }
    let ids = s.edit("Add CLIP STUDIO Material", |doc, active| {
        for p in &patterns {
            crate::pattern_cmds::ensure_in_doc(doc, p);
        }
        let mut ids = Vec::new();
        for layer in layers {
            let id = doc.insert_above(*active, layer);
            *active = Some(id);
            ids.push(id);
        }
        Ok(ids)
    })?;
    let top: Vec<u64> = ids.iter().rev().map(|id: &LayerId| id.0).collect();
    Ok(json!({ "name": name, "layers": top, "rulers": rulers, "tiling": info.tiling, "tags": info.tags, "warnings": material.warnings }))
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let text = |k: &str| -> Result<Option<String>> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_str().map(str::trim).filter(|v| !v.is_empty() && v.len() <= 4096).map(|v| Some(v.to_string())).ok_or_else(|| bad(format!("`{k}` must be a non-empty string"))),
        }
    };
    let (uuid, path) = (text("uuid")?, text("path")?);
    let ctl = &prism_raster::Interrupt::NONE;
    let (name, material, info) = match (uuid, path) {
        (Some(uuid), None) => {
            let root = crate::csp_library_cmds::root(IMPORT, p)?;
            library::import_layer(&root, &uuid, ctl).map_err(EngineError::Other)?
        }
        (None, Some(path)) => {
            let path = PathBuf::from(path);
            let (material, info) = library::read_layer_file(&path, ctl).map_err(EngineError::Other)?;
            let name = material.layers.first().map(|l| l.name.clone()).unwrap_or_default();
            (name, material, info)
        }
        _ => return Err(bad("pass either `uuid` (from brush.cspLibrary.list layers) or `path` (a .layer file or material folder)")),
    };
    place(s, &name, material, &info)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: IMPORT,
        label: "Add CLIP STUDIO Material",
        menu: &[],
        shortcut: None,
        params: r#"{"uuid":layer material id from brush.cspLibrary.list "layers","root":string? | "path":".layer file or material folder"} → {name, layers:[id] (top first), rulers, tiling, tags, warnings}. Adds the material's layers above the active layer, centred: special rulers stay on their layer, a tiling image becomes a pattern fill."#,
        enabled: has_doc,
        run: import,
        journal: true,
    }]
}

#[cfg(test)]
mod tests;
