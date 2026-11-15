//! Bounded reader for observed CLIP STUDIO auto-action value records.
//! Unknown value types fail explicitly; parsing never executes an action.
//!
//! Value tags (docs/sut-analysis.md › Auto-action value types): 6 UTF-8 string, 7 bytes,
//! 17 unsigned and 19/20 signed integers (19 ≥ 0, 20 < 0 storing the magnitude), 21 an f64,
//! 72 array and 73 object.
use serde_json::{Map, Value};
type Result<T> = std::result::Result<T, String>;
/// The key of the one-entry object a bytes value (tag 7) becomes: `{"$bytes": "<hex>"}`.
pub const BYTES_KEY: &str = "$bytes";
fn invalid() -> String {
    "invalid or unsupported CLIP STUDIO auto-action data".into()
}
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    values: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).ok_or_else(invalid)?;
        let b = self.bytes.get(self.at..end).ok_or_else(invalid)?;
        self.at = end;
        Ok(b)
    }
    fn byte(&mut self) -> Result<u8> {
        self.take(1)?.first().copied().ok_or_else(invalid)
    }
    fn integer(&mut self) -> Result<u64> {
        let n = self.byte()? as usize;
        if n > 8 {
            return Err(invalid());
        }
        Ok(self.take(n)?.iter().fold(0u64, |a, b| (a << 8) | u64::from(*b)))
    }
    fn count(&mut self) -> Result<usize> {
        usize::try_from(self.integer()?).ok().filter(|n| *n <= 4096).ok_or_else(invalid)
    }
    fn string(&mut self) -> Result<String> {
        let n = self.count()?;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| invalid())
    }
    fn fields(&mut self, depth: usize) -> Result<Map<String, Value>> {
        let n = self.count()?;
        let mut out = Map::new();
        for _ in 0..n {
            let key = self.string()?;
            let value = self.value(depth + 1)?;
            if out.insert(key, value).is_some() {
                return Err(invalid());
            }
        }
        Ok(out)
    }
    fn header(&mut self, depth: usize) -> Result<String> {
        let mut header = self.fields(depth)?;
        let name = header.remove("typename").and_then(|v| v.as_str().map(str::to_owned)).ok_or_else(invalid)?;
        if !header.is_empty() {
            return Err(invalid());
        }
        Ok(name)
    }
    fn object(&mut self, depth: usize) -> Result<Value> {
        let name = self.header(depth)?;
        let mut body = self.fields(depth)?;
        if body.insert("typename".into(), Value::String(name)).is_some() {
            return Err(invalid());
        }
        Ok(Value::Object(body))
    }
    fn value(&mut self, depth: usize) -> Result<Value> {
        self.values = self.values.saturating_add(1);
        if depth > 16 || self.values > 16384 {
            return Err("CLIP STUDIO auto-action nesting or value limit exceeded".into());
        }
        match self.byte()? {
            6 => Ok(Value::String(self.string()?)),
            // 17 is unsigned and 19 a signed integer that is not negative; both store the
            // magnitude with a byte-length prefix.
            17 | 19 => Ok(Value::from(self.integer()?)),
            // A negative signed integer stores its magnitude (saturation −100 is 20 01 64; the
            // all-ones colour channel −1 is 20 01 01).
            20 => {
                let magnitude = self.integer()?;
                let value = 0i64.checked_sub_unsigned(magnitude).ok_or_else(invalid)?;
                Ok(Value::from(value))
            }
            21 => {
                if self.byte()? != 8 {
                    return Err(invalid());
                }
                let v = f64::from_be_bytes(self.take(8)?.try_into().map_err(|_| invalid())?);
                if !v.is_finite() {
                    return Err(invalid());
                }
                Ok(Value::from(v))
            }
            // Opaque bytes (gradients, colour types), kept as hex so nothing is lost.
            7 => {
                let n = self.count()?;
                let hex: String = self.take(n)?.iter().map(|b| format!("{b:02x}")).collect();
                let mut m = Map::new();
                m.insert(BYTES_KEY.into(), Value::String(hex));
                Ok(Value::Object(m))
            }
            tag @ (72 | 73) => {
                let n = u32::from_be_bytes(self.take(4)?.try_into().map_err(|_| invalid())?) as usize;
                let mut sub = Reader { bytes: self.take(n)?, at: 0, values: self.values };
                let value = if tag == 73 {
                    sub.object(depth + 1)?
                } else {
                    if !sub.header(depth + 1)?.is_empty() {
                        return Err(invalid());
                    }
                    let n = sub.count()?;
                    let mut a = Vec::new();
                    for _ in 0..n {
                        a.push(sub.value(depth + 1)?);
                    }
                    Value::Array(a)
                };
                if sub.at != sub.bytes.len() {
                    return Err(invalid());
                }
                self.values = sub.values;
                Ok(value)
            }
            tag => Err(format!("unsupported CLIP STUDIO auto-action value type {tag}")),
        }
    }
}
/// Parse an exported LAF or its downloaded C2F envelope, retaining original command names.
pub fn read_laf(bytes: &[u8], ctl: &prism_raster::Interrupt) -> Result<Value> {
    ctl.check().map_err(|e| e.to_string())?;
    if bytes.len() > 8 << 20 {
        return Err("CLIP STUDIO auto-action exceeds 8 MiB".into());
    }
    let decoded;
    let bytes = if bytes.starts_with(crate::csp_container::MAGIC) {
        decoded = crate::csp_container::decode(bytes, ctl)?;
        &decoded
    } else {
        bytes
    };
    let mut r = Reader { bytes, at: 0, values: 0 };
    let value = r.object(0)?;
    if r.at != bytes.len() || value.get("typename").and_then(Value::as_str) != Some("typeActionSet") {
        return Err(invalid());
    }
    ctl.check().map_err(|e| e.to_string())?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn string(s: &str) -> Vec<u8> {
        [vec![1, s.len() as u8], s.as_bytes().to_vec()].concat()
    }
    fn fixture() -> Vec<u8> {
        [vec![1, 1], string("typename"), vec![6], string("typeActionSet"), vec![1, 1], string("actionSetName"), vec![6], string("Synthetic")].concat()
    }
    #[test]
    fn typed_values_are_bounded_and_container_independent() {
        let f = fixture();
        let v = read_laf(&f, &prism_raster::Interrupt::NONE).unwrap();
        assert_eq!(v["actionSetName"], "Synthetic");
        assert_eq!(read_laf(&crate::csp_container::plain_fixture(&f), &prism_raster::Interrupt::NONE).unwrap(), v);
        for n in 0..f.len() {
            assert!(read_laf(&f[..n], &prism_raster::Interrupt::NONE).is_err());
        }
        let mut bad = f.clone();
        bad.push(0);
        assert!(read_laf(&bad, &prism_raster::Interrupt::NONE).is_err());
        for i in 0..f.len() {
            let mut b = f.clone();
            b[i] = 255;
            let _ = read_laf(&b, &prism_raster::Interrupt::NONE);
        }
    }
    /// An action set whose one field `v` holds `value` (tag and payload bytes).
    fn with_value(value: &[u8]) -> Vec<u8> {
        [vec![1, 1], string("typename"), vec![6], string("typeActionSet"), vec![1, 1], string("v"), value.to_vec()].concat()
    }
    #[test]
    fn signed_float_and_byte_values() {
        let read = |value: &[u8]| read_laf(&with_value(value), &prism_raster::Interrupt::NONE).map(|v| v["v"].clone());
        // Signed integers: 19 is the value, 20 the negative of its magnitude.
        assert_eq!(read(&[19, 0]).unwrap(), 0);
        assert_eq!(read(&[19, 1, 14]).unwrap(), 14);
        assert_eq!(read(&[19, 4, 0xff, 0xff, 0xff, 0xff]).unwrap(), 4_294_967_295u64);
        assert_eq!(read(&[20, 1, 100]).unwrap(), -100);
        assert_eq!(read(&[20, 1, 1]).unwrap(), -1);
        assert_eq!(read(&[20, 8, 0x80, 0, 0, 0, 0, 0, 0, 0]).unwrap(), i64::MIN);
        assert!(read(&[20, 8, 0x80, 0, 0, 0, 0, 0, 0, 1]).is_err(), "below i64::MIN");
        // f64, big-endian, always eight bytes.
        assert_eq!(read(&[21, 8, 0x40, 0x59, 0, 0, 0, 0, 0, 0]).unwrap(), 100.0);
        assert_eq!(read(&[21, 8, 0xbf, 0xe0, 0, 0, 0, 0, 0, 0]).unwrap(), -0.5);
        assert!(read(&[21, 4, 0x40, 0x59, 0, 0]).is_err());
        assert!(read(&[21, 8, 0x7f, 0xf0, 0, 0, 0, 0, 0, 0]).is_err(), "infinity");
        assert!(read(&[21, 8, 0x7f, 0xf8, 0, 0, 0, 0, 0, 0]).is_err(), "NaN");
        // Bytes, kept as hex.
        assert_eq!(read(&[7, 1, 3, 0, 0xab, 1]).unwrap(), serde_json::json!({BYTES_KEY: "00ab01"}));
        assert_eq!(read(&[7, 0]).unwrap(), serde_json::json!({BYTES_KEY: ""}));
        assert!(read(&[7, 1, 4, 0, 1]).is_err(), "truncated");
        assert!(read(&[7, 2, 0x10, 0x01]).is_err(), "over the 4096-byte bound");
        for tag in [0u8, 1, 5, 8, 18, 22, 71, 74, 255] {
            assert!(read(&[tag, 0]).is_err(), "tag {tag}");
        }
    }
    #[test]
    #[ignore = "reads the local CLIP STUDIO material folder (PRISM_TEST_CSP_ROOT or the platform's)"]
    fn every_local_downloaded_laf_parses() {
        let root = std::env::var_os("PRISM_TEST_CSP_ROOT").map(std::path::PathBuf::from).or_else(crate::sut::library::default_root).unwrap();
        let mut stack = vec![root.join("CLIPStudioCommon").join("Material")];
        let mut count = 0;
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "laf") {
                    let v = read_laf(&std::fs::read(&p).unwrap(), &prism_raster::Interrupt::NONE).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                    assert!(v["actionArray"].is_array(), "{}", p.display());
                    count += 1;
                }
            }
        }
        eprintln!("{count} LAF files parsed");
        assert!(count > 0);
    }
    #[test]
    #[ignore = "reads PRISM_TEST_LAF, a local material never bundled in the repository"]
    fn local_laf_contains_complete_typed_actions() {
        let b = std::fs::read(std::env::var_os("PRISM_TEST_LAF").unwrap()).unwrap();
        let v = read_laf(&b, &prism_raster::Interrupt::NONE).unwrap();
        let actions = v["actionArray"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        let items = actions[0]["actionItemArray"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["actionItemKind"], "layeractionselectfromlayerstatusnew");
        assert_eq!(items[0]["actionItemParameter"]["referselected"], 0);
        assert_eq!(items[1]["actionItemKind"], "editactionclearoutofselectarea");
        assert_eq!(items[2]["actionItemKind"], "selectactiondeselect");
    }
}
