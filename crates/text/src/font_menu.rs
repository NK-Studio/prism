//! Font-menu metadata and bounded, exact-face previews (no fallback fonts).
use crate::{
    fonts::FontDb,
    raster::{Bounds, Coverage, Pen, Xform},
};
use parley::fontique::{Blob, FontStyle};
use skrifa::{
    MetadataProvider,
    instance::{LocationRef, Size},
    outline::DrawSettings,
};

pub const KOREAN_PROBE: &str = "가나다라마바사아자차카타파하한글서체견본";
#[derive(Clone, Debug, PartialEq)]
pub struct FamilyDetails {
    pub family: String,
    pub display_name: String,
    pub supports_korean: bool,
}

#[derive(Clone)]
pub struct PreviewSource {
    data: Blob<u8>,
    index: u32,
}

/// Coverage values in [0,1], independent of document colour depth and UI theme.
pub struct FontPreview {
    pub width: usize,
    pub height: usize,
    pub coverage: Vec<f32>,
}

fn covers(font: &skrifa::FontRef<'_>, text: &str) -> bool {
    let map = font.charmap();
    text.chars().all(|c| map.map(c).is_some_and(|g| g.to_u32() != 0))
}

impl FontDb {
    /// Normal face nearest weight 400. Its bytes also identify the exact preview face.
    pub fn preview_source(&mut self, family: &str) -> Option<PreviewSource> {
        let info = self.fcx.collection.family_by_name(family)?;
        let face = info.fonts().iter().min_by_key(|f| (f.style() != FontStyle::Normal, (f.weight().value() - 400.0).abs() as u32))?;
        Some(PreviewSource { data: face.load(Some(&mut self.fcx.source_cache))?, index: face.index() })
    }

    pub fn family_details(&mut self) -> Vec<FamilyDetails> {
        let names = self.korean_family_names();
        self.families()
            .into_iter()
            .map(|family| {
                let supports_korean = self.preview_source(&family).is_some_and(|s| s.supports_korean());
                FamilyDetails { display_name: names.get(&family).cloned().unwrap_or_else(|| family.clone()), family, supports_korean }
            })
            .collect()
    }
}

impl PreviewSource {
    pub fn supports_korean(&self) -> bool {
        skrifa::FontRef::from_index(self.data.as_ref(), self.index).is_ok_and(|f| covers(&f, KOREAN_PROBE))
    }

    /// Rasterizes only this face, at a bounded UI scale. Missing glyphs/unsupported outlines
    /// return None instead of showing a different font's sample.
    pub fn render(&self, pixels_per_point: f32) -> Option<FontPreview> {
        if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
            return None;
        }
        let scale = pixels_per_point.clamp(0.5, 4.0);
        let width = (220.0 * scale).ceil() as usize;
        let height = (32.0 * scale).ceil() as usize;
        let font = skrifa::FontRef::from_index(self.data.as_ref(), self.index).ok()?;
        let sample = if covers(&font, KOREAN_PROBE) { "서체견본" } else { "Sample" };
        if !covers(&font, sample) {
            return None;
        }
        let location = font.axes().location([("wght", 400.0)]);
        let coords = LocationRef::new(location.coords());
        let size = Size::new(22.0);
        let metrics = font.glyph_metrics(size, coords);
        let outlines = font.outline_glyphs();
        let mut glyphs = Vec::new();
        let mut advance = 0.0f64;
        let mut bounds = Bounds::default();
        for ch in sample.chars() {
            let id = font.charmap().map(ch)?;
            let outline = outlines.get(id)?;
            let xf = Xform([1.0, 0.0, 0.0, -1.0, advance, 0.0]);
            outline.draw(DrawSettings::unhinted(size, coords), &mut Pen::new(&mut bounds, xf)).ok()?;
            glyphs.push((id, xf));
            advance += f64::from(metrics.advance_width(id)?);
        }
        let [x0, y0, x1, y1] = bounds.rect?;
        let (w, h) = (x1 - x0, y1 - y0);
        if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
            return None;
        }
        let factor = f64::from(scale).min((width as f64 - 4.0) / w).min((height as f64 - 4.0) / h);
        let post = Xform([factor, 0.0, 0.0, factor, 2.0 - x0 * factor, (height as f64 - h * factor) / 2.0 - y0 * factor]);
        let mut coverage = Coverage::new(width, height);
        for (id, xf) in glyphs {
            outlines.get(id)?.draw(DrawSettings::unhinted(size, coords), &mut Pen::new(&mut coverage, post.mul(&xf))).ok()?;
        }
        Some(FontPreview { width, height, coverage: coverage.finish() })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires this workstation's installed Korean fonts"]
    fn installed_korean_catalog() {
        let start = std::time::Instant::now();
        let mut db = crate::fonts::FontDb::with_system_fonts();
        let details = db.family_details();
        for needle in ["NanumGothic", "NanumMyeongjo", "Nanum Brush", "Nanum Pen", "GungSeo", "BM", "Moneygraphy", "Maplestory", "Pretendard"] {
            let matches: Vec<_> =
                details.iter().filter(|f| f.family.to_lowercase().replace(' ', "").contains(&needle.to_lowercase().replace(' ', ""))).collect();
            println!("{needle}: {matches:?}");
            assert!(!matches.is_empty(), "missing {needle}");
            assert!(matches.iter().all(|f| f.supports_korean));
            for f in matches {
                assert!(db.preview_source(&f.family).unwrap().render(2.0).is_some());
            }
        }
        println!("{} families / {} Korean in {:?}", details.len(), details.iter().filter(|f| f.supports_korean).count(), start.elapsed());
    }

    #[test]
    fn exact_face_coverage_and_bounded_previews() {
        let mut db = crate::fonts::FontDb::new();
        let inter = db.preview_source("Inter").unwrap();
        assert!(!inter.supports_korean());
        assert!(db.preview_source("does-not-exist").is_none());
        let a = inter.render(2.0).unwrap();
        let b = db.preview_source("JetBrains Mono").unwrap().render(2.0).unwrap();
        assert_eq!((a.width, a.height), (440, 64));
        assert!(a.coverage.iter().any(|v| *v > 0.5));
        assert_ne!(a.coverage, b.coverage);
        // A face without a character map must not borrow even Latin sample glyphs.
        let mut bytes = crate::fonts::INTER_REGULAR.to_vec();
        let tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        for table in 0..tables {
            let offset = 12 + table * 16;
            if &bytes[offset..offset + 4] == b"cmap" {
                bytes[offset..offset + 4].copy_from_slice(b"none");
            }
        }
        let missing = super::PreviewSource { data: parley::fontique::Blob::new(std::sync::Arc::new(bytes)), index: 0 };
        assert!(missing.render(1.0).is_none());
        assert!(inter.render(f32::NAN).is_none());
        assert!(inter.render(-1.0).is_none());
        assert_eq!(inter.render(f32::MAX).unwrap().width, 880);
    }
}
