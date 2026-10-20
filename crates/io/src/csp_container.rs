//! Read-only C2F framing, independently implemented from local format observations.
//! RustCrypto supplies Blowfish; the material format uses little-endian words and CBC-CTS.
use blowfish::{
    BlowfishLE,
    cipher::{BlockDecrypt, KeyInit, generic_array::GenericArray},
};
const LIMIT: usize = 64 << 20;
pub(crate) const MAGIC: &[u8] = b"\x89C2F\r\n\x1a\n";
// Material-format constant, not an account credential. See docs/sut-analysis.md.
const MATERIAL_KEY: &[u8] = b"6E6EA6D8F1804bc6A80C072C8C4EB07C";
type Result<T> = std::result::Result<T, String>;
fn invalid() -> String {
    "invalid or unsupported CLIP STUDIO material container".into()
}
fn take(b: &[u8], at: usize, n: usize) -> Result<&[u8]> {
    b.get(at..at.checked_add(n).ok_or_else(invalid)?).ok_or_else(invalid)
}
fn u32le(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(take(b, at, 4)?.try_into().map_err(|_| invalid())?))
}
fn crc(b: &[u8]) -> u32 {
    let mut c = !0u32;
    for byte in b {
        c ^= u32::from(*byte);
        for _ in 0..8 {
            c = (c >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(c & 1));
        }
    }
    !c
}
fn block(cipher: &BlowfishLE, data: &[u8]) -> Result<[u8; 8]> {
    let mut b: [u8; 8] = data.try_into().map_err(|_| invalid())?;
    cipher.decrypt_block(GenericArray::from_mut_slice(&mut b));
    Ok(b)
}
fn xor(a: &mut [u8], b: &[u8]) {
    for (x, y) in a.iter_mut().zip(b) {
        *x ^= y;
    }
}
fn decrypt(data: &[u8], ctl: &prism_raster::Interrupt) -> Result<Vec<u8>> {
    let mut iv: [u8; 8] = take(data, 0, 8)?.try_into().map_err(|_| invalid())?;
    let data = data.get(8..).filter(|b| b.len() >= 8).ok_or_else(invalid)?;
    let cipher = BlowfishLE::new_from_slice(MATERIAL_KEY).map_err(|_| invalid())?;
    // Ciphertext stealing always swaps the last two blocks (CBC-CS3), also when the length is
    // a whole number of blocks: undo that swap, then the last block is a plain CBC block.
    let swapped;
    let data = if data.len() >= 16 && data.len().is_multiple_of(8) {
        let n = data.len();
        let mut d = data.to_vec();
        d.get_mut(n - 16..).ok_or_else(invalid)?.copy_from_slice(&[take(data, n - 8, 8)?, take(data, n - 16, 8)?].concat());
        swapped = d;
        swapped.as_slice()
    } else {
        data
    };
    let rem = data.len() % 8;
    let normal = if rem == 0 { data.len() } else { data.len() - rem - 8 };
    let mut out = Vec::with_capacity(data.len());
    for (i, c) in take(data, 0, normal)?.chunks_exact(8).enumerate() {
        if i.is_multiple_of(512) {
            ctl.check().map_err(|e| e.to_string())?;
        }
        let mut p = block(&cipher, c)?;
        xor(&mut p, &iv);
        out.extend_from_slice(&p);
        iv.copy_from_slice(c);
    }
    if rem != 0 {
        let last = block(&cipher, take(data, normal, 8)?)?;
        let mut previous = last;
        previous.get_mut(..rem).ok_or_else(invalid)?.copy_from_slice(take(data, normal + 8, rem)?);
        let mut p = block(&cipher, &previous)?;
        xor(&mut p, &iv);
        out.extend_from_slice(&p);
        out.extend(last.iter().zip(previous).take(rem).map(|(a, b)| a ^ b));
    }
    Ok(out)
}
/// Decode a complete container; validate every chunk, checksum and terminal marker.
pub(crate) fn decode(bytes: &[u8], ctl: &prism_raster::Interrupt) -> Result<Vec<u8>> {
    if bytes.len() > LIMIT || !bytes.starts_with(MAGIC) {
        return Err(invalid());
    }
    let mut at = MAGIC.len();
    let mut out = Vec::new();
    let mut head = false;
    let mut data_seen = false;
    for _ in 0..4096 {
        ctl.check().map_err(|e| e.to_string())?;
        let n = u32le(bytes, at)? as usize;
        let tagged = take(bytes, at + 4, n.checked_add(4).ok_or_else(invalid)?)?;
        let next = at.checked_add(12).and_then(|p| p.checked_add(n)).ok_or_else(invalid)?;
        if crc(tagged) != u32le(bytes, next - 4)? {
            return Err("CLIP STUDIO material checksum mismatch".into());
        }
        let tag = tagged.get(..4).ok_or_else(invalid)?;
        let data = tagged.get(4..).ok_or_else(invalid)?;
        match tag {
            b"HEAD" if !head && at == MAGIC.len() && n == 0 => head = true,
            b"dATA" if head => {
                let encoding = take(data, 0, 2)?;
                let payload = data.get(2..).ok_or_else(invalid)?;
                match encoding {
                    [0, 0] => out.extend_from_slice(payload),
                    [1, 0] => out.extend_from_slice(&decrypt(payload, ctl)?),
                    _ => return Err("unsupported CLIP STUDIO material encoding".into()),
                }
                if out.len() > LIMIT {
                    return Err(invalid());
                }
                data_seen = true;
            }
            b"TAIL" if head && data_seen && n == 0 && next == bytes.len() => return Ok(out),
            _ => return Err(invalid()),
        }
        at = next;
    }
    Err(invalid())
}
#[cfg(test)]
pub(crate) fn plain_fixture(payload: &[u8]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    for (tag, data) in [(b"HEAD", vec![]), (b"dATA", [vec![0, 0], payload.to_vec()].concat()), (b"TAIL", vec![])] {
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        let tagged = [tag.as_slice(), &data].concat();
        out.extend_from_slice(&tagged);
        out.extend_from_slice(&crc(&tagged).to_le_bytes());
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_blowfish_vectors_cover_full_and_stolen_blocks() {
        // Generated with PyCryptodome's ECB primitive and explicit word-byte reversal.
        for (n, h) in [
            (8, "3132333435363738103af4969527ad5c"),
            (9, "31323334353637382c4d6ff679cfe83610"),
            (15, "3132333435363738ef429fc03ccebea4103af4969527ad"),
            // Whole blocks: the last two are swapped (CS3), as in downloaded `.layer` files.
            (16, "3132333435363738ff4e7cd3db5cbefc103af4969527ad5c"),
            (17, "3132333435363738103af4969527ad5c9a2b18332c836d62ff"),
            (31, "3132333435363738103af4969527ad5cff4e7cd3db5cbefc0c5a3d957a4f67f0137be80cbf3d2e"),
        ] {
            let b: Vec<_> = h.as_bytes().chunks_exact(2).map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap()).collect();
            assert_eq!(decrypt(&b, &prism_raster::Interrupt::NONE).unwrap(), (0..n).collect::<Vec<u8>>());
        }
    }
    #[test]
    fn framing_rejects_damage_and_truncation() {
        let f = plain_fixture(b"synthetic material");
        assert_eq!(decode(&f, &prism_raster::Interrupt::NONE).unwrap(), b"synthetic material");
        for n in 0..f.len() {
            assert!(decode(&f[..n], &prism_raster::Interrupt::NONE).is_err());
        }
        for i in 0..f.len() {
            let mut b = f.clone();
            b[i] ^= 128;
            assert!(decode(&b, &prism_raster::Interrupt::NONE).is_err());
        }
        let mut b = f;
        b.push(0);
        assert!(decode(&b, &prism_raster::Interrupt::NONE).is_err());
    }
}
