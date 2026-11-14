//! Bounded reuse of immutable tip preparation between strokes. Exact pixel equality avoids
//! hash collisions and invalidates edits even when a caller reuses the same allocation.
use super::{BrushContext, BrushSettings, Mips, Pattern, TipShape};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock, PoisonError};

const MAX_BYTES: usize = 256 << 20;
const MAX_ENTRIES: usize = 2;

#[derive(Default)]
struct Cache {
    entries: VecDeque<(usize, BrushContext)>,
    bytes: usize,
}

fn matches(a: &BrushSettings, b: &BrushSettings) -> bool {
    a.erase == b.erase
        && a.csp_mixing.is_some() == b.csp_mixing.is_some()
        && a.csp_ribbon == b.csp_ribbon
        && a.dual_brush.enabled == b.dual_brush.enabled
        && a.texture == b.texture
        && a.tip == b.tip
        && a.dual_brush.tip == b.dual_brush.tip
        && a.csp_tip_colors == b.csp_tip_colors
}

fn bytes(c: &BrushContext) -> usize {
    let gray = |tip: &TipShape| -> usize {
        match tip {
            TipShape::Round => 0,
            TipShape::Sampled(g) => g.data.len().saturating_mul(2),
            TipShape::Sequence(v) => v.iter().fold(0usize, |n, g| n.saturating_add(g.data.len().saturating_mul(2))),
        }
    };
    let mip = |m: &Mips| m.levels.iter().fold(0usize, |n, (_, _, v)| n.saturating_add(v.len().saturating_mul(4)));
    let mut n = gray(&c.brush.tip).saturating_add(gray(&c.brush.dual_brush.tip));
    if let Pattern::Tile(g) = &c.brush.texture.pattern {
        n = n.saturating_add(g.data.len().saturating_mul(2));
    }
    for color in c.brush.csp_tip_colors.iter().flatten() {
        n = n.saturating_add(color.rgb.len());
    }
    for m in c.tip.iter().chain(c.dual_tip.iter()).chain(c.sequence.iter()).chain(c.dual_sequence.iter()) {
        n = n.saturating_add(mip(m));
    }
    for (m, _) in &c.ribbon_tips {
        n = n.saturating_add(mip(m));
    }
    for planes in c.tip_colors.iter().chain(c.ribbon_colors.iter()).flatten() {
        for m in planes.iter() {
            n = n.saturating_add(mip(m));
        }
    }
    if let Some(t) = &c.texture {
        n = n.saturating_add(t.data.len().saturating_mul(4));
    }
    n
}

impl Cache {
    fn get(&mut self, brush: &BrushSettings) -> Option<BrushContext> {
        let i = self.entries.iter().position(|(_, c)| matches(&c.brush, brush))?;
        let entry = self.entries.remove(i)?;
        let context = entry.1.clone();
        self.entries.push_back(entry);
        Some(context)
    }

    fn insert(&mut self, context: &BrushContext, limit: usize) {
        let size = bytes(context);
        if size > limit {
            return;
        }
        while self.entries.len() >= MAX_ENTRIES || self.bytes.saturating_add(size) > limit {
            let Some((old, _)) = self.entries.pop_front() else { break };
            self.bytes = self.bytes.saturating_sub(old);
        }
        self.entries.push_back((size, context.clone()));
        self.bytes = self.bytes.saturating_add(size);
    }
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}

pub(super) fn get(brush: &BrushSettings) -> Option<BrushContext> {
    cache().lock().unwrap_or_else(PoisonError::into_inner).get(brush)
}

pub(super) fn insert(context: &BrushContext) {
    cache().lock().unwrap_or_else(PoisonError::into_inner).insert(context, MAX_BYTES);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CspTipColor, GrayTile};
    use std::sync::Arc;

    #[test]
    fn exact_pixels_invalidate_edits_and_limits_bound_retention() {
        let mut b = BrushSettings {
            tip: TipShape::Sampled(GrayTile::from_fn(8, 8, |_, _| 1.0)),
            csp_tip_colors: vec![Some(CspTipColor { width: 8, height: 8, rgb: vec![255; 192].into() })],
            ..Default::default()
        };
        let c = BrushContext::prepare(b.clone());
        let mut cache = Cache::default();
        cache.insert(&c, bytes(&c).saturating_sub(1));
        assert!(cache.get(&b).is_none());
        cache.insert(&c, MAX_BYTES);
        b.size = 21.0;
        b.color = [0.0, 1.0, 0.0, 1.0];
        let hit = cache.get(&b).unwrap();
        assert!(Arc::ptr_eq(c.tip.as_ref().unwrap(), hit.tip.as_ref().unwrap()));
        if let Some(Some(color)) = b.csp_tip_colors.first_mut() {
            color.rgb[0] = 0;
        }
        assert!(cache.get(&b).is_none());
        b = c.brush.as_ref().clone();
        if let TipShape::Sampled(g) = &mut b.tip {
            g.data[0] = 0;
        }
        assert!(cache.get(&b).is_none());
        for _ in 0..4 {
            cache.insert(&c, MAX_BYTES);
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert!(cache.bytes <= MAX_BYTES);
    }
}
