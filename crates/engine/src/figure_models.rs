//! Parsed character models of 3D figure layers, cached: a figure keeps its VRM / glTF file's
//! bytes, and every render needs the parsed mesh. Entries are found by the bytes' `Arc` (shared
//! by the document, its undo history and copies) and, failing that, by content.
use prism_algo::figure3d::mesh::Model;
use prism_doc::figure::{FigureModel, FigureScene};
use std::sync::{Arc, Mutex, PoisonError};

/// Most models kept, and most bytes (file + parsed mesh).
const MAX_ENTRIES: usize = 8;
const MAX_BYTES: usize = 1 << 30;

struct Entry {
    bytes: Arc<Vec<u8>>,
    model: Arc<Model>,
    size: usize,
}

static CACHE: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

/// The parsed model of `m` (read once, then shared).
pub fn model_of(m: &FigureModel) -> Result<Arc<Model>, String> {
    {
        let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(i) = cache.iter().position(|e| Arc::ptr_eq(&e.bytes, &m.bytes) || (e.bytes.len() == m.bytes.len() && e.bytes == m.bytes)) {
            let e = cache.remove(i);
            let model = e.model.clone();
            cache.push(e);
            return Ok(model);
        }
    }
    // Named after its metadata, else the file's stem.
    let stem = std::path::Path::new(&m.file_name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let file = prism_io::model::read(&m.bytes, &stem, None, &prism_raster::Interrupt::NONE).map_err(|e| format!("{}: {e}", m.file_name))?;
    let model = Arc::new(file.model);
    let size = m.bytes.len() + model.byte_size();
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    cache.push(Entry { bytes: m.bytes.clone(), model: model.clone(), size });
    while cache.len() > MAX_ENTRIES || (cache.len() > 1 && cache.iter().map(|e| e.size).sum::<usize>() > MAX_BYTES) {
        cache.remove(0);
    }
    Ok(model)
}

/// Every figure's model, index for index (`None`: the mannequin).
pub fn models_of(scene: &FigureScene) -> Result<Vec<Option<Arc<Model>>>, String> {
    scene.figures.iter().map(|f| f.model.as_ref().map(model_of).transpose()).collect()
}
