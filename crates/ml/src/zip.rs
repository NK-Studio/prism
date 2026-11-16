//! Just enough ZIP reading to unpack the upstream package: the central directory, stored and
//! deflated entries, CRC-32 checks. No ZIP64, encryption or multi-disk archives (the pinned
//! packages use none). Every offset and length comes from the file, so every read is checked.

use std::io::Read;

use crate::{Error, Result};

const EOCD_SIG: u32 = 0x0605_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;
/// Largest entry unpacked (the biggest model is 33 MB).
pub const MAX_ENTRY_BYTES: u64 = 256 << 20;

fn bad(msg: impl Into<String>) -> Error {
    Error::Integrity(format!("the Real-ESRGAN package is damaged: {}", msg.into()))
}

fn u16_at(b: &[u8], at: usize) -> Result<u16> {
    let s = b.get(at..at.checked_add(2).ok_or_else(|| bad("offset overflow"))?).ok_or_else(|| bad("truncated"))?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    let s = b.get(at..at.checked_add(4).ok_or_else(|| bad("offset overflow"))?).ok_or_else(|| bad("truncated"))?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// One file in the archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    method: u16,
    crc: u32,
    compressed: u64,
    size: u64,
    local_offset: usize,
}

/// The archive's entries, from its central directory.
pub fn entries(zip: &[u8]) -> Result<Vec<Entry>> {
    // The end-of-central-directory record sits in the last 22 + 65535 bytes.
    let lo = zip.len().saturating_sub(22 + 0xFFFF);
    let eocd = (lo..=zip.len().saturating_sub(22)).rev().find(|&i| u32_at(zip, i).ok() == Some(EOCD_SIG)).ok_or_else(|| bad("no directory"))?;
    let count = u16_at(zip, eocd + 10)? as usize;
    let mut at = u32_at(zip, eocd + 16)? as usize;
    let mut out = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        if u32_at(zip, at)? != CENTRAL_SIG {
            return Err(bad("bad directory entry"));
        }
        let method = u16_at(zip, at.saturating_add(10))?;
        let crc = u32_at(zip, at.saturating_add(16))?;
        let compressed = u32_at(zip, at.saturating_add(20))? as u64;
        let size = u32_at(zip, at.saturating_add(24))? as u64;
        let name_len = u16_at(zip, at.saturating_add(28))? as usize;
        let extra_len = u16_at(zip, at.saturating_add(30))? as usize;
        let comment_len = u16_at(zip, at.saturating_add(32))? as usize;
        let local_offset = u32_at(zip, at.saturating_add(42))? as usize;
        let name_start = at.saturating_add(46);
        let name = zip.get(name_start..name_start.saturating_add(name_len)).ok_or_else(|| bad("truncated name"))?;
        if compressed == u32::MAX as u64 || size == u32::MAX as u64 || local_offset == u32::MAX as usize {
            return Err(bad("ZIP64 archives aren't supported"));
        }
        out.push(Entry { name: String::from_utf8_lossy(name).replace('\\', "/"), method, crc, compressed, size, local_offset });
        at = name_start.saturating_add(name_len).saturating_add(extra_len).saturating_add(comment_len);
    }
    Ok(out)
}

/// The unpacked bytes of `e`.
pub fn read(zip: &[u8], e: &Entry) -> Result<Vec<u8>> {
    if e.size > MAX_ENTRY_BYTES {
        return Err(bad(format!("{} is too large", e.name)));
    }
    let at = e.local_offset;
    if u32_at(zip, at)? != LOCAL_SIG {
        return Err(bad("bad local header"));
    }
    let start = at.saturating_add(30).saturating_add(u16_at(zip, at.saturating_add(26))? as usize).saturating_add(u16_at(zip, at.saturating_add(28))? as usize);
    let end = start.checked_add(e.compressed as usize).ok_or_else(|| bad("entry overflow"))?;
    let raw = zip.get(start..end).ok_or_else(|| bad(format!("{} is truncated", e.name)))?;
    let data = match e.method {
        0 => raw.to_vec(),
        8 => {
            let mut out = Vec::with_capacity(e.size as usize);
            flate2::read::DeflateDecoder::new(raw).take(e.size + 1).read_to_end(&mut out).map_err(|err| bad(format!("{}: {err}", e.name)))?;
            out
        }
        m => return Err(bad(format!("{} uses compression method {m}", e.name))),
    };
    if data.len() as u64 != e.size {
        return Err(bad(format!("{} has the wrong size", e.name)));
    }
    let mut crc = flate2::Crc::new();
    crc.update(&data);
    if crc.sum() != e.crc {
        return Err(bad(format!("{} fails its checksum", e.name)));
    }
    Ok(data)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// A minimal archive writer for tests: `(name, data, deflate?)`.
    pub fn build(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, deflate) in files {
            let body = if *deflate {
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.to_vec()
            };
            let mut crc = flate2::Crc::new();
            crc.update(data);
            let offset = out.len() as u32;
            let method: u16 = if *deflate { 8 } else { 0 };
            let header = |sig: u32, central: bool| {
                let mut h = Vec::new();
                h.extend(sig.to_le_bytes());
                if central {
                    h.extend(20u16.to_le_bytes());
                }
                h.extend(20u16.to_le_bytes());
                h.extend(0u16.to_le_bytes());
                h.extend(method.to_le_bytes());
                h.extend(0u32.to_le_bytes());
                h.extend(crc.sum().to_le_bytes());
                h.extend((body.len() as u32).to_le_bytes());
                h.extend((data.len() as u32).to_le_bytes());
                h.extend((name.len() as u16).to_le_bytes());
                h.extend(0u16.to_le_bytes());
                if central {
                    // comment length, disk, internal and external attributes
                    h.extend([0u8; 10]);
                    h.extend(offset.to_le_bytes());
                }
                h.extend(name.as_bytes());
                h
            };
            out.extend(header(LOCAL_SIG, false));
            out.extend(&body);
            central.extend(header(CENTRAL_SIG, true));
        }
        let cd_offset = out.len() as u32;
        out.extend(&central);
        out.extend(EOCD_SIG.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(cd_offset.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out
    }

    #[test]
    fn reads_stored_and_deflated_entries() {
        let big = vec![7u8; 5000];
        let z = build(&[("a.txt", b"hello", false), ("dir\\b.bin", &big, true)]);
        let es = entries(&z).unwrap();
        assert_eq!(es.len(), 2);
        assert_eq!(es[1].name, "dir/b.bin");
        assert_eq!(read(&z, &es[0]).unwrap(), b"hello");
        assert_eq!(read(&z, &es[1]).unwrap(), big);
    }

    #[test]
    fn damaged_archives_are_errors() {
        let z = build(&[("a.txt", b"hello", true)]);
        for cut in [0, 10, z.len() / 2, z.len() - 1] {
            let _ = entries(&z[..cut]).and_then(|es| es.iter().map(|e| read(&z[..cut], e)).collect::<Result<Vec<_>>>());
        }
        let mut flipped = z.clone();
        flipped[30 + 5 + 1] ^= 0xFF; // inside the compressed body (after the header and name)
        let es = entries(&flipped).unwrap();
        assert!(read(&flipped, &es[0]).is_err());
        assert!(entries(b"").is_err());
        assert!(entries(&[0u8; 100]).is_err());
    }
}
