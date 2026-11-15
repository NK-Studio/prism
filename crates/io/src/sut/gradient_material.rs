//! CLIP STUDIO gradient materials: a gradient set (`.colorset`, the typed value records auto
//! actions use, holding `gradationSetArray` → `gradationItemArray` → name and `gradationData`)
//! and a gradient sub-tool (`.tool`, a sub-tool database whose variants carry `GradationData`).
//! Both gradient blobs are the layout `prism_paint::csp_gradient` decodes. Evidence:
//! docs/sut-analysis.md › Gradient materials.
use super::sqlite::{Database, Value as Cell};
use prism_paint::csp_gradient::CspGradient;
use serde_json::Value;

type Result<T> = std::result::Result<T, String>;

/// Defensive caps, not CLIP STUDIO limits.
const MAX_GRADIENTS: usize = 1024;
const MAX_NAME_CHARS: usize = 255;
/// The longest gradient blob read (a stop record is 28 bytes, a curve point 16).
const MAX_GRADIENT_BYTES: usize = 64 * 1024;

/// One named group of gradients: a set of a `.colorset`, or the sub-tools of a `.tool`.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientGroup {
    /// The set's name; `None` for a sub-tool (its material name names it).
    pub name: Option<String>,
    pub gradients: Vec<(String, CspGradient)>,
}

/// Gradients read from one material file, and what could not be read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GradientMaterial {
    pub groups: Vec<GradientGroup>,
    pub warnings: Vec<String>,
}

impl GradientMaterial {
    pub fn count(&self) -> usize {
        self.groups.iter().map(|g| g.gradients.len()).sum()
    }
}

fn name(s: &str, fallback: impl FnOnce() -> String) -> String {
    let s: String = s.chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect();
    if s.trim().is_empty() { fallback() } else { s.trim().to_string() }
}

/// Read a downloaded gradient material: a `.colorset` gradient set or a gradient `.tool`, plain
/// or in its C2F envelope.
pub fn read(bytes: &[u8], ctl: &prism_raster::Interrupt) -> Result<GradientMaterial> {
    ctl.check().map_err(|e| e.to_string())?;
    if bytes.len() > 64 << 20 {
        return Err("CLIP STUDIO gradient material exceeds 64 MiB".into());
    }
    let decoded;
    let plain = if bytes.starts_with(crate::csp_container::MAGIC) {
        decoded = crate::csp_container::decode(bytes, ctl)?;
        &decoded[..]
    } else {
        bytes
    };
    let out = if plain.starts_with(b"SQLite format 3\0") { read_tool(plain)? } else { read_set(&crate::laf::read_values(plain, ctl)?)? };
    ctl.check().map_err(|e| e.to_string())?;
    if out.count() == 0 {
        let why = out.warnings.first().map_or_else(String::new, |w| format!(" ({w})"));
        return Err(format!("the CLIP STUDIO material holds no gradient Prism can read{why}"));
    }
    Ok(out)
}

fn hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || s.len() > 2 * MAX_GRADIENT_BYTES {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect()
}

/// A `.colorset`: `{gradationSetArray: [{gradationSetName, gradationItemArray: [{gradationName,
/// gradationData}]}]}`.
fn read_set(root: &Value) -> Result<GradientMaterial> {
    let sets = root.get("gradationSetArray").and_then(Value::as_array).ok_or("not a CLIP STUDIO gradient set")?;
    let mut out = GradientMaterial::default();
    let mut total = 0usize;
    for (s, set) in sets.iter().enumerate() {
        let set_name = name(set.get("gradationSetName").and_then(Value::as_str).unwrap_or(""), || format!("Gradient set {}", s + 1));
        let items = set.get("gradationItemArray").and_then(Value::as_array).ok_or_else(|| format!("{set_name}: no gradient list"))?;
        let mut gradients = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let label = name(item.get("gradationName").and_then(Value::as_str).unwrap_or(""), || format!("Gradient {}", i + 1));
            if total >= MAX_GRADIENTS {
                out.warnings.push(format!("only the first {MAX_GRADIENTS} gradients are read"));
                break;
            }
            let data = item.get("gradationData").and_then(|v| v.get(crate::laf::BYTES_KEY)).and_then(Value::as_str).and_then(hex);
            match data.ok_or_else(|| "missing gradient data".to_string()).and_then(|b| CspGradient::decode(&b).map_err(str::to_string)) {
                Ok(g) => {
                    gradients.push((label, g));
                    total += 1;
                }
                Err(e) => out.warnings.push(format!("{label}: {e}; skipped")),
            }
        }
        if !gradients.is_empty() {
            out.groups.push(GradientGroup { name: Some(set_name), gradients });
        }
    }
    Ok(out)
}

/// A gradient sub-tool `.tool`: the outer database's one `Material` holds the sub-tool database;
/// each `Node` names a sub-tool whose `Variant` (`NodeVariantID`) carries `GradationData`.
fn read_tool(bytes: &[u8]) -> Result<GradientMaterial> {
    let outer = Database::new(bytes)?;
    let materials = outer.table("Material")?;
    let [material] = materials.as_slice() else { return Err("CLIP STUDIO tool must contain exactly one material".into()) };
    let material_name = material.get("MaterialName").and_then(Cell::text).unwrap_or("");
    let tool = material.get("MaterialToolVariantManager").and_then(Cell::blob).ok_or("CLIP STUDIO material is not a sub-tool")?;
    if !tool.starts_with(b"SQLite format 3\0") {
        return Err("CLIP STUDIO sub-tool settings are not a SQLite database".into());
    }
    let inner = Database::new(tool)?;
    let (nodes, variants) = (inner.table("Node")?, inner.table("Variant")?);
    let mut out = GradientMaterial::default();
    let mut gradients = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let Some(id) = node.get("NodeVariantID").and_then(Cell::integer) else { continue };
        let Some(variant) = variants.iter().find(|v| v.get("VariantID").and_then(Cell::integer) == Some(id)) else { continue };
        let label = name(node.get("NodeName").and_then(Cell::text).unwrap_or(material_name), || format!("Gradient {}", i + 1));
        let Some(blob) = variant.get("GradationData").and_then(Cell::blob) else {
            out.warnings.push(format!("{label}: not a gradient sub-tool; skipped"));
            continue;
        };
        if gradients.len() >= MAX_GRADIENTS {
            out.warnings.push(format!("only the first {MAX_GRADIENTS} gradients are read"));
            break;
        }
        match CspGradient::decode(blob) {
            Ok(g) => gradients.push((label, g)),
            Err(e) => out.warnings.push(format!("{label}: {e}; skipped")),
        }
    }
    if !gradients.is_empty() {
        out.groups.push(GradientGroup { name: None, gradients });
    }
    Ok(out)
}
