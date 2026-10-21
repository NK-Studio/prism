//! Bounded SUT effector decoding. Serialized layout evidence is in docs/sut-analysis.md.
//! Curve coordinates are data; the independent evaluator lives in prism-paint.

use prism_paint::{Control, CspTilt, Dynamic, PressureCurve};

pub(super) struct Effector {
    /// Pen pressure and/or tilt response; `None` when neither is active.
    pub dynamic: Option<Dynamic>,
    pub other_controls: u32,
    /// Why an active tilt response was dropped.
    pub tilt_error: Option<String>,
}

/// Active-control bits shared by the supported and active masks.
const PRESSURE: u32 = 0x10;
const TILT: u32 = 0x20;
/// Random: a per-dab factor between the minimum % in word 24 and 100 %.
const RANDOM: u32 = 0x80;

fn word(blob: &[u8], at: usize) -> Result<u32, String> {
    let end = at.checked_add(4).ok_or("SUT effector offset overflow")?;
    let bytes = blob.get(at..end).and_then(|b| <[u8; 4]>::try_from(b).ok()).ok_or("truncated SUT effector")?;
    Ok(u32::from_be_bytes(bytes))
}

fn length(blob: &[u8], at: usize) -> Result<usize, String> {
    usize::try_from(word(blob, at)?).map_err(|_| "SUT effector length exceeds address space".into())
}

fn curve(blob: &[u8]) -> Result<PressureCurve, String> {
    let header = length(blob, 0)?;
    let count = length(blob, 4)?;
    let stride = length(blob, 8)?;
    if !(2..=16).contains(&count) || header < 12 || stride < 16 {
        return Err("invalid SUT pressure curve header, point count or stride".into());
    }
    let end = count.checked_mul(stride).and_then(|n| header.checked_add(n)).ok_or("SUT pressure curve length overflow")?;
    if end != blob.len() {
        return Err("SUT pressure curve length does not match its points".into());
    }
    let mut points = Vec::with_capacity(count);
    for record in blob.get(header..end).ok_or("truncated SUT pressure curve")?.chunks_exact(stride) {
        let coordinate = |at: usize| -> Result<f64, String> {
            let bytes = record.get(at..at + 8).and_then(|b| <[u8; 8]>::try_from(b).ok()).ok_or("truncated SUT pressure curve coordinate")?;
            Ok(f64::from_be_bytes(bytes))
        };
        points.push([coordinate(0)?, coordinate(8)?]);
    }
    PressureCurve::new(points)
}

/// The active input controls of an effector and its random minimum in percent, signed: colour
/// jitter stores -100 (a random factor between -1 and 1). Size and the other controls read by
/// [`read`] use 0..100.
pub(super) fn controls(blob: &[u8]) -> Result<(u32, i32), String> {
    let header = length(blob, 0)?;
    if !(40..=65_536).contains(&header) || header > blob.len() {
        return Err("invalid SUT effector header length".into());
    }
    let random_min = i32::from_be_bytes(word(blob, 24)?.to_be_bytes()).clamp(-100, 100);
    Ok((word(blob, 4)? & word(blob, 8)?, random_min))
}

/// Whether an effector varies its value with any input (pressure, tilt, speed, random…).
pub(super) fn is_dynamic(blob: &[u8]) -> bool {
    controls(blob).is_ok_and(|(active, _)| active != 0)
}

pub(super) const RANDOM_CONTROL: u32 = RANDOM;

/// Header layout (big-endian u32): 0 header length, 4 supported controls, 8 active controls,
/// 12 pressure minimum %, 16 tilt value % at response 0, 20/24 speed/random minimums %, 32/36/44
/// curve block lengths (pressure, tilt, speed), 40 tilt value % at response 1 (100 before 44-byte
/// headers). Curve blocks follow the header in that order.
pub(super) fn read(blob: &[u8]) -> Result<Effector, String> {
    // CSP writes 40/44/48-byte headers. Cap future extensions before processing them.
    let header = length(blob, 0)?;
    if !(40..=65_536).contains(&header) || header > blob.len() || !header.is_multiple_of(4) {
        return Err("invalid SUT effector header length".into());
    }
    let active = word(blob, 4)? & word(blob, 8)?;
    let other_controls = active & !(PRESSURE | TILT | RANDOM);
    let random = if active & RANDOM != 0 { Some(word(blob, 24)?.min(100) as f32 / 100.0) } else { None };
    let pressure_len = length(blob, 32)?;
    let tilt_len = length(blob, 36)?;
    let third_len = if header >= 48 { length(blob, 44)? } else { 0 };
    let pressure_end = header.checked_add(pressure_len).ok_or("SUT effector length overflow")?;
    let tilt_end = pressure_end.checked_add(tilt_len).ok_or("SUT effector length overflow")?;
    if active & (PRESSURE | TILT) == 0 {
        let dynamic = random.map(|m| Dynamic { csp_random: Some(m), ..Dynamic::default() });
        return Ok(Effector { dynamic, other_controls, tilt_error: None });
    }
    let end = tilt_end.checked_add(third_len).ok_or("SUT effector length overflow")?;
    if end != blob.len() {
        return Err("SUT effector length does not match its curve blocks".into());
    }
    let mut dynamic = None;
    if active & PRESSURE != 0 {
        let minimum = word(blob, 12)?;
        if minimum > 100 {
            return Err("SUT pressure minimum must be between 0 and 100".into());
        }
        let pressure_curve = if pressure_len == 0 { None } else { Some(curve(blob.get(header..pressure_end).ok_or("truncated SUT pressure curve")?)?) };
        dynamic = Some(Dynamic { minimum: minimum as f32 / 100.0, pressure_curve, ..Dynamic::controlled(Control::PenPressure) });
    }
    let mut tilt_error = None;
    // Without a curve block the source evaluator skips the tilt response.
    if active & TILT != 0 && tilt_len != 0 {
        let at_zero = word(blob, 16)?;
        let at_one = if header >= 44 { word(blob, 40)? } else { 100 };
        let tilt = curve(blob.get(pressure_end..tilt_end).ok_or("truncated SUT tilt curve")?)
            .and_then(|curve| CspTilt::new(at_zero as f32 / 100.0, at_one as f32 / 100.0, curve));
        match tilt {
            Ok(tilt) => dynamic.get_or_insert_with(Dynamic::default).csp_tilt = Some(tilt),
            Err(error) => tilt_error = Some(error),
        }
    }
    if let Some(m) = random {
        dynamic.get_or_insert_with(Dynamic::default).csp_random = Some(m);
    }
    Ok(Effector { dynamic, other_controls, tilt_error })
}
