//! CLIP STUDIO gradients (`GradationData`, Tool Property item 5400): the colour stops of a
//! gradient sub-tool, decoded from the blob CLIP STUDIO stores, and the stops Prism paints them
//! with. Layout evidence: docs/sut-analysis.md › Gradient data (`GradationData`).
use serde::{Deserialize, Serialize};

/// Defensive limits, not claimed CLIP STUDIO limits.
pub const MAX_STOPS: usize = 256;
pub const MAX_CURVE_POINTS: usize = 64;
/// A stop's position is a fixed-point fraction of the bar: 0x8000 is its end.
const POSITION_ONE: f64 = 32768.0;
/// Colour channels and opacity are 32-bit (0xFFFFFFFF is full).
const CHANNEL_MAX: f64 = 4_294_967_295.0;
/// The fewest bytes of the header, a stop record and a curve point.
const MIN_SIZES: (usize, usize, usize) = (24, 28, 16);
/// The most stops Prism's own gradient takes from one CLIP STUDIO gradient (colour and opacity
/// stops each; a curve adds samples).
const MAX_PRISM_STOPS: usize = 1024;
/// Where a sampled curve's last stop sits before the next stop, as a fraction of the segment.
const END_GAP: f32 = 1e-4;

/// A stop colour: the drawing colours CLIP STUDIO follows live, or a fixed colour.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CspStopColor {
    /// The main drawing colour (Prism's foreground).
    Foreground,
    /// The sub drawing colour (Prism's background).
    Background,
    Rgb([f32; 3]),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CspGradientStop {
    /// 0..=1 along the gradient.
    pub position: f32,
    pub color: CspStopColor,
    /// 0..=1.
    pub opacity: f32,
    /// How colour and opacity run to the next stop: `[x, y]` points (x along the segment, y the
    /// share of the next stop), sorted by x. Empty runs straight.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub curve: Vec<[f32; 2]>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CspGradient {
    /// At least two, sorted by position.
    pub stops: Vec<CspGradientStop>,
}

/// The stops Prism's Gradient tool paints a CLIP STUDIO gradient with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrismStops {
    pub colors: Vec<(f32, CspStopColor)>,
    /// `(position, opacity 0..1)`.
    pub opacity: Vec<(f32, f32)>,
    /// A curve between a drawing colour and another colour runs straight: Prism's stops follow
    /// the drawing colours live and cannot hold a mix of one.
    pub approximated: bool,
}

fn word(bytes: &[u8], at: usize) -> Result<u32, &'static str> {
    let b: [u8; 4] = at.checked_add(4).and_then(|end| bytes.get(at..end)).and_then(|b| b.try_into().ok()).ok_or("truncated gradient data")?;
    Ok(u32::from_be_bytes(b))
}

fn double(bytes: &[u8], at: usize) -> Result<f64, &'static str> {
    let b: [u8; 8] = at.checked_add(8).and_then(|end| bytes.get(at..end)).and_then(|b| b.try_into().ok()).ok_or("truncated gradient curve")?;
    Ok(f64::from_be_bytes(b))
}

fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl CspGradient {
    /// Decode `GradationData`: a header of big-endian words (header size, stop record size, stop
    /// count, curve point size, then two words of unknown use), the stop records (red, green,
    /// blue, opacity as 32-bit values; a flags word whose low nibble picks the colour, 0 fixed,
    /// 1 main, 2 sub; the position, 0x8000 = 1; the number of curve points to the next stop),
    /// then every stop's curve points in stop order (x, y as big-endian doubles).
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        let size = |at: usize| word(bytes, at).map(|w| w as usize);
        let (header, record, count, point) = (size(0)?, size(4)?, size(8)?, size(12)?);
        if header < MIN_SIZES.0 || record < MIN_SIZES.1 || point < MIN_SIZES.2 || header.max(record).max(point) > 4096 {
            return Err("unsupported gradient data header");
        }
        if !(2..=MAX_STOPS).contains(&count) {
            return Err("gradient stop count out of range");
        }
        let mut stops = Vec::with_capacity(count);
        let mut points = Vec::with_capacity(count);
        let mut at = header;
        for _ in 0..count {
            let w = |k: usize| word(bytes, at.saturating_add(k * 4));
            let channel = |v: u32| (f64::from(v) / CHANNEL_MAX) as f32;
            let rgb = [channel(w(0)?), channel(w(1)?), channel(w(2)?)];
            let color = match w(4)? & 0xf {
                1 => CspStopColor::Foreground,
                2 => CspStopColor::Background,
                _ => CspStopColor::Rgb(rgb),
            };
            let n = w(6)? as usize;
            if n > MAX_CURVE_POINTS {
                return Err("too many gradient curve points");
            }
            stops.push(CspGradientStop { position: (f64::from(w(5)?) / POSITION_ONE).min(1.0) as f32, color, opacity: channel(w(3)?), curve: Vec::new() });
            points.push(n);
            at = at.checked_add(record).ok_or("truncated gradient data")?;
        }
        for (stop, n) in stops.iter_mut().zip(points) {
            for _ in 0..n {
                let (x, y) = (double(bytes, at)?, double(bytes, at.saturating_add(8))?);
                if !x.is_finite() || !y.is_finite() {
                    return Err("invalid gradient curve point");
                }
                stop.curve.push([x as f32, y as f32]);
                at = at.checked_add(point).ok_or("truncated gradient curve")?;
            }
        }
        CspGradient { stops }.sanitized().ok_or("invalid gradient stops")
    }

    /// This gradient with values clamped to range, stops and curve points sorted and capped;
    /// `None` without two stops.
    pub fn sanitized(mut self) -> Option<Self> {
        self.stops.truncate(MAX_STOPS);
        for s in &mut self.stops {
            s.position = unit(s.position);
            s.opacity = unit(s.opacity);
            if let CspStopColor::Rgb(c) = &mut s.color {
                *c = c.map(unit);
            }
            s.curve.truncate(MAX_CURVE_POINTS);
            for p in &mut s.curve {
                *p = p.map(unit);
            }
            s.curve.sort_by(|a, b| a[0].total_cmp(&b[0]));
        }
        self.stops.sort_by(|a, b| a.position.total_cmp(&b.position));
        (self.stops.len() >= 2).then_some(self)
    }

    /// Whether every value is in range and sorted (what [`Self::sanitized`] keeps unchanged).
    pub fn is_valid(&self) -> bool {
        self.clone().sanitized().as_ref() == Some(self)
    }

    /// The same gradient end to start (CLIP STUDIO's Reverse).
    pub fn reversed(&self) -> Self {
        let n = self.stops.len();
        let stops = (0..n)
            .rev()
            .filter_map(|i| {
                let s = self.stops.get(i)?;
                // The segment that now leaves this stop was the previous stop's, read backwards.
                let mut curve: Vec<[f32; 2]> = i.checked_sub(1).and_then(|p| self.stops.get(p)).map(|p| p.curve.iter().map(|[x, y]| [1.0 - x, 1.0 - y]).collect()).unwrap_or_default();
                curve.sort_by(|a, b| a[0].total_cmp(&b[0]));
                Some(CspGradientStop { position: 1.0 - s.position, color: s.color, opacity: s.opacity, curve })
            })
            .collect();
        CspGradient { stops }
    }

    /// The colour (red, green, blue, alpha) at `t` with the drawing colours `fg` and `bg`,
    /// following the curves exactly.
    pub fn sample(&self, t: f32, fg: [f32; 3], bg: [f32; 3]) -> [f32; 4] {
        let rgb = |c: CspStopColor| match c {
            CspStopColor::Foreground => fg,
            CspStopColor::Background => bg,
            CspStopColor::Rgb(c) => c,
        };
        let at = |s: &CspGradientStop| {
            let c = rgb(s.color);
            [c[0], c[1], c[2], s.opacity]
        };
        let t = unit(t);
        let Some(first) = self.stops.first() else { return [fg[0], fg[1], fg[2], 1.0] };
        if t <= first.position {
            return at(first);
        }
        for w in self.stops.windows(2) {
            let [a, b] = w else { continue };
            // A stop's own position shows that stop (the end of a step).
            if t < b.position {
                let span = b.position - a.position;
                let x = if span > 1e-6 { (t - a.position) / span } else { 1.0 };
                let y = curve_at(&a.curve, x);
                let (ca, cb) = (at(a), at(b));
                return std::array::from_fn(|i| lerp(ca[i], cb[i], y));
            }
        }
        self.stops.last().map_or([fg[0], fg[1], fg[2], 1.0], at)
    }

    /// Colour and opacity stops for Prism's Gradient tool. Straight segments map one to one; a
    /// curve becomes sampled stops (exact for opacity and for fixed colours, and for steps).
    pub fn prism_stops(&self) -> PrismStops {
        let mut out = PrismStops::default();
        let n = self.stops.len().max(1);
        let samples = (MAX_PRISM_STOPS / n).saturating_sub(1).clamp(1, 16);
        for (i, s) in self.stops.iter().enumerate() {
            out.colors.push((s.position, s.color));
            out.opacity.push((s.position, s.opacity));
            let Some(next) = self.stops.get(i + 1) else { break };
            if is_straight(&s.curve) {
                continue;
            }
            let span = next.position - s.position;
            // The last sample sits just before the next stop: Prism's stops cannot hold two values
            // at one position, so a step's edge is a hair wide.
            let end = |x: f32| if x >= 1.0 { 1.0 - END_GAP } else { x };
            let curve: Vec<(f32, f32)> = (1..=samples).map(|k| k as f32 / samples as f32).map(|x| (s.position + end(x) * span, curve_at(&s.curve, x))).collect();
            out.opacity.extend(curve.iter().map(|(t, y)| (*t, lerp(s.opacity, next.opacity, *y))));
            let colors: Option<Vec<(f32, CspStopColor)>> = curve.iter().map(|(t, y)| mix(s.color, next.color, *y).map(|c| (*t, c))).collect();
            match colors {
                Some(c) => out.colors.extend(c),
                None => out.approximated = true,
            }
        }
        prune(&mut out.colors);
        prune(&mut out.opacity);
        out
    }
}

/// `a` to `b` at `y` as one stop colour, when one can hold it.
fn mix(a: CspStopColor, b: CspStopColor, y: f32) -> Option<CspStopColor> {
    const EPS: f32 = 1e-4;
    match (a, b) {
        _ if y <= EPS || a == b => Some(a),
        _ if y >= 1.0 - EPS => Some(b),
        (CspStopColor::Rgb(a), CspStopColor::Rgb(b)) => Some(CspStopColor::Rgb(std::array::from_fn(|i| lerp(a[i], b[i], y)))),
        _ => None,
    }
}

/// No curve, or one that runs straight from the stop to the next.
fn is_straight(curve: &[[f32; 2]]) -> bool {
    curve.iter().all(|[x, y]| (x - y).abs() < 1e-4) && curve.first().is_none_or(|p| p[0] <= 1e-4) && curve.last().is_none_or(|p| p[0] >= 1.0 - 1e-4)
}

/// Drop repeated stops and the middle of runs of one value.
fn prune<V: PartialEq + Copy>(v: &mut Vec<(f32, V)>) {
    v.dedup_by(|b, a| a.0 == b.0 && a.1 == b.1);
    let keep: Vec<bool> = (0..v.len())
        .map(|i| {
            let (prev, cur, next) = (i.checked_sub(1).and_then(|p| v.get(p)), v.get(i), v.get(i + 1));
            !matches!((prev, cur, next), (Some(p), Some(c), Some(n)) if p.1 == c.1 && n.1 == c.1)
        })
        .collect();
    let mut keep = keep.into_iter();
    v.retain(|_| keep.next().unwrap_or(true));
}

/// The curve's y at `x` (0..1): straight with no points, flat outside them, otherwise a monotone
/// cubic through them (an inference: CLIP STUDIO's interpolation is not traced).
pub fn curve_at(points: &[[f32; 2]], x: f32) -> f32 {
    let x = unit(x);
    let (Some(first), Some(last)) = (points.first(), points.last()) else { return x };
    if x <= first[0] {
        return unit(first[1]);
    }
    if x >= last[0] {
        return unit(last[1]);
    }
    let slope = |k: usize| -> Option<f32> {
        let (a, b) = (points.get(k)?, points.get(k + 1)?);
        let h = b[0] - a[0];
        Some(if h > 1e-6 { (b[1] - a[1]) / h } else { 0.0 })
    };
    // Fritsch–Carlson tangents: the mean of the neighbouring slopes, flat at a turn.
    let tangent = |k: usize| -> f32 {
        match (k.checked_sub(1).and_then(slope), slope(k)) {
            (Some(a), Some(b)) if a * b > 0.0 => (a + b) / 2.0,
            (Some(_), Some(_)) => 0.0,
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => 0.0,
        }
    };
    for k in 0..points.len().saturating_sub(1) {
        let (Some(a), Some(b)) = (points.get(k), points.get(k + 1)) else { break };
        if x > b[0] {
            continue;
        }
        let h = b[0] - a[0];
        if h <= 1e-6 {
            return unit(b[1]);
        }
        let d = (b[1] - a[1]) / h;
        let (mut m0, mut m1) = (tangent(k), tangent(k + 1));
        if d == 0.0 {
            (m0, m1) = (0.0, 0.0);
        } else {
            // Keep the piece monotone.
            let (al, be) = (m0 / d, m1 / d);
            let r = al * al + be * be;
            if r > 9.0 {
                let s = 3.0 / r.sqrt();
                (m0, m1) = (s * al * d, s * be * d);
            }
        }
        let t = (x - a[0]) / h;
        let (t2, t3) = (t * t, t * t * t);
        let y = (2.0 * t3 - 3.0 * t2 + 1.0) * a[1] + (t3 - 2.0 * t2 + t) * h * m0 + (-2.0 * t3 + 3.0 * t2) * b[1] + (t3 - t2) * h * m1;
        return unit(y);
    }
    unit(last[1])
}

#[cfg(test)]
#[path = "csp_gradient_tests.rs"]
mod tests;
