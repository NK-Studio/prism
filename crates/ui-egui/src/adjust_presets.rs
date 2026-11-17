//! The built-in presets of Photoshop 27.4's Image › Adjustments dialogs (the "Preset:" dropdown):
//! Levels, Curves, Exposure, Hue/Saturation, Black & White and Channel Mixer.
//!
//! The settings were observed, not copied: Photoshop was asked to make an adjustment layer from
//! each of its presets and the layer's settings were read back through its scripting interface
//! (2026-11-17, `docs/adjust-analysis.md` › Presets). Names are Photoshop's English ones (the
//! file names), in its order (alphabetical); the Korean names are in the `preset` context of the
//! catalogue.

use serde_json::{Value, json};

/// A built-in preset: its English name and the command params it sets (the rest stay default).
pub struct Preset {
    pub name: &'static str,
    pub params: fn() -> Value,
}

const LEVELS: &[Preset] = &[
    Preset { name: "Darker", params: || json!({"inBlack": 15, "inWhite": 255, "gamma": 1, "outBlack": 0, "outWhite": 255}) },
    Preset { name: "Increase Contrast 1", params: || json!({"inBlack": 10, "inWhite": 245, "gamma": 1, "outBlack": 0, "outWhite": 255}) },
    Preset { name: "Increase Contrast 2", params: || json!({"inBlack": 20, "inWhite": 235, "gamma": 1, "outBlack": 0, "outWhite": 255}) },
    Preset { name: "Increase Contrast 3", params: || json!({"inBlack": 30, "inWhite": 225, "gamma": 1, "outBlack": 0, "outWhite": 255}) },
    Preset { name: "Lighten Shadows", params: || json!({"inBlack": 0, "inWhite": 255, "gamma": 1.6, "outBlack": 0, "outWhite": 255}) },
    Preset { name: "Lighter", params: || json!({"inBlack": 0, "inWhite": 230, "gamma": 1, "outBlack": 0, "outWhite": 255}) },
    Preset { name: "Midtones Brighter", params: || json!({"inBlack": 0, "inWhite": 255, "gamma": 1.25, "outBlack": 0, "outWhite": 255}) },
    Preset { name: "Midtones Darker", params: || json!({"inBlack": 0, "inWhite": 255, "gamma": 0.75, "outBlack": 0, "outWhite": 255}) },
];

const CURVES_RGB: &[Preset] = &[
    Preset {
        name: "Color Negative (RGB)",
        params: || {
            json!({
                "red": [[33, 255], [119, 127], [185, 0]],
                "green": [[28, 255], [77, 127], [132, 0]],
                "blue": [[25, 255], [60, 127], [108, 0]],
            })
        },
    },
    Preset {
        name: "Cross Process (RGB)",
        params: || {
            json!({
                "red": [[0, 0], [64, 40], [128, 125], [175, 190], [255, 255]],
                "green": [[0, 0], [64, 48], [97, 128], [190, 208], [255, 208]],
                "blue": [[0, 0], [59, 24], [181, 223], [255, 255]],
            })
        },
    },
    Preset { name: "Darker (RGB)", params: || json!({"points": [[0, 0], [130, 101], [255, 255]]}) },
    Preset { name: "Increase Contrast (RGB)", params: || json!({"points": [[0, 0], [38, 17], [212, 231], [231, 250], [255, 255]]}) },
    Preset { name: "Lighter (RGB)", params: || json!({"points": [[0, 0], [103, 125], [255, 255]]}) },
    Preset { name: "Linear Contrast (RGB)", params: || json!({"points": [[0, 0], [78, 73], [177, 182], [255, 255]]}) },
    Preset { name: "Medium Contrast (RGB)", params: || json!({"points": [[0, 0], [73, 56], [163, 164], [255, 255]]}) },
    Preset { name: "Negative (RGB)", params: || json!({"points": [[0, 255], [255, 0]]}) },
    Preset { name: "Strong Contrast (RGB)", params: || json!({"points": [[0, 0], [77, 50], [151, 153], [175, 188], [255, 255]]}) },
];

const EXPOSURE: &[Preset] = &[
    Preset { name: "Minus 1.0", params: || json!({"exposure": -1, "offset": 0, "gamma": 1}) },
    Preset { name: "Minus 2.0", params: || json!({"exposure": -2, "offset": 0, "gamma": 1}) },
    Preset { name: "Plus 1.0", params: || json!({"exposure": 1, "offset": 0, "gamma": 1}) },
    Preset { name: "Plus 2.0", params: || json!({"exposure": 2, "offset": 0, "gamma": 1}) },
];

const HUE_SATURATION: &[Preset] = &[
    Preset { name: "Cyanotype", params: || json!({"colorize": true, "hue": 215, "saturation": 25, "lightness": 0}) },
    Preset { name: "Increase Saturation More", params: || json!({"hue": 0, "saturation": 30, "lightness": 0}) },
    Preset { name: "Increase Saturation", params: || json!({"hue": 0, "saturation": 10, "lightness": 0}) },
    Preset { name: "Old Style", params: || json!({"hue": 0, "saturation": -40, "lightness": 5}) },
    Preset { name: "Red Boost", params: || json!({"hue": -5, "saturation": 20, "lightness": 0}) },
    Preset { name: "Sepia", params: || json!({"colorize": true, "hue": 35, "saturation": 25, "lightness": 0}) },
    Preset { name: "Strong Saturation", params: || json!({"hue": 0, "saturation": 50, "lightness": 0}) },
    Preset { name: "Yellow Boost", params: || json!({"hue": 5, "saturation": 20, "lightness": 0}) },
];

/// Black & White's presets set the six weights (reds … magentas) and leave Tint off.
const BLACK_WHITE: &[Preset] = &[
    Preset { name: "Blue Filter", params: || bw([0, 0, 0, 110, 110, 110]) },
    Preset { name: "Darker", params: || bw([30, 50, 30, 50, 10, 70]) },
    Preset { name: "Green Filter", params: || bw([50, 120, 90, 50, 0, 0]) },
    Preset { name: "High Contrast Blue Filter", params: || bw([-50, -50, -50, 150, 150, 150]) },
    Preset { name: "High Contrast Red Filter", params: || bw([120, 120, -10, -50, -50, 120]) },
    Preset { name: "Infrared", params: || bw([-40, 235, 144, -68, -3, -107]) },
    Preset { name: "Lighter", params: || bw([50, 70, 50, 70, 30, 90]) },
    Preset { name: "Maximum Black", params: || bw([0, 0, 0, 0, 0, 0]) },
    Preset { name: "Maximum White", params: || bw([100, 100, 100, 100, 100, 100]) },
    Preset { name: "Neutral Density", params: || bw([128, 128, 100, 100, 128, 100]) },
    Preset { name: "Red Filter", params: || bw([120, 110, -10, -50, 0, 120]) },
    Preset { name: "Yellow Filter", params: || bw([120, 110, 40, -30, 0, 70]) },
];

fn bw(w: [i32; 6]) -> Value {
    json!({"reds": w[0], "yellows": w[1], "greens": w[2], "cyans": w[3], "blues": w[4], "magentas": w[5], "tint": false})
}

/// Channel Mixer's presets (RGB documents): monochrome mixes.
const CHANNEL_MIXER_RGB: &[Preset] = &[
    Preset { name: "Black & White Infrared (RGB)", params: || mono([-70, 200, -30, 0]) },
    Preset { name: "Black & White with Blue Filter (RGB)", params: || mono([0, 0, 100, 0]) },
    Preset { name: "Black & White with Green Filter (RGB)", params: || mono([0, 100, 0, 0]) },
    Preset { name: "Black & White with Orange Filter (RGB)", params: || mono([50, 50, 0, 0]) },
    Preset { name: "Black & White with Red Filter (RGB)", params: || mono([100, 0, 0, 0]) },
    Preset { name: "Black & White with Yellow Filter (RGB)", params: || mono([34, 66, 0, 0]) },
];

fn mono(gray: [i32; 4]) -> Value {
    json!({"monochrome": true, "gray": gray})
}

/// The built-in presets of adjustment `kind` for a document in `mode` (Photoshop lists its
/// "(RGB)" Curves presets for RGB documents only).
pub fn presets(kind: &str, mode: prism_doc::ColorMode) -> &'static [Preset] {
    match kind {
        "levels" => LEVELS,
        "curves" if mode == prism_doc::ColorMode::Rgb => CURVES_RGB,
        "exposure" => EXPOSURE,
        "hueSaturation" => HUE_SATURATION,
        "blackWhite" => BLACK_WHITE,
        "channelMixer" if mode == prism_doc::ColorMode::Rgb => CHANNEL_MIXER_RGB,
        _ => &[],
    }
}

/// A preset's complete settings: the kind's defaults with the preset's params over them.
pub fn resolve(kind: &str, mode: prism_doc::ColorMode, preset: &Preset) -> Option<Value> {
    let adj = prism_engine::adjust_params::from_params(kind, &(preset.params)(), None, mode).ok()?;
    Some(prism_engine::adjust_params::to_params(&adj))
}

/// The preset's name as shown (Photoshop's own wording in the UI language).
pub fn label(name: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "preset", name)
}

/// Which entry of the dropdown the settings `values` are: "default", "p{index}" of a built-in
/// preset, or "custom". Compared as settings, not as JSON (35 and 35.0 are the same hue).
pub fn matching(kind: &str, mode: prism_doc::ColorMode, values: &Value, defaults: &Value) -> String {
    let normal = |v: &Value| prism_engine::adjust_params::from_params(kind, v, None, mode).ok().map(|a| prism_engine::adjust_params::to_params(&a));
    let now = normal(values);
    if now.is_some() && now == normal(defaults) {
        return "default".to_string();
    }
    presets(kind, mode)
        .iter()
        .position(|p| now.is_some() && resolve(kind, mode, p) == now)
        .map_or_else(|| "custom".to_string(), |i| format!("p{i}"))
}

/// Photoshop's "Preset:" dropdown: Default, the built-in presets, and Custom once the settings
/// match none of them. Picking one replaces `values`; returns whether it did.
pub fn dropdown(ui: &mut egui::Ui, id: &str, kind: &str, mode: prism_doc::ColorMode, values: &mut Value, defaults: &Value, width: f32) -> bool {
    let list = presets(kind, mode);
    let resolved: Vec<Option<Value>> = list.iter().map(|p| resolve(kind, mode, p)).collect();
    let current = matching(kind, mode, values, defaults);
    let mut pick = current.clone();
    let mut options: Vec<(String, &str)> = vec![("default".to_string(), tl!("Default"))];
    options.extend(list.iter().enumerate().map(|(i, p)| (format!("p{i}"), label(p.name))));
    if current == "custom" {
        options.push(("custom".to_string(), tl!("Custom")));
    }
    if !crate::widgets::dropdown(ui, id, &mut pick, &options, width) || pick == current {
        return false;
    }
    if pick == "default" {
        *values = defaults.clone();
        return true;
    }
    match pick.strip_prefix('p').and_then(|i| i.parse::<usize>().ok()).and_then(|i| resolved.get(i).cloned().flatten()) {
        Some(v) => {
            *values = v;
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every preset resolves to valid settings that differ from the defaults.
    #[test]
    fn every_builtin_preset_resolves() {
        for kind in ["levels", "curves", "exposure", "hueSaturation", "blackWhite", "channelMixer"] {
            let list = presets(kind, prism_doc::ColorMode::Rgb);
            assert!(!list.is_empty(), "{kind}");
            let default = prism_engine::adjust_params::default_for(kind, prism_doc::ColorMode::Rgb).map(|d| prism_engine::adjust_params::to_params(&d)).unwrap();
            for p in list {
                let v = resolve(kind, prism_doc::ColorMode::Rgb, p).unwrap_or_else(|| panic!("{kind} {}", p.name));
                assert_ne!(v, default, "{kind} {}", p.name);
            }
        }
        assert!(presets("curves", prism_doc::ColorMode::Cmyk).is_empty());
        assert!(presets("vibrance", prism_doc::ColorMode::Rgb).is_empty());
    }

    /// The dropdown shows the preset the settings match, written either way (35 or 35.0).
    #[test]
    fn matching_names_the_preset() {
        let rgb = prism_doc::ColorMode::Rgb;
        let d = |k: &str| prism_engine::adjust_params::default_for(k, rgb).map(|d| prism_engine::adjust_params::to_params(&d)).unwrap();
        let hs = d("hueSaturation");
        let mut sepia = hs.clone();
        sepia["colorize"] = json!(true);
        sepia["hue"] = json!(35);
        sepia["saturation"] = json!(25);
        assert_eq!(matching("hueSaturation", rgb, &sepia, &hs), "p5");
        assert_eq!(matching("hueSaturation", rgb, &hs, &hs), "default");
        let lv = d("levels");
        let mut v = lv.clone();
        v["gamma"] = json!(1.6);
        assert_eq!(matching("levels", rgb, &v, &lv), "p4");
        v["gamma"] = json!(1.7);
        assert_eq!(matching("levels", rgb, &v, &lv), "custom");
        assert_eq!(matching("vibrance", rgb, &json!({"vibrance": 3}), &d("vibrance")), "custom");
    }

    /// Photoshop 27.4's values (observed): Sepia colourises at hue 35, saturation 25; Levels'
    /// Increase Contrast 2 clips 20 / 235.
    #[test]
    fn presets_carry_photoshops_values() {
        let sepia = resolve("hueSaturation", prism_doc::ColorMode::Rgb, &HUE_SATURATION[5]).unwrap();
        assert_eq!((sepia["colorize"].clone(), sepia["hue"].as_f64(), sepia["saturation"].as_f64()), (json!(true), Some(35.0), Some(25.0)));
        let ic2 = resolve("levels", prism_doc::ColorMode::Rgb, &LEVELS[2]).unwrap();
        assert_eq!((ic2["inBlack"].as_f64(), ic2["inWhite"].as_f64()), (Some(20.0), Some(235.0)));
    }
}
