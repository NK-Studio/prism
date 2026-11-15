//! CLIP STUDIO colour set materials (`data/material_0.colorset` of a `color_set` material): a C2F
//! container holding a SQLite database whose `colorsetmaterial` table lists the colours in palette
//! order (`_PW_ID`), each with a name and 32-bit red, green and blue (`0xFFFFFFFF` = full, the
//! byte repeated as in CLIP STUDIO's other colours) and a transparent flag. Evidence:
//! docs/sut-analysis.md › Colour set materials.
use super::sqlite::{Database, Value};

type Result<T> = std::result::Result<T, String>;

/// Defensive caps, not CLIP STUDIO limits.
const MAX_COLORS: usize = 65_536;
const MAX_NAME_CHARS: usize = 255;

/// One colour of a set, in palette order.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorSetItem {
    /// As stored; usually empty.
    pub name: String,
    /// Red, green, blue 0..=1.
    pub rgb: [f32; 3],
}

/// The colours of one set and what could not be kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorSet {
    pub colors: Vec<ColorSetItem>,
    /// Transparent entries (CLIP STUDIO's transparent colour), left out.
    pub transparent: usize,
    pub warnings: Vec<String>,
}

fn channel(v: Option<&Value>) -> Option<f32> {
    let v = v?.integer()?;
    (0..=i64::from(u32::MAX)).contains(&v).then(|| (v as f64 / f64::from(u32::MAX)) as f32)
}

/// Read a colour set, plain SQLite or in its C2F envelope.
pub fn read(bytes: &[u8], ctl: &prism_raster::Interrupt) -> Result<ColorSet> {
    ctl.check().map_err(|e| e.to_string())?;
    if bytes.len() > 64 << 20 {
        return Err("CLIP STUDIO colour set exceeds 64 MiB".into());
    }
    let decoded;
    let plain = if bytes.starts_with(crate::csp_container::MAGIC) {
        decoded = crate::csp_container::decode(bytes, ctl)?;
        &decoded[..]
    } else {
        bytes
    };
    if !plain.starts_with(b"SQLite format 3\0") {
        return Err("not a CLIP STUDIO colour set".into());
    }
    let mut rows = Database::new(plain)?.table("colorsetmaterial").map_err(|e| format!("not a CLIP STUDIO colour set ({e})"))?;
    rows.sort_by_key(|r| r.get("_PW_ID").and_then(Value::integer).unwrap_or(i64::MAX));
    let mut out = ColorSet::default();
    let mut unreadable = 0;
    for r in &rows {
        if out.colors.len() >= MAX_COLORS {
            out.warnings.push(format!("only the first {MAX_COLORS} colours are read"));
            break;
        }
        if r.get("colorsetitemtransparent").and_then(Value::integer).unwrap_or(0) != 0 {
            out.transparent += 1;
            continue;
        }
        let rgb = [channel(r.get("colorsetitemred")), channel(r.get("colorsetitemgreen")), channel(r.get("colorsetitemblue"))];
        let [Some(red), Some(green), Some(blue)] = rgb else {
            unreadable += 1;
            continue;
        };
        let name: String = r.get("colorsetitemname").and_then(Value::text).unwrap_or("").chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect();
        out.colors.push(ColorSetItem { name: name.trim().to_string(), rgb: [red, green, blue] });
    }
    if unreadable > 0 {
        out.warnings.push(format!("{unreadable} colours with unreadable channels were skipped"));
    }
    if out.transparent > 0 {
        out.warnings.push(format!("{} transparent colours were left out: swatches are opaque", out.transparent));
    }
    ctl.check().map_err(|e| e.to_string())?;
    if out.colors.is_empty() {
        return Err("the CLIP STUDIO colour set holds no colour Prism can use".into());
    }
    Ok(out)
}
