//! Read-only subset of SQLite's public file format: rowid table b-trees and records.
//! No SQL execution, native library, temp files, or extension loading. All traversal is bounded.
//! Specification: https://www.sqlite.org/fileformat.html
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, String>;
fn invalid() -> String {
    "invalid or unsupported SUT SQLite database".into()
}
fn slice(b: &[u8], at: usize, n: usize) -> Result<&[u8]> {
    b.get(at..at.checked_add(n).ok_or_else(invalid)?).ok_or_else(invalid)
}
fn uint(b: &[u8], at: usize, n: usize) -> Result<u64> {
    Ok(slice(b, at, n)?.iter().fold(0, |v, x| (v << 8) | u64::from(*x)))
}
fn varint(b: &[u8], at: &mut usize) -> Result<u64> {
    let mut v = 0u64;
    for i in 0..9 {
        let x = *b.get(*at).ok_or_else(invalid)?;
        *at = at.checked_add(1).ok_or_else(invalid)?;
        if i == 8 {
            return Ok((v << 8) | u64::from(x));
        }
        v = (v << 7) | u64::from(x & 127);
        if x < 128 {
            return Ok(v);
        }
    }
    Err(invalid())
}
#[derive(Clone, Debug)]
pub(super) enum Value {
    Null,
    Number(f64),
    Text(String),
    Blob(Vec<u8>),
}
impl Value {
    pub fn number(&self) -> Option<f64> {
        if let Self::Number(v) = self { Some(*v).filter(|v| v.is_finite()) } else { None }
    }
    pub fn text(&self) -> Option<&str> {
        if let Self::Text(v) = self { Some(v) } else { None }
    }
    pub fn blob(&self) -> Option<&[u8]> {
        if let Self::Blob(v) = self { Some(v) } else { None }
    }
}
fn record(b: &[u8]) -> Result<Vec<Value>> {
    let mut at = 0;
    let header = usize::try_from(varint(b, &mut at)?).map_err(|_| invalid())?;
    if header < at || header > b.len() {
        return Err(invalid());
    }
    let mut data = header;
    let mut values = Vec::new();
    while at < header {
        if values.len() >= 2048 {
            return Err(invalid());
        }
        let t = varint(slice(b, 0, header)?, &mut at)?;
        let n = match t {
            0 | 8 | 9 => 0,
            1..=4 => t,
            5 => 6,
            6 | 7 => 8,
            10 | 11 => return Err(invalid()),
            _ => (t - 12) / 2,
        };
        let n = usize::try_from(n).map_err(|_| invalid())?;
        let bytes = slice(b, data, n)?;
        let value = match t {
            0 => Value::Null,
            1..=6 => {
                let u = uint(bytes, 0, n)?;
                let shift = 64 - n * 8;
                Value::Number(((u << shift) as i64 >> shift) as f64)
            }
            7 => Value::Number(f64::from_bits(uint(bytes, 0, 8)?)),
            8 | 9 => Value::Number((t - 8) as f64),
            _ if t % 2 == 0 => Value::Blob(bytes.to_vec()),
            _ => Value::Text(String::from_utf8(bytes.to_vec()).map_err(|_| invalid())?),
        };
        values.push(value);
        data = data.checked_add(n).ok_or_else(invalid)?;
    }
    if data != b.len() {
        return Err(invalid());
    }
    Ok(values)
}

pub(super) struct Database<'a> {
    bytes: &'a [u8],
    page_size: usize,
    usable: usize,
}
impl<'a> Database<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() > 64 << 20 {
            return Err("SUT files larger than 64 MiB are unsupported".into());
        }
        let offset = bytes.windows(16).position(|w| w == b"SQLite format 3\0").ok_or_else(invalid)?;
        let bytes = bytes.get(offset..).ok_or_else(invalid)?;
        let p = uint(bytes, 16, 2)? as usize;
        let page_size = if p == 1 { 65536 } else { p };
        if !(512..=65536).contains(&page_size) || !page_size.is_power_of_two() || !bytes.len().is_multiple_of(page_size) {
            return Err(invalid());
        }
        let usable = page_size.checked_sub(uint(bytes, 20, 1)? as usize).filter(|n| *n >= 480).ok_or_else(invalid)?;
        if uint(bytes, 56, 4)? != 1 {
            return Err("SUT SQLite text encoding must be UTF-8".into());
        }
        Ok(Self { bytes, page_size, usable })
    }
    fn page(&self, id: u64) -> Result<&[u8]> {
        let at = usize::try_from(id.checked_sub(1).ok_or_else(invalid)?).map_err(|_| invalid())?.checked_mul(self.page_size).ok_or_else(invalid)?;
        slice(self.bytes, at, self.usable)
    }
    fn rows(&self, root: u64) -> Result<Vec<Vec<Value>>> {
        let mut pending = vec![(root, 0)];
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        let mut budget = 0usize;
        while let Some((id, depth)) = pending.pop() {
            if depth > 32 || !seen.insert(id) {
                return Err(invalid());
            }
            let p = self.page(id)?;
            let h = if id == 1 { 100 } else { 0 };
            let kind = uint(p, h, 1)?;
            let n = uint(p, h + 3, 2)? as usize;
            let header_len = match kind {
                5 => 12,
                13 => 8,
                _ => return Err(invalid()),
            };
            slice(p, h + header_len, n * 2)?;
            if kind == 5 {
                pending.push((uint(p, h + 8, 4)?, depth + 1));
            }
            for i in (0..n).rev() {
                let mut at = uint(p, h + header_len + i * 2, 2)? as usize;
                if kind == 5 {
                    pending.push((uint(p, at, 4)?, depth + 1));
                    continue;
                }
                let size = usize::try_from(varint(p, &mut at)?).map_err(|_| invalid())?;
                varint(p, &mut at)?; // rowid (column _PW_ID is not used)
                budget = budget.checked_add(size).filter(|n| *n <= 64 << 20).ok_or_else(invalid)?;
                if out.len() >= 4096 {
                    return Err(invalid());
                }
                let max = self.usable - 35;
                let min = (self.usable - 12) * 32 / 255 - 23;
                let local = if size <= max {
                    size
                } else {
                    let k = min + (size - min) % (self.usable - 4);
                    if k <= max { k } else { min }
                };
                let mut payload = slice(p, at, local)?.to_vec();
                if local < size {
                    let mut next = uint(p, at + local, 4)?;
                    while payload.len() < size {
                        if !seen.insert(next) {
                            return Err(invalid());
                        }
                        let ov = self.page(next)?;
                        next = uint(ov, 0, 4)?;
                        let take = (size - payload.len()).min(self.usable - 4);
                        payload.extend_from_slice(slice(ov, 4, take)?);
                    }
                    if next != 0 {
                        return Err(invalid());
                    }
                }
                out.push(record(&payload)?);
            }
        }
        Ok(out)
    }
    pub fn table(&self, name: &str) -> Result<Vec<BTreeMap<String, Value>>> {
        for r in self.rows(1)? {
            if r.get(1).and_then(Value::text) != Some(name) {
                continue;
            }
            let root = r.get(3).and_then(Value::number).filter(|v| *v >= 1.0 && *v <= u32::MAX as f64).ok_or_else(invalid)? as u64;
            let sql = r.get(4).and_then(Value::text).ok_or_else(invalid)?;
            let start = sql.find('(').ok_or_else(invalid)?;
            let end = sql.rfind(')').ok_or_else(invalid)?;
            let body = sql.get(start + 1..end).ok_or_else(invalid)?;
            // CSP tables use a flat list of quoted column names, without expressions.
            let columns: Vec<String> =
                body.split(',').map(|c| c.split_whitespace().next().unwrap_or("").trim_matches(['"', '`', '[', ']']).to_string()).collect();
            return self
                .rows(root)?
                .into_iter()
                .map(|r| {
                    if r.len() != columns.len() {
                        return Err(invalid());
                    }
                    Ok(columns.iter().cloned().zip(r).collect())
                })
                .collect();
        }
        Err(format!("SUT table {name} is missing"))
    }
}
