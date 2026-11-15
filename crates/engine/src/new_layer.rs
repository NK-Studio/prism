//! The options of Layer › New › Layer… (Photoshop's New Layer dialog): colour label, "Use
//! Previous Layer to Create Clipping Mask", blend mode, opacity and "Fill with neutral color".
//! `layer.new.layer` applies them to the layer it creates, in the same history step.

use prism_color::BlendMode;
use prism_doc::{Document, LabelColor, Layer};
use serde_json::Value;

use crate::{EngineError, Result};

const CMD: &str = "layer.new.layer";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// The colour that leaves the layers below unchanged when blended in a mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Neutral {
    Black,
    Gray,
    White,
}

impl Neutral {
    /// Opaque RGBA of the colour. 50% gray is 128/255, as Edit › Fill's 50% Gray.
    pub fn rgba(self) -> [f32; 4] {
        let v = match self {
            Neutral::Black => 0.0,
            Neutral::Gray => 128.0 / 255.0,
            Neutral::White => 1.0,
        };
        [v, v, v, 1.0]
    }

    /// Photoshop's name for it in the New Layer dialog ("Fill with Overlay-neutral color (50% gray)").
    pub fn label(self) -> &'static str {
        match self {
            Neutral::Black => "black",
            Neutral::Gray => "50% gray",
            Neutral::White => "white",
        }
    }
}

/// The neutral colour of a blend mode, as Photoshop's New Layer dialog offers it. Normal,
/// Dissolve, Hard Mix and the component modes (Hue, Saturation, Color, Luminosity) have none.
pub fn neutral(mode: BlendMode) -> Option<Neutral> {
    use BlendMode::*;
    match mode {
        Darken | Multiply | ColorBurn | LinearBurn | DarkerColor | Divide => Some(Neutral::White),
        Lighten | Screen | ColorDodge | LinearDodge | LighterColor | Difference | Exclusion | Subtract => Some(Neutral::Black),
        Overlay | SoftLight | HardLight | VividLight | LinearLight | PinLight => Some(Neutral::Gray),
        _ => None,
    }
}

/// Validated New Layer options; the default changes nothing about a plain new layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    pub label: LabelColor,
    pub clipped: bool,
    pub blend: BlendMode,
    pub opacity: f32,
    pub fill: Option<Neutral>,
}

impl Default for Options {
    fn default() -> Self {
        Options { label: LabelColor::None, clipped: false, blend: BlendMode::Normal, opacity: 1.0, fill: None }
    }
}

fn flag(p: &Value, key: &str) -> Result<bool> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(v) => v.as_bool().ok_or_else(|| bad(format!("`{key}` must be a boolean"))),
    }
}

impl Options {
    /// Read `color`, `clipped`, `blend`, `opacity` and `fillNeutral` from the command's params.
    pub fn from_params(p: &Value) -> Result<Options> {
        let mut o = Options::default();
        match p.get("color") {
            None | Some(Value::Null) => {}
            Some(v) => {
                o.label = v.as_str().and_then(LabelColor::from_id).ok_or_else(|| {
                    bad("`color` must be a layer colour id (none, red, orange, yellow, green, seafoam, blue, indigo, magenta, fuchsia, violet, gray)")
                })?;
            }
        }
        o.clipped = flag(p, "clipped")?;
        match p.get("blend") {
            None | Some(Value::Null) => {}
            Some(v) => {
                let name = v.as_str().ok_or_else(|| bad("`blend` must be a blend mode name"))?;
                o.blend = crate::commands::blend_from_str(name)
                    .filter(|m| *m != BlendMode::PassThrough)
                    .ok_or_else(|| bad(format!("unknown blend mode `{name}`")))?;
            }
        }
        match p.get("opacity") {
            None | Some(Value::Null) => {}
            Some(v) => {
                let x = v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| bad("`opacity` must be a number in 0..1"))?;
                o.opacity = (x as f32).clamp(0.0, 1.0);
            }
        }
        if flag(p, "fillNeutral")? {
            o.fill = Some(neutral(o.blend).ok_or_else(|| bad(format!("{} mode has no neutral colour to fill with", o.blend.label())))?);
        }
        Ok(o)
    }

    /// Apply the options to a new raster layer of `doc` before it is inserted.
    pub fn apply(&self, doc: &Document, layer: &mut Layer) {
        layer.label = self.label;
        layer.clipped = self.clipped;
        layer.blend = self.blend;
        layer.opacity = self.opacity;
        if let Some(fill) = self.fill
            && let Some(surface) = layer.surface_mut()
        {
            let px = prism_raster::from_rgba(&surface.format(), fill.rgba());
            surface.fill_rect(doc.bounds(), &px);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::Session;

    fn session(mode: &str, depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 24, "height": 16, "mode": mode, "depth": depth, "background": "white"})).unwrap();
        s
    }

    fn active(s: &Session) -> &Layer {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap()
    }

    #[test]
    fn no_params_makes_a_plain_transparent_layer() {
        let mut s = session("rgb", 8);
        s.execute(CMD, json!({})).unwrap();
        let l = active(&s);
        assert_eq!((l.name.as_str(), l.blend, l.opacity, l.clipped, l.label), ("Layer 1", BlendMode::Normal, 1.0, false, LabelColor::None));
        assert!(l.surface().unwrap().content_bounds().is_empty());
    }

    #[test]
    fn dialog_options_land_on_the_new_layer_in_one_history_step() {
        let mut s = session("rgb", 8);
        s.execute(CMD, json!({"name": "Shade", "color": "red", "clipped": true, "blend": "multiply", "opacity": 0.4})).unwrap();
        let l = active(&s);
        assert_eq!((l.name.as_str(), l.blend, l.clipped, l.label), ("Shade", BlendMode::Multiply, true, LabelColor::Red));
        assert!((l.opacity - 0.4).abs() < 1e-6);
        assert!(l.surface().unwrap().content_bounds().is_empty(), "not filled unless asked");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers.len(), 1, "one undo removes the layer and its options");
    }

    #[test]
    fn every_blend_mode_has_photoshops_neutral_colour() {
        let none = [BlendMode::Normal, BlendMode::Dissolve, BlendMode::HardMix, BlendMode::Hue, BlendMode::Saturation, BlendMode::Color, BlendMode::Luminosity];
        for m in BlendMode::LAYER_MODES {
            assert_eq!(neutral(m).is_none(), none.contains(&m), "{m:?}");
        }
        assert_eq!(neutral(BlendMode::Multiply), Some(Neutral::White));
        assert_eq!(neutral(BlendMode::Screen), Some(Neutral::Black));
        assert_eq!(neutral(BlendMode::Overlay), Some(Neutral::Gray));
        assert_eq!(neutral(BlendMode::PassThrough), None);
    }

    /// The neutral fill covers the canvas and leaves the composite unchanged, in every colour
    /// mode and depth.
    #[test]
    fn neutral_fill_covers_the_canvas_in_every_mode_and_depth() {
        for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("gray", 8), ("gray", 16), ("cmyk", 8), ("lab", 16)] {
            for (blend, want) in [("multiply", 1.0), ("screen", 0.0), ("overlay", 128.0 / 255.0)] {
                let mut s = session(mode, depth);
                s.execute(CMD, json!({"blend": blend, "fillNeutral": true})).unwrap();
                let l = active(&s);
                let surface = l.surface().unwrap();
                assert_eq!(surface.content_bounds(), s.active().unwrap().doc.bounds(), "{mode}/{depth} {blend}");
                for (x, y) in [(0, 0), (23, 15), (11, 7)] {
                    let got = surface.rgba(x, y);
                    assert!(got[3] > 0.999, "{mode}/{depth} {blend}: opaque, got {got:?}");
                    // CMYK and Lab round-trip through their own primaries: allow a few levels.
                    assert!(got.iter().take(3).all(|c| (c - want).abs() < 6.0 / 255.0), "{mode}/{depth} {blend}: {got:?} vs {want}");
                }
            }
        }
    }

    #[test]
    fn bad_params_are_errors_not_panics() {
        let mut s = session("rgb", 8);
        for p in [
            json!({"color": "chartreuse"}),
            json!({"color": 3}),
            json!({"clipped": "yes"}),
            json!({"blend": "nope"}),
            json!({"blend": 7}),
            json!({"blend": "passThrough"}),
            json!({"opacity": "half"}),
            json!({"opacity": []}),
            json!({"fillNeutral": 1}),
            // Normal has no neutral colour (Photoshop greys the checkbox).
            json!({"fillNeutral": true}),
            json!({"blend": "hue", "fillNeutral": true}),
        ] {
            assert!(s.execute(CMD, p.clone()).is_err(), "{p}");
        }
        assert_eq!(s.active().unwrap().doc.layers.len(), 1, "a refused New Layer adds nothing");
        // Out-of-range opacity clamps; explicit nulls mean "default".
        s.execute(CMD, json!({"opacity": 1e300, "color": null, "blend": null, "clipped": null, "fillNeutral": null})).unwrap();
        assert_eq!(active(&s).opacity, 1.0);
        s.execute(CMD, json!({"opacity": -4})).unwrap();
        assert_eq!(active(&s).opacity, 0.0);
    }
}
