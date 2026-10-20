//! Bounded reader for observed CLIP STUDIO auto-action value records.
//! Unknown value types fail explicitly; parsing never executes an action.
use serde_json::{Map, Value};
type Result<T> = std::result::Result<T, String>;
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
            17 => Ok(Value::from(self.integer()?)),
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
