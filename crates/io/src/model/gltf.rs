//! glTF 2.0 plumbing: the GLB container, buffers (embedded, `data:` URIs or files beside a
//! `.gltf`), accessors of every component type, node transforms. Every size and index comes from
//! the file and is checked; nothing here panics on any input.
use serde_json::Value;
use std::path::Path;

pub(super) type Result<T> = std::result::Result<T, String>;
pub(super) type M4 = [f64; 16];

/// Largest model file read.
pub const MAX_FILE: usize = 256 << 20;
/// Largest JSON chunk.
const MAX_JSON: usize = 64 << 20;

/// A parsed glTF document: its JSON and its buffers.
pub(super) struct Doc {
    pub json: Value,
    pub buffers: Vec<Vec<u8>>,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Decode standard base64 (whitespace ignored).
pub(super) fn base64(text: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return Err("a glTF data URI is not base64".into()),
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

/// The bytes a `uri` names: a `data:` URI, or a file beside the `.gltf` (`dir`).
pub(super) fn uri_bytes(uri: &str, dir: Option<&Path>) -> Result<Vec<u8>> {
    if let Some(rest) = uri.strip_prefix("data:") {
        let (_, data) = rest.split_once(";base64,").ok_or("only base64 data URIs are supported")?;
        return base64(data);
    }
    let dir = dir.ok_or("this glTF keeps its data in other files: open it from disk")?;
    let name = percent_decode(uri);
    // Stay beside the file: no absolute paths, no climbing out.
    let rel = Path::new(&name);
    if rel.is_absolute() || rel.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::Prefix(_) | std::path::Component::RootDir)) {
        return Err(format!("the glTF refers to a file outside its folder: {name}"));
    }
    let path = dir.join(rel);
    let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_FILE as u64 {
        return Err(format!("{} is too large", path.display()));
    }
    std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b'%'
            && let Some(h) = b.get(i + 1..i + 3)
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(h).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(c);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse a `.glb`/`.vrm` (binary) or `.gltf` (JSON) file.
pub(super) fn parse(bytes: &[u8], dir: Option<&Path>) -> Result<Doc> {
    if bytes.len() > MAX_FILE {
        return Err("the model file exceeds 256 MiB".into());
    }
    let (json_bytes, bin): (&[u8], Option<&[u8]>) = if bytes.starts_with(b"glTF") {
        if u32_at(bytes, 4) != Some(2) {
            return Err("only glTF 2.0 binaries are supported".into());
        }
        let total = (u32_at(bytes, 8).ok_or("truncated GLB")? as usize).min(bytes.len());
        let (mut at, mut json, mut bin) = (12usize, None, None);
        while at + 8 <= total {
            let len = u32_at(bytes, at).ok_or("truncated GLB chunk")? as usize;
            let kind = u32_at(bytes, at + 4).ok_or("truncated GLB chunk")?;
            let body = bytes.get(at + 8..at.saturating_add(8).saturating_add(len)).ok_or("a GLB chunk runs past the end")?;
            match kind {
                0x4E4F_534A if json.is_none() => json = Some(body),
                0x004E_4942 if bin.is_none() => bin = Some(body),
                _ => {}
            }
            at = at.saturating_add(8).saturating_add(len).saturating_add(3) & !3;
        }
        (json.ok_or("the GLB has no JSON chunk")?, bin)
    } else {
        (bytes, None)
    };
    if json_bytes.len() > MAX_JSON {
        return Err("the glTF JSON exceeds 64 MiB".into());
    }
    let text = std::str::from_utf8(json_bytes).map_err(|_| "the glTF JSON is not UTF-8")?;
    let json: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("not a glTF file ({e})"))?;
    let version = json.get("asset").and_then(|a| a.get("version")).and_then(Value::as_str).unwrap_or("");
    if !version.starts_with('2') {
        return Err("only glTF 2.0 is supported".into());
    }
    let mut buffers = Vec::new();
    for (i, b) in arr(&json, "buffers").iter().enumerate() {
        let data = match b.get("uri").and_then(Value::as_str) {
            Some(uri) => uri_bytes(uri, dir)?,
            None if i == 0 => bin.ok_or("the glTF's first buffer has no data")?.to_vec(),
            None => return Err("a glTF buffer has no data".into()),
        };
        let want = b.get("byteLength").and_then(Value::as_u64).unwrap_or(0) as usize;
        if data.len() < want {
            return Err("a glTF buffer is shorter than it says".into());
        }
        buffers.push(data);
    }
    Ok(Doc { json, buffers })
}

/// `json[key]` as an array (empty when missing).
pub(super) fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

pub(super) fn index(v: &Value, key: &str) -> Option<usize> {
    v.get(key).and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok())
}

pub(super) fn floats<const N: usize>(v: &Value, key: &str) -> Option<[f64; N]> {
    let a = v.get(key)?.as_array()?;
    if a.len() != N {
        return None;
    }
    let mut out = [0.0; N];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x.as_f64().filter(|x| x.is_finite())?;
    }
    Some(out)
}

impl Doc {
    /// The bytes of buffer view `i` (offset applied) and its byte stride, if any.
    fn view(&self, i: usize) -> Result<(&[u8], Option<usize>)> {
        let v = arr(&self.json, "bufferViews").get(i).ok_or("a glTF accessor refers to a missing buffer view")?;
        let buffer = self.buffers.get(index(v, "buffer").ok_or("a buffer view has no buffer")?).ok_or("a buffer view refers to a missing buffer")?;
        let off = index(v, "byteOffset").unwrap_or(0);
        let len = index(v, "byteLength").ok_or("a buffer view has no length")?;
        let bytes = buffer.get(off..off.checked_add(len).ok_or("buffer view overflow")?).ok_or("a buffer view runs past its buffer")?;
        Ok((bytes, index(v, "byteStride").filter(|s| *s > 0)))
    }

    /// The raw bytes of buffer view `i`.
    pub(super) fn view_bytes(&self, i: usize) -> Result<&[u8]> {
        Ok(self.view(i)?.0)
    }

    /// Accessor `i` as `count` rows of `width` floats (normalized integers scaled to 0..1 or
    /// −1..1, plain integers as their values). `width` must match the accessor's type.
    pub(super) fn read(&self, i: usize, width: usize) -> Result<Vec<f64>> {
        let a = arr(&self.json, "accessors").get(i).ok_or("a glTF mesh refers to a missing accessor")?;
        let count = index(a, "count").ok_or("an accessor has no count")?;
        let comps = match a.get("type").and_then(Value::as_str) {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4") => 4,
            Some("MAT4") => 16,
            _ => return Err("an accessor has an unsupported type".into()),
        };
        if comps != width {
            return Err("an accessor has the wrong shape".into());
        }
        let ctype = a.get("componentType").and_then(Value::as_u64).unwrap_or(0);
        let size = match ctype {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return Err("an accessor has an unsupported component type".into()),
        };
        let normalized = a.get("normalized").and_then(Value::as_bool).unwrap_or(false);
        let total = count.checked_mul(comps).filter(|n| *n <= 64_000_000).ok_or("an accessor is too large")?;
        let mut out = vec![0.0; total];
        let Some(view) = index(a, "bufferView") else { return Ok(out) };
        let (bytes, stride) = self.view(view)?;
        let base = index(a, "byteOffset").unwrap_or(0);
        let elem = size * comps;
        let stride = stride.unwrap_or(elem);
        for r in 0..count {
            let start = r.checked_mul(stride).and_then(|s| s.checked_add(base)).ok_or("accessor overflow")?;
            let row = bytes.get(start..start + elem).ok_or("an accessor runs past its buffer view")?;
            for c in 0..comps {
                let b = row.get(c * size..c * size + size).ok_or("accessor overflow")?;
                let v = match ctype {
                    5120 => {
                        let x = f64::from(b[0] as i8);
                        if normalized { (x / 127.0).max(-1.0) } else { x }
                    }
                    5121 => {
                        let x = f64::from(b[0]);
                        if normalized { x / 255.0 } else { x }
                    }
                    5122 => {
                        let x = f64::from(i16::from_le_bytes([b[0], b[1]]));
                        if normalized { (x / 32767.0).max(-1.0) } else { x }
                    }
                    5123 => {
                        let x = f64::from(u16::from_le_bytes([b[0], b[1]]));
                        if normalized { x / 65535.0 } else { x }
                    }
                    5125 => f64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                    _ => f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                };
                if let Some(o) = out.get_mut(r * comps + c) {
                    *o = if v.is_finite() { v } else { 0.0 };
                }
            }
        }
        Ok(out)
    }
}

pub(super) const IDENTITY: M4 = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// Column-major product `a·b`.
pub(super) fn mul(a: &M4, b: &M4) -> M4 {
    let mut o = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            let mut s = 0.0;
            for k in 0..4 {
                s += a[k * 4 + r] * b[c * 4 + k];
            }
            o[c * 4 + r] = s;
        }
    }
    o
}

pub(super) fn apply(m: &M4, p: [f64; 3]) -> [f64; 3] {
    [m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12], m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13], m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14]]
}

pub(super) fn apply_dir(m: &M4, p: [f64; 3]) -> [f64; 3] {
    [m[0] * p[0] + m[4] * p[1] + m[8] * p[2], m[1] * p[0] + m[5] * p[1] + m[9] * p[2], m[2] * p[0] + m[6] * p[1] + m[10] * p[2]]
}

/// A node's local matrix: `matrix`, or translation · rotation (x, y, z, w) · scale.
pub(super) fn local(node: &Value) -> M4 {
    if let Some(m) = floats::<16>(node, "matrix") {
        return m;
    }
    let t = floats::<3>(node, "translation").unwrap_or([0.0; 3]);
    let q = floats::<4>(node, "rotation").unwrap_or([0.0, 0.0, 0.0, 1.0]);
    let s = floats::<3>(node, "scale").unwrap_or([1.0; 3]);
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    let [x, y, z, w] = if n > 1e-12 { [q[0] / n, q[1] / n, q[2] / n, q[3] / n] } else { [0.0, 0.0, 0.0, 1.0] };
    let r = [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + z * w),
        2.0 * (x * z - y * w),
        2.0 * (x * y - z * w),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z + x * w),
        2.0 * (x * z + y * w),
        2.0 * (y * z - x * w),
        1.0 - 2.0 * (x * x + y * y),
    ];
    [r[0] * s[0], r[1] * s[0], r[2] * s[0], 0.0, r[3] * s[1], r[4] * s[1], r[5] * s[1], 0.0, r[6] * s[2], r[7] * s[2], r[8] * s[2], 0.0, t[0], t[1], t[2], 1.0]
}

/// The rotation of a matrix with its scale (and any mirroring) taken out, as w, x, y, z.
pub(super) fn rotation_of(m: &M4) -> [f64; 4] {
    let col = |c: usize| [m[c * 4], m[c * 4 + 1], m[c * 4 + 2]];
    let norm = |v: [f64; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if l > 1e-12 { [v[0] / l, v[1] / l, v[2] / l] } else { v }
    };
    let (x, y, mut z) = (norm(col(0)), norm(col(1)), norm(col(2)));
    // Mirrored: flip one axis so the rest is a rotation.
    let det = x[0] * (y[1] * z[2] - y[2] * z[1]) - y[0] * (x[1] * z[2] - x[2] * z[1]) + z[0] * (x[1] * y[2] - x[2] * y[1]);
    if det < 0.0 {
        z = [-z[0], -z[1], -z[2]];
    }
    let (m00, m11, m22) = (x[0], y[1], z[2]);
    let tr = m00 + m11 + m22;
    let q = if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        [0.25 * s, (y[2] - z[1]) / s, (z[0] - x[2]) / s, (x[1] - y[0]) / s]
    } else if m00 > m11 && m00 > m22 {
        let s = (1.0 + m00 - m11 - m22).max(1e-12).sqrt() * 2.0;
        [(y[2] - z[1]) / s, 0.25 * s, (y[0] + x[1]) / s, (z[0] + x[2]) / s]
    } else if m11 > m22 {
        let s = (1.0 + m11 - m00 - m22).max(1e-12).sqrt() * 2.0;
        [(z[0] - x[2]) / s, (y[0] + x[1]) / s, 0.25 * s, (z[1] + y[2]) / s]
    } else {
        let s = (1.0 + m22 - m00 - m11).max(1e-12).sqrt() * 2.0;
        [(x[1] - y[0]) / s, (z[0] + x[2]) / s, (z[1] + y[2]) / s, 0.25 * s]
    };
    prism_doc::figure::normalize_quat(q)
}
