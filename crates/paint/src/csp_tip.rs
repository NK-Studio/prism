//! CLIP STUDIO multi-tip order and colour brush tips.
//!
//! Evidence (docs/sut-analysis.md › Tip order, rotation input and colour tips): CSP picks the tip
//! of a dab from a per-stroke dab counter `n` and its tip count `c` by `BrushPatternOrderType`
//! (0 repeat `n mod c`, 1 reverse `0,1,…,c−1,c−2,…,1`, 2 do not repeat `min(n, c−1)`, 3 random).
//! A colour tip keeps its own colours unless the brush changes the tip colour.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::rng::{rand01, stream};
use crate::tile::{GrayTile, b64_decode, b64_encode};

/// How a multi-tip brush walks its tips (CSP's "Set how brush tip shapes repeat").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CspTipOrder {
    /// 0, 1, …, c−1, 0, 1, …
    #[default]
    Repeat,
    /// 0, 1, …, c−1, c−2, …, 1, 0, 1, … (round trip).
    Reverse,
    /// 0, 1, …, c−1, then the last tip for the rest of the stroke.
    Once,
    /// A uniformly random tip per dab.
    Random,
}

impl CspTipOrder {
    /// The order a stored `BrushPatternOrderType` code names (`None` for codes CSP does not save
    /// for brushes).
    pub fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            0 => Self::Repeat,
            1 => Self::Reverse,
            2 => Self::Once,
            3 => Self::Random,
            _ => return None,
        })
    }

    /// The tip of dab `index` among `count` tips (always `< count`, or 0 without tips).
    pub fn tip_index(self, index: u64, count: usize, seed: u64) -> usize {
        let c = count as u64;
        if c <= 1 {
            return 0;
        }
        let i = match self {
            Self::Repeat => index % c,
            Self::Reverse => {
                let period = c.saturating_mul(2) - 2;
                let k = index % period;
                if k >= c - 1 { period - k } else { k }
            }
            Self::Once => index.min(c - 1),
            Self::Random => ((rand01(seed, index, stream::TIP) * count as f32) as u64).min(c - 1),
        };
        usize::try_from(i).unwrap_or(0).min(count - 1)
    }
}

/// The straight RGB colours (8 bits a channel, row-major) of a colour tip, painted instead of the
/// brush colour; the tip's own [`GrayTile`] stays its shape (alpha).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CspTipColor {
    pub width: u32,
    pub height: u32,
    #[serde(serialize_with = "rgb_out", deserialize_with = "rgb_in")]
    pub rgb: Vec<u8>,
}

fn rgb_out<S: Serializer>(rgb: &[u8], s: S) -> Result<S::Ok, S::Error> {
    b64_encode(rgb).serialize(s)
}

fn rgb_in<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    use serde::de::Error;
    b64_decode(&String::deserialize(d)?).ok_or_else(|| D::Error::custom("invalid base64 in tip colours"))
}

impl CspTipColor {
    /// Whether these colours belong to `tip`: same size, three bytes a texel.
    pub fn fits(&self, tip: &GrayTile) -> bool {
        tip.is_valid() && self.width == tip.width && self.height == tip.height && Some(self.rgb.len()) == (tip.data.len()).checked_mul(3)
    }

    /// The red, green and blue planes premultiplied by the tip's alpha, so they filter (mip-map,
    /// interpolate) like the tip itself; `None` when the colours do not fit the tip.
    pub fn premultiplied(&self, tip: &GrayTile) -> Option<[GrayTile; 3]> {
        if !self.fits(tip) {
            return None;
        }
        let plane = |c: usize| GrayTile {
            width: tip.width,
            height: tip.height,
            data: tip
                .data
                .iter()
                .zip(self.rgb.chunks_exact(3))
                .map(|(a, px)| ((u32::from(*a) * u32::from(px.get(c).copied().unwrap_or(0)) + 127) / 255) as u16)
                .collect(),
        };
        Some([plane(0), plane(1), plane(2)])
    }

    /// The tip's mean colour (0..1), weighted by its alpha; `None` when the colours do not fit or
    /// the tip is empty.
    pub fn mean(&self, tip: &GrayTile) -> Option<[f32; 3]> {
        if !self.fits(tip) {
            return None;
        }
        let (mut sum, mut weight) = ([0.0f64; 3], 0.0f64);
        for (a, px) in tip.data.iter().zip(self.rgb.chunks_exact(3)) {
            let a = f64::from(*a);
            for (s, c) in sum.iter_mut().zip(px) {
                *s += a * f64::from(*c);
            }
            weight += a;
        }
        (weight > 0.0).then(|| sum.map(|s| (s / weight / 255.0) as f32))
    }

    /// Whether every visible texel has the same colour (within `tolerance` of 255).
    pub fn is_uniform(&self, tip: &GrayTile, tolerance: u8) -> bool {
        let mut visible = tip.data.iter().zip(self.rgb.chunks_exact(3)).filter(|(a, _)| **a > 0).map(|(_, px)| px);
        let Some(first) = visible.next() else { return true };
        visible.all(|px| px.iter().zip(first).all(|(a, b)| a.abs_diff(*b) <= tolerance))
    }
}
