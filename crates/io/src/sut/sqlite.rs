//! Read-only subset of SQLite's public file format: rowid table b-trees and records.
//! No SQL execution, native library, temp files, or extension loading. All traversal is bounded.
//! Specification: https://www.sqlite.org/fileformat.html
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, String>;
/// Rows read from one table; the 64 MiB byte budget bounds the work as well.
const MAX_ROWS: usize = 65_536;
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
    Integer(i64),
    Number(f64),
    Text(String),
    Blob(Vec<u8>),
}
impl Value {
    pub fn number(&self) -> Option<f64> {
        match self {
            Self::Integer(v) => Some(*v as f64),
            Self::Number(v) if v.is_finite() => Some(*v),
            _ => None,
        }
    }
    /// Identifiers must retain SQLite's signed 64-bit precision. Integral REAL values are
    /// accepted only within the i64 range; its upper endpoint rounds to 2^63 in f64.
    pub fn integer(&self) -> Option<i64> {
        match self {
            Self::Integer(v) => Some(*v),
            Self::Number(v) if v.is_finite() && v.fract() == 0.0 && *v >= i64::MIN as f64 && *v < -(i64::MIN as f64) => Some(*v as i64),
            _ => None,
        }
    }
    pub fn text(&self) -> Option<&str> {
        if let Self::Text(v) = self { Some(v) } else { None }
    }
    pub fn blob(&self) -> Option<&[u8]> {
        if let Self::Blob(v) = self { Some(v) } else { None }
    }
}
fn record(b: &[u8], encoding: u8) -> Result<Vec<Value>> {
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
                Value::Integer((u << shift) as i64 >> shift)
            }
            7 => Value::Number(f64::from_bits(uint(bytes, 0, 8)?)),
            8 | 9 => Value::Integer((t - 8) as i64),
            _ if t % 2 == 0 => Value::Blob(bytes.to_vec()),
            _ => Value::Text(if encoding == 2 {
                if !bytes.len().is_multiple_of(2) {
                    return Err(invalid());
                }
                let units = bytes.chunks_exact(2).map(|v| v.try_into().map(u16::from_le_bytes).map_err(|_| invalid())).collect::<Result<Vec<_>>>()?;
                String::from_utf16(&units).map_err(|_| invalid())?
            } else {
                String::from_utf8(bytes.to_vec()).map_err(|_| invalid())?
            }),
        };
        values.push(value);
        data = data.checked_add(n).ok_or_else(invalid)?;
    }
    if data != b.len() {
        return Err(invalid());
    }
    Ok(values)
}

/// The value SQLite supplies for a column missing from an older record: its literal
/// `DEFAULT` (NULL, a number or a quoted string), or NULL without one.
fn declared_default(column: &str) -> Option<Value> {
    let words: Vec<&str> = column.split_whitespace().collect();
    let Some(at) = words.iter().position(|w| w.eq_ignore_ascii_case("DEFAULT")) else { return Some(Value::Null) };
    let literal = words.get(at + 1..)?.join(" ");
    if literal.eq_ignore_ascii_case("NULL") {
        return Some(Value::Null);
    }
    if let Some(text) = literal.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')) {
        return Some(Value::Text(text.replace("''", "'")));
    }
    if let Ok(v) = literal.parse::<i64>() {
        return Some(Value::Integer(v));
    }
    literal.parse::<f64>().ok().filter(|v| v.is_finite()).map(Value::Number)
}

pub(super) struct Database<'a> {
    bytes: &'a [u8],
    page_size: usize,
    usable: usize,
    encoding: u8,
    first_page: u64,
    recovered_schema: Option<Vec<Vec<Value>>>,
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
        // SUT files are UTF-8; CSP's own tool library (`.todb`) is UTF-16LE.
        let encoding = match uint(bytes, 56, 4)? {
            1 => 1,
            2 => 2,
            _ => return Err("SUT SQLite text encoding must be UTF-8 or UTF-16LE".into()),
        };
        Ok(Self { bytes, page_size, usable, encoding, first_page: 1, recovered_schema: None })
    }
    /// Limited recovery of the observed C2F material layout. Its first 5120 bytes (five
    /// 1024-byte pages) are unavailable. Only complete, structurally valid sqlite_schema leaf
    /// records are recovered; ordinary SUT database validation remains unchanged.
    /// Pages are `page_size` bytes (1024 and 4096 are observed); the encoded 5120-byte prefix
    /// hides every page it touches.
    pub fn partial_material_pages_sized(bytes: &'a [u8], encoding: u8, page_size: usize) -> Result<Self> {
        if !matches!(page_size, 1024 | 4096) {
            return Err(invalid());
        }
        let first_page = 5120usize.div_ceil(page_size) + 1;
        if bytes.len() > 64 << 20 || bytes.len() < first_page * page_size || !bytes.len().is_multiple_of(page_size) || !matches!(encoding, 1 | 2) {
            return Err(invalid());
        }
        let pages = bytes.len() / page_size;
        let mut db = Self { bytes, page_size, usable: page_size, encoding, first_page: first_page as u64, recovered_schema: None };
        let mut schema = BTreeMap::<String, Vec<Value>>::new();
        // Schema recovery is bounded independently of image size and never interprets
        // a text occurrence as a root page. A candidate must be a whole schema leaf.
        for id in first_page as u64..=pages.min(512) as u64 {
            let page = db.page(id)?;
            if page.first() != Some(&13) || uint(page, 3, 2)? > 64 {
                continue;
            }
            let Ok(rows) = db.rows_bounded(id, 64 << 10) else { continue };
            if rows.is_empty()
                || !rows.iter().all(|r| {
                    r.len() == 5
                        && matches!(r.first().and_then(Value::text), Some("table" | "index"))
                        && r.get(1).and_then(Value::text).is_some()
                        && r.get(2).and_then(Value::text).is_some()
                        && r.get(3).and_then(Value::integer).is_some_and(|v| v >= 0 && v <= pages as i64)
                        && matches!(r.get(4), Some(Value::Text(_) | Value::Null))
                })
            {
                continue;
            }
            for row in rows {
                if row.first().and_then(Value::text) != Some("table") {
                    continue;
                }
                let name = row.get(1).and_then(Value::text).ok_or_else(invalid)?;
                let root = row.get(3).and_then(Value::integer).ok_or_else(invalid)?;
                let sql = row.get(4).and_then(Value::text).ok_or_else(invalid)?;
                if !matches!(name, "Layer" | "Mipmap" | "MipmapInfo" | "Offscreen" | "Canvas") {
                    continue;
                }
                if row.get(2).and_then(Value::text) != Some(name) || !sql.starts_with(&format!("CREATE TABLE {name}(")) || !sql.ends_with(')') {
                    return Err(invalid());
                }
                // Missing prefix tables cannot be read and must never be guessed.
                if root < first_page as i64 {
                    continue;
                }
                if !matches!(db.page(root as u64)?.first(), Some(5 | 13)) || schema.contains_key(name) {
                    return Err("ambiguous or invalid material SQLite schema".into());
                }
                schema.insert(name.to_owned(), row);
            }
        }
        if !["Layer", "Mipmap", "MipmapInfo", "Offscreen"].iter().all(|name| schema.contains_key(*name)) {
            return Err("material SQLite schema is unavailable in supported plaintext pages".into());
        }
        db.recovered_schema = Some(schema.into_values().collect());
        Ok(db)
    }
    fn page(&self, id: u64) -> Result<&[u8]> {
        if id < self.first_page {
            return Err("material references an unavailable SQLite page".into());
        }
        let at = usize::try_from(id.checked_sub(1).ok_or_else(invalid)?).map_err(|_| invalid())?.checked_mul(self.page_size).ok_or_else(invalid)?;
        slice(self.bytes, at, self.usable)
    }
    fn rows(&self, root: u64) -> Result<Vec<(i64, Vec<Value>)>> {
        self.rows_with_ids(root, 64 << 20)
    }
    fn rows_bounded(&self, root: u64, max_bytes: usize) -> Result<Vec<Vec<Value>>> {
        Ok(self.rows_with_ids(root, max_bytes)?.into_iter().map(|(_, r)| r).collect())
    }
    /// Leaf records with their rowids (an `INTEGER PRIMARY KEY` column stores NULL and
    /// aliases the rowid).
    fn rows_with_ids(&self, root: u64, max_bytes: usize) -> Result<Vec<(i64, Vec<Value>)>> {
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
                let rowid = varint(p, &mut at)? as i64;
                budget = budget.checked_add(size).filter(|n| *n <= max_bytes).ok_or_else(invalid)?;
                if out.len() >= MAX_ROWS {
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
                out.push((rowid, record(&payload, self.encoding)?));
            }
        }
        Ok(out)
    }
    pub fn table(&self, name: &str) -> Result<Vec<BTreeMap<String, Value>>> {
        let ordinary_schema;
        let schema = if let Some(schema) = &self.recovered_schema {
            schema
        } else {
            ordinary_schema = self.rows(1)?.into_iter().map(|(_, r)| r).collect::<Vec<_>>();
            &ordinary_schema
        };
        for r in schema {
            if r.get(1).and_then(Value::text) != Some(name) {
                continue;
            }
            let root = r.get(3).and_then(Value::integer).filter(|v| *v >= 1 && *v <= i64::from(u32::MAX)).ok_or_else(invalid)? as u64;
            let sql = r.get(4).and_then(Value::text).ok_or_else(invalid)?;
            let start = sql.find('(').ok_or_else(invalid)?;
            let end = sql.rfind(')').ok_or_else(invalid)?;
            let body = sql.get(start + 1..end).ok_or_else(invalid)?;
            // CSP tables use a flat list of quoted column names, without expressions.
            let columns: Vec<String> =
                body.split(',').map(|c| c.split_whitespace().next().unwrap_or("").trim_matches(['"', '`', '[', ']']).to_string()).collect();
            let rowid_alias = body.split(',').position(|c| c.to_ascii_uppercase().split_whitespace().collect::<Vec<_>>().windows(3).any(|w| w == ["INTEGER", "PRIMARY", "KEY"]));
            return self
                .rows(root)?
                .into_iter()
                .map(|(rowid, mut r)| {
                    if let Some(slot) = rowid_alias.and_then(|i| r.get_mut(i))
                        && matches!(slot, Value::Null)
                    {
                        *slot = Value::Integer(rowid);
                    }
                    if r.len() != columns.len() {
                        // Columns added after a row was written (ALTER TABLE ADD COLUMN; CSP's
                        // catalogs and material schemas both grew this way) are not stored:
                        // SQLite reads them as their declared defaults.
                        if r.len() > columns.len() {
                            return Err(invalid());
                        }
                        for column in body.split(',').skip(r.len()) {
                            r.push(declared_default(column).ok_or_else(invalid)?);
                        }
                    }
                    Ok(columns.iter().cloned().zip(r).collect())
                })
                .collect();
        }
        Err(format!("SUT table {name} is missing"))
    }
}
