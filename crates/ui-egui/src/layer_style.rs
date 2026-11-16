//! Photoshop-style Layer Style dialog: one row per effect instance on the left,
//! parameters on the right. State lives in the dialog's `fields` (JSON), so
//! automation can drive it like any other dialog:
//!
//! - `layer`: layer id; `globalLight`: the document's light angle (degrees)
//! - `selected`: `"blendingOptions"` or an instance id from `effects`
//! - `effects`: `[{id, kind, on, params, fx?}]` in the layer's effect order.
//!   `fx` is the effect snapshot the dialog loaded: the apply edits that
//!   snapshot with `params` (engine-side overlay), so everything the dialog
//!   doesn't model — gradient strokes, imported contours, PSD data — survives.
//!   An entry without `fx` is a new instance built from `params`.
//! - `p:blendingOptions`: the parameters of `layer.layerStyle.blendingOptions` (blend mode,
//!   opacity, fill opacity, channels, knockout, the Advanced Blending switches, Blend If) plus
//!   the page-only `blendIfChannel`; `channelNames`: the document's colour channels.

use egui::{Color32, RichText, Sense, Stroke, StrokeKind, vec2};
use prism_doc::effects::{FxPaint, StrokePosition};
use prism_doc::{Effect, Layer};
use serde_json::{Map, Value, json};

use crate::theme::Tokens;
use crate::{PrismApp, widgets};

#[derive(Clone, Copy)]
enum P {
    /// Starts a titled group box, as Photoshop frames Structure, Quality, …; an empty title
    /// ends the box and the rows that follow sit under it (Layer Knocks Out Drop Shadow).
    Group,
    Slider(f32, f32, &'static str),
    Color,
    /// Blend mode; `Some(key)` puts that colour's swatch on the same row (Drop Shadow, Satin).
    Blend(Option<&'static str>),
    Choice(&'static [(&'static str, &'static str)]),
    /// Radio buttons in one row (Direction: Up / Down, Source: Center / Edge).
    Radio(&'static [(&'static str, &'static str)]),
    Check,
    /// Angle dial and field; `Some((key, label))` puts that check box beside it (Use Global Light).
    Angle(Option<(&'static str, &'static str)>),
    /// A pattern from the dialog's `patternList` field (`[[id, name], …]`).
    Pattern,
    /// A two-stop gradient: the `from` swatch, a ramp, then the `to` swatch.
    Gradient,
}

const GLOBAL_LIGHT: Option<(&str, &str)> = Some(("useGlobalLight", "Use Global Light"));

/// Effect kinds in Photoshop's list order: (command kind, label, params).
/// The Styles preset gallery page id.
pub const STYLES_PAGE: &str = "styles";
/// The Blending Options page id (not an effect).
pub const BLENDING: &str = "blendingOptions";

pub const KINDS: &[(&str, &str)] = &[
    ("bevelEmboss", "Bevel & Emboss"),
    ("stroke", "Stroke"),
    ("innerShadow", "Inner Shadow"),
    ("innerGlow", "Inner Glow"),
    ("satin", "Satin"),
    ("colorOverlay", "Color Overlay"),
    ("gradientOverlay", "Gradient Overlay"),
    ("patternOverlay", "Pattern Overlay"),
    ("outerGlow", "Outer Glow"),
    ("dropShadow", "Drop Shadow"),
];

/// Effect kinds Photoshop offers several instances of (stored as `…Multi` lists in the PSD).
const MULTI: &[&str] = &["stroke", "dropShadow", "innerShadow", "colorOverlay", "gradientOverlay"];

fn multi(kind: &str) -> bool {
    MULTI.contains(&kind)
}

fn kind_of(e: &Effect) -> &'static str {
    prism_engine::layer_style::kind_of(e)
}

fn defaults(kind: &str) -> Value {
    prism_engine::layer_style::effect_defaults(kind)
}

/// Factory params for a new instance, with the shared light angle filled in.
fn fresh_params(kind: &str, light: f32) -> Value {
    let mut p = defaults(kind);
    if p.get("useGlobalLight").and_then(Value::as_bool) == Some(true) {
        p["angle"] = json!(light);
    }
    p
}

fn spec(kind: &str) -> &'static [(&'static str, &'static str, P)] {
    const POS: &[(&str, &str)] = &[("outside", "Outside"), ("inside", "Inside"), ("center", "Center")];
    const GSTYLE: &[(&str, &str)] = &[("linear", "Linear"), ("radial", "Radial"), ("angle", "Angle"), ("reflected", "Reflected"), ("diamond", "Diamond")];
    const BSTYLE: &[(&str, &str)] = &[("inner", "Inner Bevel"), ("outer", "Outer Bevel"), ("emboss", "Emboss"), ("pillow", "Pillow Emboss")];
    const BTECH: &[(&str, &str)] = &[("smooth", "Smooth"), ("chiselHard", "Chisel Hard"), ("chiselSoft", "Chisel Soft")];
    const GTECH: &[(&str, &str)] = &[("softer", "Softer"), ("precise", "Precise")];
    const DIR: &[(&str, &str)] = &[("up", "Up"), ("down", "Down")];
    const SRC: &[(&str, &str)] = &[("center", "Center"), ("edge", "Edge")];
    // The built-in contour presets (prism_engine::layer_style::CONTOURS).
    const CONTOUR: &[(&str, &str)] = &[
        ("Linear", "Linear"),
        ("Cone", "Cone"),
        ("Cone (Inverted)", "Cone (Inverted)"),
        ("Domed", "Domed"),
        ("Domed (Inverted)", "Domed (Inverted)"),
        ("Diagonal (Descending)", "Diagonal (Descending)"),
    ];
    // Photoshop's pages: titled boxes (Structure, Quality, …), the rows in its order.
    match kind {
        "dropShadow" => &[
            ("", "Structure", P::Group),
            ("blend", "Blend Mode", P::Blend(Some("color"))),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("angle", "Angle", P::Angle(GLOBAL_LIGHT)),
            ("distance", "Distance", P::Slider(0.0, 300.0, "px")),
            ("spread", "Spread", P::Slider(0.0, 100.0, "%")),
            ("size", "Size", P::Slider(0.0, 250.0, "px")),
            ("", "Quality", P::Group),
            ("contour", "Contour", P::Choice(CONTOUR)),
            ("noise", "Noise", P::Slider(0.0, 100.0, "%")),
            ("", "", P::Group),
            ("knocksOut", "Layer Knocks Out Drop Shadow", P::Check),
        ],
        "innerShadow" => &[
            ("", "Structure", P::Group),
            ("blend", "Blend Mode", P::Blend(Some("color"))),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("angle", "Angle", P::Angle(GLOBAL_LIGHT)),
            ("distance", "Distance", P::Slider(0.0, 300.0, "px")),
            ("choke", "Choke", P::Slider(0.0, 100.0, "%")),
            ("size", "Size", P::Slider(0.0, 250.0, "px")),
            ("", "Quality", P::Group),
            ("contour", "Contour", P::Choice(CONTOUR)),
            ("noise", "Noise", P::Slider(0.0, 100.0, "%")),
        ],
        "outerGlow" => &[
            ("", "Structure", P::Group),
            ("blend", "Blend Mode", P::Blend(None)),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("noise", "Noise", P::Slider(0.0, 100.0, "%")),
            ("color", "Color", P::Color),
            ("", "Elements", P::Group),
            ("technique", "Technique", P::Choice(GTECH)),
            ("spread", "Spread", P::Slider(0.0, 100.0, "%")),
            ("size", "Size", P::Slider(0.0, 250.0, "px")),
            ("", "Quality", P::Group),
            ("contour", "Contour", P::Choice(CONTOUR)),
            ("range", "Range", P::Slider(1.0, 100.0, "%")),
        ],
        "innerGlow" => &[
            ("", "Structure", P::Group),
            ("blend", "Blend Mode", P::Blend(None)),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("noise", "Noise", P::Slider(0.0, 100.0, "%")),
            ("color", "Color", P::Color),
            ("", "Elements", P::Group),
            ("technique", "Technique", P::Choice(GTECH)),
            ("source", "Source", P::Radio(SRC)),
            ("choke", "Choke", P::Slider(0.0, 100.0, "%")),
            ("size", "Size", P::Slider(0.0, 250.0, "px")),
            ("", "Quality", P::Group),
            ("contour", "Contour", P::Choice(CONTOUR)),
            ("range", "Range", P::Slider(1.0, 100.0, "%")),
        ],
        "stroke" => &[
            ("", "Structure", P::Group),
            ("size", "Size", P::Slider(1.0, 250.0, "px")),
            ("position", "Position", P::Choice(POS)),
            ("blend", "Blend Mode", P::Blend(None)),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("", "Fill Type", P::Group),
            ("color", "Color", P::Color),
        ],
        "colorOverlay" => &[
            ("", "Color", P::Group),
            ("blend", "Blend Mode", P::Blend(Some("color"))),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
        ],
        "gradientOverlay" => &[
            ("", "Gradient", P::Group),
            ("blend", "Blend Mode", P::Blend(None)),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("from", "Gradient", P::Gradient),
            ("reverse", "Reverse", P::Check),
            ("style", "Style", P::Choice(GSTYLE)),
            ("angle", "Angle", P::Angle(None)),
            ("scale", "Scale", P::Slider(10.0, 150.0, "%")),
            ("dither", "Dither", P::Check),
        ],
        "patternOverlay" => &[
            ("", "Pattern", P::Group),
            ("blend", "Blend Mode", P::Blend(None)),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("pattern", "Pattern", P::Pattern),
            ("angle", "Angle", P::Angle(None)),
            ("scale", "Scale", P::Slider(1.0, 1000.0, "%")),
            ("link", "Link with Layer", P::Check),
        ],
        "bevelEmboss" => &[
            ("", "Structure", P::Group),
            ("style", "Style", P::Choice(BSTYLE)),
            ("technique", "Technique", P::Choice(BTECH)),
            ("depth", "Depth", P::Slider(1.0, 1000.0, "%")),
            ("direction", "Direction", P::Radio(DIR)),
            ("size", "Size", P::Slider(0.0, 250.0, "px")),
            ("soften", "Soften", P::Slider(0.0, 16.0, "px")),
            ("", "Shading", P::Group),
            ("angle", "Angle", P::Angle(GLOBAL_LIGHT)),
            ("altitude", "Altitude", P::Slider(0.0, 90.0, "°")),
            ("glossContour", "Gloss Contour", P::Choice(CONTOUR)),
        ],
        // Bevel & Emboss's Contour and Texture elements: their own pages, editing the bevel's
        // params (see BEVEL_PARTS).
        "bevelContour" => &[
            ("", "Elements", P::Group),
            ("contourName", "Contour", P::Choice(CONTOUR)),
            ("contourAntiAlias", "Anti-aliased", P::Check),
            ("contourRange", "Range", P::Slider(1.0, 100.0, "%")),
        ],
        "bevelTexture" => &[
            ("", "Elements", P::Group),
            ("texture", "Pattern", P::Pattern),
            ("textureScale", "Scale", P::Slider(1.0, 1000.0, "%")),
            ("textureDepth", "Depth", P::Slider(-1000.0, 1000.0, "%")),
            ("textureInvert", "Invert", P::Check),
            ("textureLink", "Link with Layer", P::Check),
        ],
        "satin" => &[
            ("", "Structure", P::Group),
            ("blend", "Blend Mode", P::Blend(Some("color"))),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("angle", "Angle", P::Angle(None)),
            ("distance", "Distance", P::Slider(0.0, 250.0, "px")),
            ("size", "Size", P::Slider(0.0, 250.0, "px")),
            ("invert", "Invert", P::Check),
        ],
        _ => &[],
    }
}

fn hex(c: &prism_doc::Color) -> String {
    let [r, g, b, _] = c.to_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Current values of an existing effect, in the command's parameter units —
/// only the keys this dialog models. Everything else (gradient strokes, multi-stop
/// gradients, imported contours) is left out so the apply overlay can't clobber it.
/// `light` is the document's global light angle: an effect that uses it shows that angle.
fn values_of(e: &Effect, light: f32) -> Value {
    let mut v = defaults(kind_of(e));
    let set = |v: &mut Value, k: &str, x: Value| {
        v[k] = x;
    };
    let drop_keys = |v: &mut Value, keys: &[&str]| {
        if let Some(o) = v.as_object_mut() {
            for k in keys {
                o.remove(*k);
            }
        }
    };
    match e {
        Effect::DropShadow(s) | Effect::InnerShadow(s) => {
            set(&mut v, "blend", json!(s.common.blend.label()));
            set(&mut v, "opacity", json!((s.common.opacity * 100.0).round()));
            set(&mut v, "color", json!(hex(&s.color)));
            set(&mut v, "angle", json!(if s.use_global_light { light } else { s.angle }));
            set(&mut v, "useGlobalLight", json!(s.use_global_light));
            set(&mut v, "distance", json!(s.distance));
            set(&mut v, if matches!(e, Effect::DropShadow(_)) { "spread" } else { "choke" }, json!((s.spread * 100.0).round()));
            set(&mut v, "size", json!(s.size));
            set(&mut v, "contour", prism_engine::layer_style::contour_param(&s.contour));
            set(&mut v, "noise", json!((s.noise * 100.0).round()));
            if matches!(e, Effect::DropShadow(_)) {
                set(&mut v, "knocksOut", json!(s.knocks_out));
            }
        }
        Effect::OuterGlow(g) | Effect::InnerGlow(g) => {
            set(&mut v, "blend", json!(g.common.blend.label()));
            set(&mut v, "opacity", json!((g.common.opacity * 100.0).round()));
            if let FxPaint::Color(c) = &g.paint {
                set(&mut v, "color", json!(hex(c)));
            } else {
                drop_keys(&mut v, &["color"]);
            }
            set(&mut v, "technique", json!(if g.technique == prism_doc::effects::GlowTechnique::Precise { "precise" } else { "softer" }));
            set(&mut v, if matches!(e, Effect::OuterGlow(_)) { "spread" } else { "choke" }, json!((g.spread * 100.0).round()));
            set(&mut v, "size", json!(g.size));
            set(&mut v, "range", json!((g.range * 100.0).round()));
            set(&mut v, "contour", prism_engine::layer_style::contour_param(&g.contour));
            set(&mut v, "noise", json!((g.noise * 100.0).round()));
        }
        Effect::Stroke(s) => {
            set(&mut v, "blend", json!(s.common.blend.label()));
            set(&mut v, "opacity", json!((s.common.opacity * 100.0).round()));
            set(&mut v, "size", json!(s.size));
            set(
                &mut v,
                "position",
                json!(match s.position {
                    StrokePosition::Inside => "inside",
                    StrokePosition::Center => "center",
                    _ => "outside",
                }),
            );
            if let FxPaint::Color(c) = &s.paint {
                set(&mut v, "color", json!(hex(c)));
            } else {
                // A gradient/pattern stroke: the dialog's colour field doesn't model
                // the paint, so the key is left out and the snapshot carries it.
                drop_keys(&mut v, &["color"]);
            }
        }
        Effect::ColorOverlay { common, color } => {
            set(&mut v, "blend", json!(common.blend.label()));
            set(&mut v, "opacity", json!((common.opacity * 100.0).round()));
            set(&mut v, "color", json!(hex(color)));
        }
        Effect::GradientOverlay { common, gradient: g, dither } => {
            set(&mut v, "blend", json!(common.blend.label()));
            set(&mut v, "opacity", json!((common.opacity * 100.0).round()));
            set(&mut v, "dither", json!(dither));
            // The From/To fields model a plain two-stop gradient; anything richer
            // (more stops, transparency stops) is carried by the snapshot instead.
            if g.stops.len() == 2 && g.opacity_stops.is_empty() {
                set(&mut v, "from", json!(hex(&g.stops[0].1)));
                set(&mut v, "to", json!(hex(&g.stops[1].1)));
                set(&mut v, "reverse", json!(g.reverse));
                set(
                    &mut v,
                    "style",
                    json!(match g.style {
                        prism_doc::effects::GradientStyle::Radial => "radial",
                        prism_doc::effects::GradientStyle::Angle => "angle",
                        prism_doc::effects::GradientStyle::Reflected => "reflected",
                        prism_doc::effects::GradientStyle::Diamond => "diamond",
                        prism_doc::effects::GradientStyle::Linear => "linear",
                    }),
                );
                set(&mut v, "angle", json!(g.angle));
                set(&mut v, "scale", json!((g.scale * 100.0).round()));
            } else {
                drop_keys(&mut v, &["from", "to", "reverse", "style", "angle", "scale"]);
            }
        }
        Effect::PatternOverlay { common, name, id, scale, angle, link, phase } => {
            set(&mut v, "blend", json!(common.blend.label()));
            set(&mut v, "opacity", json!((common.opacity * 100.0).round()));
            set(&mut v, "pattern", json!(if id.is_empty() { name } else { id }));
            set(&mut v, "scale", json!((scale * 100.0).round()));
            set(&mut v, "angle", json!(angle));
            set(&mut v, "link", json!(link));
            set(&mut v, "phaseX", json!(phase.0));
            set(&mut v, "phaseY", json!(phase.1));
        }
        Effect::Satin(s) => {
            set(&mut v, "blend", json!(s.common.blend.label()));
            set(&mut v, "opacity", json!((s.common.opacity * 100.0).round()));
            set(&mut v, "color", json!(hex(&s.color)));
            set(&mut v, "angle", json!(s.angle));
            set(&mut v, "distance", json!(s.distance));
            set(&mut v, "size", json!(s.size));
            set(&mut v, "invert", json!(s.invert));
            set(&mut v, "contour", prism_engine::layer_style::contour_param(&s.contour));
        }
        Effect::BevelEmboss(b) => {
            set(
                &mut v,
                "style",
                json!(match b.style {
                    prism_doc::effects::BevelStyle::OuterBevel => "outer",
                    prism_doc::effects::BevelStyle::Emboss => "emboss",
                    prism_doc::effects::BevelStyle::PillowEmboss => "pillow",
                    prism_doc::effects::BevelStyle::StrokeEmboss => "stroke",
                    prism_doc::effects::BevelStyle::InnerBevel => "inner",
                }),
            );
            set(
                &mut v,
                "technique",
                json!(match b.technique {
                    prism_doc::effects::BevelTechnique::ChiselHard => "chiselHard",
                    prism_doc::effects::BevelTechnique::ChiselSoft => "chiselSoft",
                    prism_doc::effects::BevelTechnique::Smooth => "smooth",
                }),
            );
            set(&mut v, "depth", json!((b.depth * 100.0).round()));
            set(&mut v, "size", json!(b.size));
            set(&mut v, "soften", json!(b.soften));
            set(&mut v, "angle", json!(if b.use_global_light { light } else { b.angle }));
            set(&mut v, "useGlobalLight", json!(b.use_global_light));
            set(&mut v, "altitude", json!(b.altitude));
            set(&mut v, "direction", json!(if b.up { "up" } else { "down" }));
            set(&mut v, "glossContour", prism_engine::layer_style::contour_param(&b.gloss_contour));
            // The Contour and Texture elements (their own list rows under Bevel & Emboss).
            set(&mut v, "contour", json!(b.contour.is_some()));
            if let Some(c) = &b.contour {
                set(&mut v, "contourName", prism_engine::layer_style::contour_param(&c.contour));
                set(&mut v, "contourRange", json!((c.range * 100.0).round()));
                set(&mut v, "contourAntiAlias", json!(c.anti_alias));
            }
            match &b.texture {
                Some(x) => {
                    set(&mut v, "texture", json!(if x.id.is_empty() { &x.name } else { &x.id }));
                    set(&mut v, "textureScale", json!((x.scale * 100.0).round()));
                    set(&mut v, "textureDepth", json!((x.depth * 100.0).round()));
                    set(&mut v, "textureInvert", json!(x.invert));
                    set(&mut v, "textureLink", json!(x.link));
                }
                None => set(&mut v, "texture", json!(false)),
            }
        }
    }
    v
}

/// The dialog's effect entries (JSON array of `{id, kind, on, params, fx?}`).
fn effects_of(f: &Map<String, Value>) -> &[Value] {
    f.get("effects").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn entry<'a>(f: &'a Map<String, Value>, id: &str) -> Option<&'a Value> {
    effects_of(f).iter().find(|e| e.get("id").and_then(Value::as_str) == Some(id))
}

fn entry_mut<'a>(f: &'a mut Map<String, Value>, id: &str) -> Option<&'a mut Value> {
    f.get_mut("effects").and_then(Value::as_array_mut).and_then(|a| a.iter_mut().find(|e| e.get("id").and_then(Value::as_str) == Some(id)))
}

/// Stores one edited parameter of an instance (or of the blending page).
fn set_param(f: &mut Map<String, Value>, id: &str, key: &str, value: Value) {
    if id == BLENDING || id == STYLES_PAGE {
        if let Some(o) = f.get_mut("p:blendingOptions").and_then(Value::as_object_mut) {
            o.insert(key.into(), value);
        }
        return;
    }
    // All globally lit instances share one angle, including disabled instances.
    // Keep the opening angle intact for automation's changed-value fallback in apply().
    let shared = entry(f, id).and_then(|e| e.get("params")).is_some_and(|p| {
        (key == "angle" && p.get("useGlobalLight").and_then(Value::as_bool) == Some(true)) || (key == "useGlobalLight" && value.as_bool() == Some(true))
    });
    let shared_angle =
        if shared { if key == "angle" { Some(value.clone()) } else { f.get("pendingGlobalLight").or_else(|| f.get("globalLight")).cloned() } } else { None };
    if let Some(e) = entry_mut(f, id)
        && let Some(o) = e.get_mut("params").and_then(Value::as_object_mut)
    {
        o.insert(key.into(), value);
    }
    if let Some(angle) = shared_angle {
        f.insert("pendingGlobalLight".into(), angle.clone());
        if let Some(effects) = f.get_mut("effects").and_then(Value::as_array_mut) {
            for e in effects {
                if let Some(p) = e.get_mut("params").and_then(Value::as_object_mut)
                    && p.get("useGlobalLight").and_then(Value::as_bool) == Some(true)
                {
                    p.insert("angle".into(), angle.clone());
                }
            }
        }
    }
}

/// A fresh instance id: one past the highest numbered `fxN` in the list.
fn next_id(f: &Map<String, Value>) -> String {
    let max = effects_of(f)
        .iter()
        .filter_map(|e| e.get("id").and_then(Value::as_str))
        .filter_map(|id| id.strip_prefix("fx"))
        .filter_map(|n| n.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("fx{}", max + 1)
}

/// Adds an instance of `kind`, right after the last one of its kind (the list,
/// and so the rendered stack, keeps the layer's effect order), and selects it.
fn add_instance(f: &mut Map<String, Value>, kind: &str) {
    let light = f.get("pendingGlobalLight").or_else(|| f.get("globalLight")).and_then(Value::as_f64).unwrap_or(120.0) as f32;
    let id = next_id(f);
    let new = json!({"id": id, "kind": kind, "on": true, "params": fresh_params(kind, light)});
    if let Some(a) = f.get_mut("effects").and_then(Value::as_array_mut) {
        let pos = a.iter().rposition(|e| e.get("kind").and_then(Value::as_str) == Some(kind)).map_or(a.len(), |i| i + 1);
        a.insert(pos, new);
    }
    f.insert("selected".into(), json!(id));
}

/// Removes an instance; the selection moves to a sibling of the same kind,
/// else to the blending page.
fn remove_instance(f: &mut Map<String, Value>, id: &str) {
    let kind = entry(f, id).and_then(|e| e.get("kind").and_then(Value::as_str)).map(str::to_string);
    if let Some(a) = f.get_mut("effects").and_then(Value::as_array_mut) {
        a.retain(|e| e.get("id").and_then(Value::as_str) != Some(id));
    }
    let selected = f.get("selected").and_then(Value::as_str).unwrap_or_default().to_string();
    // Also when one of a removed bevel's elements (Contour, Texture) was showing.
    if split_part(&selected).0 == id {
        let sibling = kind
            .as_deref()
            .and_then(|kind| effects_of(f).iter().find(|e| e.get("kind").and_then(Value::as_str) == Some(kind)))
            .and_then(|e| e.get("id").and_then(Value::as_str))
            .map(str::to_string);
        f.insert("selected".into(), json!(sibling.unwrap_or_else(|| STYLES_PAGE.into())));
    }
}

/// Dialog fields for a layer: one entry per effect instance (all of them, in
/// the layer's order), the blending options and the selected page. `select`
/// names a kind to open: its first instance is selected (and enabled), or one
/// is created. `light` is the document's global light angle.
pub fn initial_fields(layer: &Layer, select: Option<&str>, light: f32) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert("layer".into(), json!(layer.id.0));
    f.insert("globalLight".into(), json!(light));
    f.insert("preview".into(), json!(true));
    // The Name field; OK renames the layer when it changed (`layerName` is the name it had).
    f.insert("name".into(), json!(layer.name));
    f.insert("layerName".into(), json!(layer.name));
    set_blending_fields(&mut f, layer, prism_color::ColorMode::Rgb);
    let mut effects = Vec::new();
    for (i, e) in layer.effects.items.iter().enumerate() {
        effects.push(json!({
            "id": format!("fx{}", i + 1),
            "kind": kind_of(e),
            "on": e.enabled(),
            "params": values_of(e, light),
            "fx": serde_json::to_value(e).unwrap_or(Value::Null),
        }));
    }
    f.insert("effects".into(), Value::Array(effects));
    let selected = match select {
        None => effects_of(&f).first().and_then(|e| e.get("id").and_then(Value::as_str)).map(str::to_string).unwrap_or_else(|| "dropShadow".into()),
        Some(STYLES_PAGE) => STYLES_PAGE.to_string(),
        Some(BLENDING) => BLENDING.to_string(),
        Some(kind) => {
            let existing = effects_of(&f)
                .iter()
                .find(|e| e.get("kind").and_then(Value::as_str) == Some(kind))
                .map(|e| e.get("id").and_then(Value::as_str).unwrap_or_default().to_string());
            match existing {
                Some(id) => {
                    if let Some(e) = entry_mut(&mut f, &id) {
                        e["on"] = json!(true);
                    }
                    id
                }
                None => {
                    add_instance(&mut f, kind);
                    f.get("selected").and_then(Value::as_str).unwrap_or_default().to_string()
                }
            }
        }
    };
    f.insert("selected".into(), json!(selected));
    f
}

pub fn open(app: &mut PrismApp, select: Option<&str>) -> Option<u64> {
    let st = app.session.active()?;
    let layer = st.doc.layer(st.active_layer?)?.clone();
    let light = st.doc.global_light.angle;
    let mode = st.doc.mode;
    let mut f = initial_fields(&layer, select, light);
    // Channels and Blend If list the document's own colour channels.
    set_blending_fields(&mut f, &layer, mode);
    f.insert("patternList".into(), pattern_list(app));
    Some(app.ui.open_dialog(crate::state::DialogKind::LayerStyle, f))
}

/// `[[id, name], …]` of the patterns a style can use (document's, then the library's).
pub fn pattern_list(app: &PrismApp) -> Value {
    let mut out: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let doc_pats = app.session.active().map(|d| d.doc.patterns.clone()).unwrap_or_default();
    for p in doc_pats.iter().chain(app.session.patterns.items.iter()) {
        if seen.insert(p.id.clone()) {
            out.push(json!([p.id, p.display_name()]));
        }
    }
    Value::Array(out)
}

/// Apply the dialog: replace the layer's effects with the dialog's instances.
pub fn confirm(app: &mut PrismApp, f: &Map<String, Value>) -> Result<Value, String> {
    apply(f, |id, p| app.run(id, p))
}

/// Every parameter group is edited as an object. Validate before preview or confirmation so
/// malformed values supplied through `ui.dialog.set` are reported before any command runs.
fn validate_params(f: &Map<String, Value>) -> Result<(), String> {
    let key = format!("p:{BLENDING}");
    if let Some(value) = f.get(&key)
        && !value.is_object()
    {
        return Err(format!("invalid layer style parameters: `{key}` must be an object"));
    }
    let Some(effects) = f.get("effects") else { return Ok(()) };
    let Some(effects) = effects.as_array() else {
        return Err("invalid layer style parameters: `effects` must be an array".into());
    };
    for (i, e) in effects.iter().enumerate() {
        let Some(e) = e.as_object() else {
            return Err(format!("invalid layer style parameters: `effects[{i}]` must be an object"));
        };
        if let Some(params) = e.get("params")
            && !params.is_object()
        {
            return Err(format!("invalid layer style parameters: `effects[{i}].params` must be an object"));
        }
    }
    Ok(())
}

/// Runs the dialog's commands through `run`: blending options, then the whole
/// effect list in one `layer.layerStyle.replace` (instances keep their order;
/// their snapshots carry what the dialog doesn't model).
fn apply(f: &Map<String, Value>, mut run: impl FnMut(&str, Value) -> Result<Value, String>) -> Result<Value, String> {
    validate_params(f)?;
    let layer = f.get("layer").cloned().unwrap_or(Value::Null);
    let initial_light_angle = f.get("globalLight").and_then(Value::as_f64);
    if let (Some(name), Some(old)) = (f.get("name").and_then(Value::as_str), f.get("layerName").and_then(Value::as_str))
        && !name.trim().is_empty()
        && name.trim() != old
    {
        run("layer.renameLayer", json!({"layer": layer.clone(), "name": name.trim()}))?;
    }
    if let Some(bo) = f.get(&format!("p:{BLENDING}")).and_then(Value::as_object) {
        let mut bo = bo.clone();
        // Which Blend If channel the page shows is dialog state, not a parameter.
        bo.remove("blendIfChannel");
        bo.insert("layer".into(), layer.clone());
        let p = Value::Object(bo);
        run("layer.layerStyle.blendingOptions", p)?;
    }
    let mut entries = Vec::new();
    for e in effects_of(f) {
        let (Some(kind), Some(_)) = (e.get("kind").and_then(Value::as_str), e.get("id").and_then(Value::as_str)) else { continue };
        let on = e.get("on").and_then(Value::as_bool).unwrap_or(true);
        let params = e.get("params").cloned().unwrap_or_else(|| json!({}));
        let mut params = params;
        params["enabled"] = json!(on);
        let entry = match e.get("fx").filter(|fx| fx.is_object()) {
            Some(fx) => json!({"kind": kind, "fx": fx, "params": params}),
            None => {
                // A new instance: the page shows the factory defaults merged with
                // the edits, and the engine builds from the full set.
                let mut p = defaults(kind);
                if let (Some(o), Some(pv)) = (p.as_object_mut(), params.as_object()) {
                    for (k, v) in pv {
                        o.insert(k.clone(), v.clone());
                    }
                }
                json!({"kind": kind, "params": p})
            }
        };
        entries.push(entry);
    }
    run("layer.layerStyle.replace", json!({"layer": layer, "effects": entries}))?;
    // An effect lit by the global light follows the document's light angle, so the Angle slider
    // drives that shared angle rather than the per-effect angle the compositor ignores.
    let mut light_angle: Option<f64> = None;
    for e in effects_of(f) {
        if e.get("on").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let params = e.get("params").cloned().unwrap_or_else(|| json!({}));
        if params.get("useGlobalLight").and_then(Value::as_bool) == Some(true)
            && let Some(angle) = params.get("angle").and_then(Value::as_f64)
            && initial_light_angle.is_none_or(|initial| angle != initial)
        {
            light_angle = Some(angle);
        }
    }
    if let Some(angle) = f.get("pendingGlobalLight").and_then(Value::as_f64).or(light_angle) {
        run("layer.layerStyle.globalLight", json!({"angle": angle}))?;
    }
    Ok(Value::Null)
}

/// Hash of the fields that change the rendered style (not the selected page).
pub fn preview_hash(f: &Map<String, Value>) -> u64 {
    f.iter()
        .filter(|(k, _)| matches!(k.as_str(), "layer" | "effects" | "p:blendingOptions" | "pendingGlobalLight"))
        .flat_map(|(k, v)| k.bytes().chain(v.to_string().into_bytes()))
        .fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// `doc` with the dialog's style applied, run on a scratch session (no history) for the live preview.
pub fn preview_document(
    doc: &prism_doc::Document,
    patterns: &prism_engine::pattern_cmds::PatternLibrary,
    f: &Map<String, Value>,
) -> Result<prism_doc::Document, String> {
    let mut s = prism_engine::Session::new();
    s.patterns = patterns.clone();
    s.add_document(doc.clone(), None);
    apply(f, |id, p| s.execute(id, p).map_err(|e| e.to_string()))?;
    s.active().map(|d| (*d.doc).clone()).ok_or_else(|| "no preview document".into())
}

/// Applies a style preset to the dialog fields in-place without touching the document.
/// Clicking Cancel leaves the document untouched; clicking OK commits.
fn apply_style_preset(app: &PrismApp, f: &mut Map<String, Value>, name: &str) {
    let Ok(style) = prism_engine::presets::styles::find_style(&app.session, &json!({"preset": name}), "apply_style_preset") else { return };
    let light = f.get("pendingGlobalLight").or_else(|| f.get("globalLight")).and_then(Value::as_f64).unwrap_or(120.0) as f32;
    let mut effects = Vec::new();
    for (i, e) in style.effects.iter().enumerate() {
        effects.push(json!({
            "id": format!("fx{}", i + 1),
            "kind": kind_of(e),
            "on": e.enabled(),
            "params": values_of(e, light),
            "fx": serde_json::to_value(e).unwrap_or(Value::Null),
        }));
    }
    f.insert("effects".into(), Value::Array(effects));
    if let Some(b) = style.blend {
        set_param(f, BLENDING, "blend", json!(b.label()));
    }
    if let Some(fo) = style.fill_opacity {
        set_param(f, BLENDING, "fillOpacity", json!((fo * 100.0).round()));
    }
}

/// Saves the dialog's pending state (enabled instances + blending page) as a style preset.
fn save_new_style(app: &mut PrismApp, f: &Map<String, Value>) {
    let mut entries = Vec::new();
    for e in effects_of(f) {
        if e.get("on").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let Some(kind) = e.get("kind").and_then(Value::as_str) else { continue };
        let mut params = defaults(kind);
        if let (Some(o), Some(pv)) = (params.as_object_mut(), e.get("params").and_then(Value::as_object)) {
            for (k, v) in pv {
                o.insert(k.clone(), v.clone());
            }
        }
        entries.push(json!([kind, params]));
    }
    let bo = f.get("p:blendingOptions").cloned().unwrap_or_else(|| json!({}));
    if let Ok(v) = app.run(
        "style.presets.new",
        json!({
            "layer": f.get("layer").cloned().unwrap_or(Value::Null),
            "effects": entries,
            "blend": bo.get("blend").cloned().unwrap_or(Value::Null),
            "fillOpacity": bo.get("fillOpacity").cloned().unwrap_or(Value::Null),
        }),
    ) {
        app.ui.status = crate::i18n::fmt(tl!("Style {name} saved"), &[("name", v["name"].as_str().unwrap_or("Style"))]);
    }
}

/// The dialog's layout in points, measured on Photoshop 2026's dialog (macOS, at 2×): a title
/// band, then the Name row over the effect list and the selected page, and the column of OK /
/// Cancel / New Style… / Preview with the style swatch; 770 × 531 pt in all.
pub(crate) const TITLE_H: f32 = 27.0;
/// The body's margins under the title band (left, right, top, bottom).
const PAD: (f32, f32, f32, f32) = (12.0, 10.0, 10.0, 11.0);
/// The effect list's width, its scroll gutter included.
const LIST_W: f32 = 227.0;
/// The scroll gutter down the list's right side, shown whether or not the list scrolls.
const GUTTER_W: f32 = 16.0;
/// The toolbar under the list's rows (fx, the arrows, the trash), inside the list's frame.
const TOOLBAR_H: f32 = 25.0;
const LIST_GAP: f32 = 10.0;
const PAGE_W: f32 = 366.0;
const SIDE_GAP: f32 = 17.0;
const SIDE_W: f32 = 128.0;
/// The Name row and the space under it, above the list and the page.
const NAME_H: f32 = 19.0;
const NAME_GAP: f32 = 11.0;
/// The list's and the page's fixed height. Photoshop's dialog keeps one size whichever page is
/// shown: this fits the tallest page (Blending Options with Blend If) and shorter pages leave the
/// rest blank, so picking another effect never resizes the window. A page that would still not
/// fit (a larger UI font) scrolls instead of growing the dialog.
pub(crate) const PAGE_H: f32 = 454.0;
const ROW_H: f32 = 26.0;
/// The right-aligned label column of a page row ("Blend Mode:"), then the gap before its controls.
const LABEL_W: f32 = 74.0;
/// A page row's height and the space between rows.
const FIELD_H: f32 = 18.0;
const ROW_GAP: f32 = 3.0;
/// A titled box's inner padding at the sides, where its title starts, and the space between
/// two boxes stacked on a page.
const BOX_PAD: f32 = 10.0;
const TITLE_INSET: f32 = 19.0;
const BOX_GAP: f32 = 11.0;
/// Check boxes, in the list and on the pages.
const CHECK: f32 = 11.0;
/// Photoshop's dialog buttons: OK, Cancel, New Style…, Make Default, Reset to Default.
const BUTTON_H: f32 = 25.0;
/// Label text on the pages and in the list.
fn label_font() -> egui::FontId {
    egui::FontId::proportional(12.5)
}
/// Number fields on the pages: Photoshop's short box with proportional digits.
fn field_look() -> widgets::FieldLook {
    widgets::FieldLook { height: 18.0, font: egui::FontId::proportional(12.5), lift: 0.5, fixed2: false, format: None }
}
/// Where the dialog keeps how tall the selected page's content is (view state, egui memory).
fn page_content_id() -> egui::Id {
    egui::Id::new("layer-style-page-content-h")
}

/// How tall the last drawn page's content was; above [`PAGE_H`] it scrolls.
#[cfg(test)]
pub(crate) fn page_content_height(ctx: &egui::Context) -> Option<f32> {
    ctx.data(|d| d.get_temp(page_content_id()))
}

/// Adds an instance of every effect kind and returns every page id the list can select: Styles,
/// Blending Options, each instance and the bevel's Contour and Texture.
#[cfg(test)]
pub(crate) fn add_every_page(f: &mut Map<String, Value>) -> Vec<String> {
    for &(kind, _) in KINDS {
        add_instance(f, kind);
    }
    let mut pages = vec![STYLES_PAGE.to_string(), BLENDING.to_string()];
    for e in effects_of(f) {
        let (Some(id), Some(kind)) = (e.get("id").and_then(Value::as_str), e.get("kind").and_then(Value::as_str)) else { continue };
        pages.push(id.to_string());
        if kind == "bevelEmboss" {
            pages.extend(BEVEL_PARTS.iter().map(|p| format!("{id}#{}", p.0)));
        }
    }
    pages
}

/// The dialog's width, edge to edge: the margins, the three columns and the gaps between them.
pub const WIDTH: f32 = PAD.0 + LIST_W + LIST_GAP + PAGE_W + SIDE_GAP + SIDE_W + PAD.1;

/// Photoshop's title band across the dialog's top, `height` tall: its name centred on a darker
/// band with a dark line under it, and a close button at its right end off macOS (on macOS the
/// system's window buttons sit over the band's left end in a window of its own). `corner` is the
/// dialog frame's rounding, kept on the band's top corners. Returns the band, which drags the
/// dialog, and whether the close button was clicked.
pub fn title_bar(ui: &mut egui::Ui, title: &str, id: egui::Id, corner: egui::CornerRadius, height: f32) -> (egui::Response, bool) {
    let t = Tokens::get(ui.ctx());
    let spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    let (bar, _) = ui.allocate_exact_size(vec2(WIDTH, height), Sense::hover());
    ui.spacing_mut().item_spacing.y = spacing;
    let p = ui.painter();
    p.rect_filled(bar, egui::CornerRadius { sw: 0, se: 0, ..corner }, t.dock);
    p.hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, Color32::from_black_alpha(170)));
    let galley = p.layout_no_wrap(title.to_owned(), crate::theme::semibold(13.0), t.text_dim);
    p.galley(bar.center() - galley.size() / 2.0, galley, t.text_dim);
    // Painted text: name the band, so tests and agents find the dialog by its title.
    let resp = ui.interact(bar, id.with("title"), Sense::drag());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, title));
    let mut close = false;
    if !cfg!(target_os = "macos") {
        // The same answer as Cancel (no accessibility name of its own, so it never doubles it).
        let r = egui::Rect::from_center_size(egui::pos2(bar.right() - 20.0, bar.center().y), vec2(30.0, height - 6.0));
        let hit = ui.interact(r, id.with("close"), Sense::click());
        if hit.hovered() {
            ui.painter().rect_filled(r, t.radius_sm, t.hover);
        }
        let (c, s) = (r.center(), 4.5);
        let ink = Stroke::new(1.3, if hit.hovered() { t.text } else { t.text_dim });
        ui.painter().line_segment([c + vec2(-s, -s), c + vec2(s, s)], ink);
        ui.painter().line_segment([c + vec2(-s, s), c + vec2(s, -s)], ink);
        close = hit.clicked();
    }
    (resp, close)
}

/// The enable checkbox of an effect row.
fn fx_checkbox_rect(row: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(row.left() + 5.0, row.center().y - CHECK / 2.0), vec2(CHECK, CHECK))
}

/// Photoshop's check box: a light box with a dark tick when on, a dark box with a light edge when
/// off (brighter while the pointer is over it).
fn paint_check(p: &egui::Painter, cb: egui::Rect, on: bool, hovered: bool, t: &Tokens) {
    if on {
        p.rect_filled(cb, 2.0, if hovered { t.text } else { t.text_dim });
        let at = |x: f32, y: f32| cb.min + vec2(cb.width() * x, cb.height() * y);
        let tick = Stroke::new(1.6, t.canvas);
        p.line_segment([at(0.22, 0.52), at(0.43, 0.73)], tick);
        p.line_segment([at(0.43, 0.73), at(0.79, 0.29)], tick);
    } else {
        p.rect_filled(cb, 2.0, t.field);
        p.rect_stroke(cb, 2.0, Stroke::new(1.0, if hovered { t.text_dim } else { t.text_faint }), StrokeKind::Inside);
    }
}

/// A page's check box and its label; a click on either toggles it.
fn check_box(ui: &mut egui::Ui, on: &mut bool, label: &str) -> egui::Response {
    check_box_sized(ui, on, label, CHECK, label_font())
}

fn check_box_sized(ui: &mut egui::Ui, on: &mut bool, label: &str, size: f32, font: egui::FontId) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let text = tl!(label);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, t.text_dim);
    let gap = 8.0;
    let (rect, mut resp) = ui.allocate_exact_size(vec2(size + gap + galley.size().x, size + 3.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, text));
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let cb = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.center().y - size / 2.0), vec2(size, size));
    paint_check(ui.painter(), cb, *on, resp.hovered(), &t);
    if resp.has_focus() {
        ui.painter().rect_stroke(cb.expand(2.0), 3.0, Stroke::new(1.5, t.accent), StrokeKind::Outside);
    }
    ui.painter().galley(egui::pos2(cb.right() + gap, rect.center().y - galley.size().y / 2.0), galley, t.text_dim);
    resp
}

/// Photoshop's dialog button: a pill with a thin edge, brighter on the default button (OK).
fn pill_button(ui: &mut egui::Ui, label: &str, width: f32, default: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(13.0), t.text);
    let size = vec2(width.max(galley.size().x + 28.0), BUTTON_H);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    // Painted text: name the button for accessibility (and so tests and agents can find it).
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    let r = rect.height() / 2.0;
    let p = ui.painter();
    if resp.is_pointer_button_down_on() {
        p.rect_filled(rect, r, t.pressed);
    } else if resp.hovered() {
        p.rect_filled(rect, r, t.hover);
    }
    let edge = if default { t.text } else { t.card.lerp_to_gamma(t.text, 0.3) };
    p.rect_stroke(rect, r, Stroke::new(1.0, edge), StrokeKind::Inside);
    if resp.has_focus() {
        p.rect_stroke(rect.expand(2.0), r + 2.0, Stroke::new(2.0, t.accent), StrokeKind::Outside);
    }
    p.galley(rect.center() - galley.size() / 2.0, galley, t.text);
    resp
}

fn selected_fill(t: &Tokens) -> Color32 {
    t.row_selected.gamma_multiply(if t.pro { 1.0 } else { 0.0 }).max_alpha(t.hover)
}

/// What a click on an effect list row hit.
#[derive(Default)]
struct RowClicks {
    row: bool,
    check: bool,
    plus: bool,
}

/// One row of the effect list, a band with an etched line under it as in Photoshop, lighter when
/// selected; `check` draws the enable box, `plus` the add-another-instance button at its right end.
/// `indent` shifts the box and label right (Bevel & Emboss's Contour and Texture).
fn list_row(ui: &mut egui::Ui, id: egui::Id, label: &str, indent: f32, check: Option<bool>, selected: bool, plus: bool) -> RowClicks {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 0.0, selected_fill(&t));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.5));
    }
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, t.separator));
    let mut out = RowClicks::default();
    let text_x = match check {
        Some(on) => {
            let cb = fx_checkbox_rect(rect).translate(vec2(indent, 0.0));
            let cresp = ui.interact(cb.expand(4.0), id.with("check"), Sense::click());
            paint_check(ui.painter(), cb, on, cresp.hovered(), &t);
            out.check = cresp.clicked();
            25.0
        }
        None => 5.0,
    };
    // Photoshop draws every name alike, on or off; the box says which.
    ui.painter().text(rect.left_center() + vec2(text_x + indent, 0.0), egui::Align2::LEFT_CENTER, tl!(label), egui::FontId::proportional(13.0), t.text_dim);
    if plus {
        // A boxed plus at the row's right end, as in Photoshop.
        let r = egui::Rect::from_center_size(rect.right_center() - vec2(11.0, 0.0), vec2(13.0, 13.0));
        let presp = ui.interact(r.expand(3.0), id.with("plus"), Sense::click()).on_hover_text(tl!("Add a new instance of this effect"));
        let ink = Stroke::new(1.0, if presp.hovered() { t.accent } else { t.text });
        ui.painter().rect_stroke(r, 1.0, ink, StrokeKind::Inside);
        ui.painter().hline((r.left() + 3.0)..=(r.right() - 3.0), r.center().y, ink);
        ui.painter().vline(r.center().x, (r.top() + 3.0)..=(r.bottom() - 3.0), ink);
        out.plus = presp.clicked();
    }
    out.row = resp.clicked() && !out.check && !out.plus;
    out
}

/// Bevel & Emboss's elements listed under it: (page suffix, label, spec). Their pages are
/// selected as `<bevel id>#<suffix>` and edit that bevel's params.
const BEVEL_PARTS: &[(&str, &str, &str)] = &[("contour", "Contour", "bevelContour"), ("texture", "Texture", "bevelTexture")];

/// A selected page id split into the instance and the bevel element it shows, if any.
fn split_part(selected: &str) -> (&str, Option<&str>) {
    match selected.split_once('#') {
        Some((id, part)) if BEVEL_PARTS.iter().any(|p| p.0 == part) => (id, Some(part)),
        _ => (selected, None),
    }
}

/// Whether a bevel's element is on: Contour is a flag, Texture a pattern (off: `false` or "").
fn part_on(f: &Map<String, Value>, id: &str, part: &str) -> bool {
    let p = entry(f, id).and_then(|e| e.get("params"));
    match part {
        "contour" => p.and_then(|p| p.get("contour")).and_then(Value::as_bool).unwrap_or(false),
        _ => p.and_then(|p| p.get("texture")).and_then(Value::as_str).is_some_and(|t| !t.is_empty()),
    }
}

/// Turns a bevel's element on or off. Texture needs a pattern, the first the dialog lists; with
/// no patterns at all it stays off. Turning one on turns the bevel on.
fn set_part(f: &mut Map<String, Value>, id: &str, part: &str, on: bool) {
    let value = match (part, on) {
        ("contour", on) => json!(on),
        (_, false) => json!(false),
        (_, true) => {
            let first = f.get("patternList").and_then(Value::as_array).and_then(|a| a.first()).and_then(|e| e.get(0)).and_then(Value::as_str);
            match first {
                Some(p) => json!(p),
                None => return,
            }
        }
    };
    set_param(f, id, part, value);
    if on && let Some(e) = entry_mut(f, id) {
        e["on"] = json!(true);
    }
}

/// Shows `kind`'s page as the list's fx menu does: its first instance, turned on, or a new one.
fn show_kind(f: &mut Map<String, Value>, kind: &str) {
    let first = effects_of(f)
        .iter()
        .find(|e| e.get("kind").and_then(Value::as_str) == Some(kind))
        .and_then(|e| e.get("id").and_then(Value::as_str))
        .map(str::to_string);
    match first {
        Some(id) => {
            if let Some(e) = entry_mut(f, &id) {
                e["on"] = json!(true);
            }
            f.insert("selected".into(), json!(id));
        }
        None => add_instance(f, kind),
    }
}

/// The positions of instance `id` and of the next instance of its kind towards the top (`up`) or
/// the bottom of the list, when there is one: what the move arrows swap.
fn sibling(f: &Map<String, Value>, id: &str, up: bool) -> Option<(usize, usize)> {
    let list = effects_of(f);
    let kind_at = |i: usize| list.get(i).and_then(|e| e.get("kind")).and_then(Value::as_str);
    let i = list.iter().position(|e| e.get("id").and_then(Value::as_str) == Some(id))?;
    let kind = kind_at(i)?;
    let j = if up { (0..i).rev().find(|&j| kind_at(j) == Some(kind))? } else { (i + 1..list.len()).find(|&j| kind_at(j) == Some(kind))? };
    Some((i, j))
}

/// Moves instance `id` past its neighbour of the same kind (the up and down arrows).
fn move_instance(f: &mut Map<String, Value>, id: &str, up: bool) {
    if let Some((i, j)) = sibling(f, id, up)
        && let Some(a) = f.get_mut("effects").and_then(Value::as_array_mut)
        && i.max(j) < a.len()
    {
        a.swap(i, j);
    }
}

/// The effect list (Styles, Blending Options, then every kind and its instances) in Photoshop's
/// frame: the rows beside a scroll gutter, then the toolbar (the fx menu, move up / down, delete)
/// along the frame's bottom. As tall as the page beside it.
fn effect_list(ui: &mut egui::Ui, f: &mut Map<String, Value>, selected: &str) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(LIST_W, PAGE_H), Sense::hover());
    let rows = egui::Rect::from_min_max(rect.min + vec2(1.0, 1.0), egui::pos2(rect.right() - 1.0, rect.bottom() - 2.0 - TOOLBAR_H));
    let gutter = egui::Rect::from_min_max(egui::pos2(rows.right() - GUTTER_W, rows.top()), rows.max);
    let p = ui.painter();
    p.rect_filled(gutter, 0.0, t.field.lerp_to_gamma(t.card, 0.5));
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, t.card_border), StrokeKind::Inside);
    p.hline(rect.x_range(), rows.bottom() + 0.5, Stroke::new(1.0, t.separator));
    let mut list = ui.new_child(egui::UiBuilder::new().id_salt("layer-style-list").max_rect(rows).layout(egui::Layout::top_down(egui::Align::Min)));
    // The scroll bar, when the rows overflow, runs in the gutter.
    egui::ScrollArea::vertical().id_salt("layer-style-list").max_height(rows.height()).auto_shrink([false, false]).show(&mut list, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.set_width(rows.width() - GUTTER_W);
        effect_rows(ui, f, selected);
    });
    let bar = egui::Rect::from_min_max(egui::pos2(rect.left() + 6.0, rows.bottom() + 1.0), rect.max - vec2(6.0, 1.0));
    let mut tools = ui.new_child(egui::UiBuilder::new().id_salt("layer-style-tools").max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Center)));
    list_toolbar(&mut tools, f, selected);
}

fn effect_rows(ui: &mut egui::Ui, f: &mut Map<String, Value>, selected: &str) {
    for (id, label) in [(STYLES_PAGE, "Styles"), (BLENDING, "Blending Options")] {
        if list_row(ui, ui.id().with(("fxrow", id)), label, 0.0, None, selected == id, false).row {
            f.insert("selected".into(), json!(id));
        }
    }
    for &(kind, label) in KINDS {
        let ids: Vec<String> = effects_of(f)
            .iter()
            .filter(|e| e.get("kind").and_then(Value::as_str) == Some(kind))
            .filter_map(|e| e.get("id").and_then(Value::as_str).map(str::to_string))
            .collect();
        if ids.is_empty() {
            // The effect isn't on the layer: an empty checkbox (#1357); a click anywhere adds it,
            // turned on, as Photoshop turns an effect on when its name is clicked.
            let c = list_row(ui, ui.id().with(("fxrow", kind)), label, 0.0, Some(false), false, multi(kind));
            if c.row || c.check || c.plus {
                add_instance(f, kind);
            }
            if kind == "bevelEmboss" {
                // Photoshop lists Contour and Texture even before there is a bevel: a click adds
                // the bevel with that element on.
                for &(part, part_label, _) in BEVEL_PARTS {
                    let c = list_row(ui, ui.id().with(("fxrow", kind, part)), part_label, 10.0, Some(false), false, false);
                    if c.row || c.check {
                        add_instance(f, kind);
                        let id = f.get("selected").and_then(Value::as_str).unwrap_or_default().to_string();
                        set_part(f, &id, part, true);
                        f.insert("selected".into(), json!(format!("{id}#{part}")));
                    }
                }
            }
            continue;
        }
        for id in &ids {
            let on = entry(f, id).and_then(|e| e.get("on")).and_then(Value::as_bool).unwrap_or(false);
            let c = list_row(ui, ui.id().with(("fxrow", kind, id.as_str())), label, 0.0, Some(on), id == selected, multi(kind));
            if c.check {
                if let Some(e) = entry_mut(f, id) {
                    e["on"] = json!(!on);
                }
                f.insert("selected".into(), json!(id));
            } else if c.plus {
                add_instance(f, kind);
            } else if c.row {
                // Photoshop: clicking an effect's name shows its page and turns it on.
                if let Some(e) = entry_mut(f, id) {
                    e["on"] = json!(true);
                }
                f.insert("selected".into(), json!(id));
            }
            if kind == "bevelEmboss" {
                for &(part, part_label, _) in BEVEL_PARTS {
                    let page = format!("{id}#{part}");
                    let on = part_on(f, id, part);
                    let c = list_row(ui, ui.id().with(("fxrow", kind, id.as_str(), part)), part_label, 10.0, Some(on), page == selected, false);
                    if c.check {
                        set_part(f, id, part, !on);
                        f.insert("selected".into(), json!(page));
                    } else if c.row {
                        if !on {
                            set_part(f, id, part, true);
                        }
                        f.insert("selected".into(), json!(page));
                    }
                }
            }
        }
    }
}

/// The toolbar under the list: fx (show or add an effect), move the selected instance up or
/// down among its kind, delete it.
fn list_toolbar(ui: &mut egui::Ui, f: &mut Map<String, Value>, selected: &str) {
    let t = Tokens::get(ui.ctx());
    // Contour or Texture selected: the arrows and the trash act on its bevel.
    let selected = split_part(selected).0;
    let instance = entry(f, selected).is_some();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 24.0), Sense::click());
        let resp = resp.on_hover_text(tl!("Add a layer style"));
        let tint = crate::icons::button_chrome(ui, rect, false, resp.hovered());
        ui.painter().text(rect.center() - vec2(4.0, 1.0), egui::Align2::CENTER_CENTER, "fx", crate::theme::semibold(14.0), tint);
        let tip = rect.right_bottom() - vec2(7.0, 7.0);
        ui.painter().add(egui::Shape::convex_polygon(vec![tip + vec2(-3.0, -2.0), tip + vec2(3.0, -2.0), tip + vec2(0.0, 1.5)], t.text_dim, Stroke::NONE));
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_min_width(170.0);
            if ui.button(tl!("Blending Options")).clicked() {
                f.insert("selected".into(), json!(BLENDING));
                ui.close();
            }
            ui.separator();
            for &(kind, label) in KINDS {
                if ui.button(tl!(label)).clicked() {
                    show_kind(f, kind);
                    ui.close();
                }
            }
        });
        ui.add_space(6.0);
        let up = sibling(f, selected, true).is_some();
        if ui.add_enabled_ui(up, |ui| arrow_button(ui, true, "Move Effect Up")).inner.clicked() {
            move_instance(f, selected, true);
        }
        let down = sibling(f, selected, false).is_some();
        if ui.add_enabled_ui(down, |ui| arrow_button(ui, false, "Move Effect Down")).inner.clicked() {
            move_instance(f, selected, false);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled_ui(instance, |ui| crate::icons::button(ui, "trash", 24.0, false, tl!("Delete Effect"))).inner.clicked() {
                remove_instance(f, selected);
            }
        });
    });
}

/// Photoshop's move up / down buttons under the list: a solid block arrow, dimmed while there is
/// nothing to move past.
fn arrow_button(ui: &mut egui::Ui, up: bool, tooltip: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 22.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!(tooltip)));
    let resp = resp.on_hover_text(tl!(tooltip));
    let ink = if !ui.is_enabled() {
        t.text_faint.gamma_multiply(0.7)
    } else if resp.hovered() {
        t.text
    } else {
        t.text_dim
    };
    // The head (a triangle) and the stem (a bar), pointing up or down from the centre.
    let c = rect.center();
    let s = if up { 1.0 } else { -1.0 };
    let head = vec![c + vec2(0.0, -5.5 * s), c + vec2(5.5, 0.0), c + vec2(-5.5, 0.0)];
    ui.painter().add(egui::Shape::convex_polygon(head, ink, Stroke::NONE));
    let stem = egui::Rect::from_two_pos(c + vec2(-2.25, -0.5 * s), c + vec2(2.25, 5.5 * s));
    ui.painter().rect_filled(stem, 0.0, ink);
    resp
}

/// Photoshop's titled group box: a dark etched frame whose title sits in a gap of its top edge.
/// An empty title draws no frame, only the box's side padding. `min_h` is the least height the
/// box takes in all, its title included (0 = as tall as its rows).
fn group_box<R>(ui: &mut egui::Ui, title: &str, min_h: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    if title.is_empty() {
        return ui
            .horizontal_top(|ui| {
                ui.add_space(BOX_PAD);
                ui.vertical(add).inner
            })
            .inner;
    }
    titled_box(
        ui,
        min_h,
        |ui| {
            let t = Tokens::get(ui.ctx());
            ui.add(egui::Label::new(RichText::new(tl!(title)).font(label_font()).color(t.text)).selectable(false));
        },
        add,
    )
}

/// A [`group_box`] whose title is `header`'s widgets (Blend If's channel dropdown sits in the
/// title, as in Photoshop), set into a gap of the frame's top edge.
fn titled_box<R>(ui: &mut egui::Ui, min_h: f32, header: impl FnOnce(&mut egui::Ui), add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    let avail = ui.available_rect_before_wrap();
    let top = avail.top();
    let head = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(TITLE_INSET);
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                header(ui);
            })
            .response
            .rect
        })
        .inner;
    let (pad_top, pad_bottom) = (5.0, 10.0);
    let content_min = (min_h - (ui.cursor().top() - top) - pad_top - pad_bottom).max(0.0);
    let inner = egui::Frame::NONE
        .inner_margin(egui::Margin { left: BOX_PAD as i8, right: BOX_PAD as i8, top: pad_top as i8, bottom: pad_bottom as i8 })
        .show(ui, |ui| {
            ui.set_width((avail.width() - 2.0 * BOX_PAD).max(40.0));
            ui.set_min_height(content_min);
            add(ui)
        });
    // Half a pixel in, so the 1 pt lines land on whole pixels at 1× and 2×.
    let r = egui::Rect::from_min_max(egui::pos2(avail.left(), head.center().y.round()), egui::pos2(avail.right(), inner.response.rect.bottom())).shrink(0.5);
    let stroke = Stroke::new(1.0, t.card_border);
    let p = ui.painter();
    p.hline(r.left()..=(head.left() - 5.0), r.top(), stroke);
    p.hline((head.right() + 5.0)..=r.right(), r.top(), stroke);
    p.vline(r.right(), r.y_range(), stroke);
    p.hline(r.x_range(), r.bottom(), stroke);
    p.vline(r.left(), r.y_range(), stroke);
    inner.inner
}

/// The space between two boxes stacked on a page.
fn box_gap(ui: &mut egui::Ui) {
    ui.add_space((BOX_GAP - ui.spacing().item_spacing.y).max(0.0));
}

/// A page row: the label right-aligned in a fixed column ("Blend Mode:"), then its controls.
fn field_row<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    field_row_h(ui, label, FIELD_H, add)
}

/// A [`field_row`] `height` tall, its label centred on it (the Angle dial's row).
fn field_row_h<R>(ui: &mut egui::Ui, label: &str, height: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(LABEL_W, height), Sense::hover());
        if !label.is_empty() {
            ui.painter().text(r.right_center(), egui::Align2::RIGHT_CENTER, format!("{}:", tl!(label)), label_font(), t.text_dim);
        }
        add(ui)
    })
    .inner
}

/// A check box in the controls' column (no label of its own), Photoshop's tighter row.
fn check_row(ui: &mut egui::Ui, on: &mut bool, label: &str) -> bool {
    // As tall as the box (not a full control row): Photoshop stacks these 17 pt apart.
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(vec2(ui.available_width(), CHECK), layout, |ui| {
        ui.add_space(LABEL_W);
        check_box(ui, on, label).changed()
    })
    .inner
}

/// Slider, number field and unit after the label, as Photoshop lays out Opacity, Size, ….
fn slider_field(ui: &mut egui::Ui, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    field_row(ui, label, |ui| {
        let s = page_slider(ui, v, range.clone());
        let n = widgets::value_field_look(ui, v, range, "", 42.0, &field_look());
        ui.add_space(-4.0);
        ui.label(RichText::new(unit).font(label_font()).color(t.text_dim));
        s.changed() || n.changed()
    })
}

/// Photoshop's dialog slider: a thin track with a light pointer under it, its tip on the value.
fn page_slider(ui: &mut egui::Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, mut resp) = ui.allocate_exact_size(vec2(108.0, FIELD_H), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*value), ""));
    let (lo, hi) = (*range.start(), *range.end());
    let c = rect.center().y;
    let track = egui::Rect::from_min_max(egui::pos2(rect.left() + 7.0, c - 6.0), egui::pos2(rect.right() - 7.0, c - 3.25));
    if let Some(p) = resp.interact_pointer_pos()
        && (resp.dragged() || resp.clicked())
        && hi > lo
    {
        let v = lo + ((p.x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0) * (hi - lo);
        if (v - *value).abs() > f32::EPSILON {
            *value = v;
            resp.mark_changed();
        }
    }
    let f = if hi > lo { ((*value - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
    let x = track.left() + f * track.width();
    let p = ui.painter();
    p.rect_filled(track, 1.0, t.field_border);
    // The pointer: a house shape whose tip touches the track.
    let tip = c - 5.0;
    let pts = vec![
        egui::pos2(x, tip),
        egui::pos2(x + 5.0, tip + 5.5),
        egui::pos2(x + 5.0, c + 6.5),
        egui::pos2(x - 5.0, c + 6.5),
        egui::pos2(x - 5.0, tip + 5.5),
    ];
    let edge = if resp.hovered() || resp.dragged() { t.accent } else { t.canvas };
    p.add(egui::Shape::convex_polygon(pts, t.text, Stroke::new(1.0, edge)));
    resp
}

/// A blend mode dropdown.
fn blend_dropdown(ui: &mut egui::Ui, id: &str, cur: &mut String) -> bool {
    let opts: Vec<(String, &str)> = prism_color::BlendMode::LAYER_MODES.iter().map(|m| (m.label().to_string(), m.label())).collect();
    widgets::dropdown(ui, id, cur, &opts, 104.0)
}

/// A colour swatch editing `#rrggbb` text; returns the new value when it changed.
fn hex_swatch(ui: &mut egui::Ui, hex: &str) -> Option<Value> {
    let mut c = parse_hex(hex);
    widgets::color_edit_button_srgba(ui, &mut c).changed().then(|| json!(format!("#{r:02x}{g:02x}{b:02x}", r = c.r(), g = c.g(), b = c.b())))
}

/// The dialog body: the Name field over the effect list and the selected page, and Photoshop's
/// button column (OK, Cancel, New Style…, Preview, the swatch). Returns `Some(true)` for OK and
/// `Some(false)` for Cancel.
pub fn body(app: &mut PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> Option<bool> {
    let selected = f.get("selected").and_then(Value::as_str).unwrap_or("dropShadow").to_string();
    let mut style_click: Option<String> = None;
    let mut new_style = false;
    let mut outcome = None;
    let invalid = validate_params(f).err();
    let margin = egui::Margin { left: PAD.0 as i8, right: PAD.1 as i8, top: PAD.2 as i8, bottom: PAD.3 as i8 };
    egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
        // Every gap below is spelled out, as measured on Photoshop's dialog.
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                name_row(ui, f);
                ui.add_space(NAME_GAP);
                // Both columns have fixed sizes: the list and the page end on one line and the
                // dialog keeps its size whichever page is selected.
                ui.horizontal_top(|ui| {
                    effect_list(ui, f, &selected);
                    ui.add_space(LIST_GAP);
                    ui.vertical(|ui| {
                        ui.set_width(PAGE_W);
                        ui.set_height(PAGE_H);
                        // Photoshop's compact page rows: 20 pt controls, 3 pt apart.
                        let s = ui.spacing_mut();
                        s.item_spacing = vec2(8.0, ROW_GAP);
                        s.interact_size.y = FIELD_H;
                        s.button_padding.y = 2.0;
                        let out = egui::ScrollArea::vertical().id_salt("layer-style-page").max_height(PAGE_H).auto_shrink([false, false]).show(ui, |ui| {
                            page(app, ui, f, &selected, invalid.as_deref(), &mut style_click);
                        });
                        ui.data_mut(|d| d.insert_temp(page_content_id(), out.content_size.y));
                    });
                });
            });
            ui.add_space(SIDE_GAP);
            ui.vertical(|ui| {
                ui.set_width(SIDE_W);
                side_column(app, ui, f, &mut outcome, &mut new_style);
            });
        });
    });
    // Style swatch clicks load the style into the dialog (OK applies it, Cancel drops it);
    // New Style… saves the dialog's pending state as a preset.
    if let Some(name) = style_click {
        apply_style_preset(app, f, &name);
    }
    if new_style {
        save_new_style(app, f);
    }
    outcome
}

/// The layer's name, renamed on OK as in Photoshop's dialog.
/// The row keeps its height without a name, so the list and the page never move.
fn name_row(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let name = f.get("name").and_then(Value::as_str).map(str::to_string);
    ui.horizontal(|ui| {
        ui.set_height(NAME_H);
        // The label right-aligned before the field, which lines up over the list's checkboxes.
        let (r, _) = ui.allocate_exact_size(vec2(45.0, NAME_H), Sense::hover());
        ui.painter().text(r.right_center(), egui::Align2::RIGHT_CENTER, format!("{}:", tl!("Name")), label_font(), t.text_dim);
        ui.add_space(5.0);
        if let Some(mut name) = name {
            let edit = egui::TextEdit::singleline(&mut name).font(label_font()).margin(vec2(4.0, 1.0)).vertical_align(egui::Align::Center);
            if ui.add_sized(vec2(LIST_W, NAME_H), edit).changed() {
                f.insert("name".into(), json!(name));
            }
        }
    });
}

/// OK, Cancel, New Style…, Preview and the swatch of the pending style, spaced as in Photoshop.
fn side_column(app: &PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, outcome: &mut Option<bool>, new_style: &mut bool) {
    if pill_button(ui, tl!("OK"), SIDE_W, true).clicked() {
        *outcome = Some(true);
    }
    ui.add_space(10.0);
    if pill_button(ui, tl!("Cancel"), SIDE_W, false).clicked() {
        *outcome = Some(false);
    }
    ui.add_space(10.0);
    if pill_button(ui, tl!("New Style…"), SIDE_W, false).clicked() {
        *new_style = true;
    }
    ui.add_space(11.0);
    let mut preview = f.get("preview").and_then(Value::as_bool).unwrap_or(true);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        if check_box_sized(ui, &mut preview, "Preview", 12.5, egui::FontId::proportional(13.5)).changed() {
            f.insert("preview".into(), json!(preview));
        }
    });
    ui.add_space(11.5);
    preview_swatch(app, ui, f);
}

/// The pending style as a preset: the enabled instances with their page values.
fn pending_style(f: &Map<String, Value>) -> prism_engine::presets::styles::StylePreset {
    let mut effects = Vec::new();
    for e in effects_of(f) {
        if e.get("on").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let Some(kind) = e.get("kind").and_then(Value::as_str) else { continue };
        let mut params = e.get("params").filter(|p| p.is_object()).cloned().unwrap_or_else(|| json!({}));
        // The swatch has no document light: each effect is drawn at the angle its page shows.
        params["useGlobalLight"] = json!(false);
        params["enabled"] = json!(true);
        let snapshot = e.get("fx").filter(|v| v.is_object()).and_then(|v| serde_json::from_value::<Effect>(v.clone()).ok());
        let fx = match snapshot {
            Some(mut fx) => {
                prism_engine::layer_style::overlay_effect(&mut fx, &params);
                Some(fx)
            }
            None => {
                let mut p = defaults(kind);
                if let (Some(o), Some(pv)) = (p.as_object_mut(), params.as_object()) {
                    for (k, v) in pv {
                        o.insert(k.clone(), v.clone());
                    }
                }
                prism_engine::layer_style::effect_from_params(kind, &p)
            }
        };
        effects.extend(fx);
    }
    let fill = f.get(&format!("p:{BLENDING}")).and_then(|b| b.get("fillOpacity")).and_then(Value::as_f64);
    prism_engine::presets::styles::StylePreset { name: String::new(), effects, blend: None, fill_opacity: fill.map(|v| (v / 100.0).clamp(0.0, 1.0) as f32) }
}

/// Photoshop's preview swatch: the pending style on a grey square in a dark well, redrawn when
/// it changes.
fn preview_swatch(app: &PrismApp, ui: &mut egui::Ui, f: &Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(SIDE_W, SIDE_W + 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, t.canvas);
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Inside);
    let inner = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(rect.width().min(rect.height()) - 2.0));
    let px = (inner.width() * ui.ctx().pixels_per_point()).round().clamp(32.0, 256.0) as u32;
    let key = egui::Id::new("layer-style-swatch");
    let hash = preview_hash(f) ^ u64::from(px).rotate_left(32);
    let cached: Option<(u64, egui::TextureHandle)> = ui.data(|d| d.get_temp(key));
    let tex = match cached {
        Some((h, tex)) if h == hash => Some(tex),
        _ => {
            let rgba = prism_engine::presets::styles::dialog_preview(&app.session, &pending_style(f), px);
            let side = px as usize;
            (rgba.len() == side * side * 4).then(|| {
                let img = egui::ColorImage::from_rgba_unmultiplied([side, side], &rgba);
                let tex = ui.ctx().load_texture("layer-style-swatch", img, egui::TextureOptions::LINEAR);
                ui.data_mut(|d| d.insert_temp(key, (hash, tex.clone())));
                tex
            })
        }
    };
    if let Some(tex) = tex {
        ui.painter().image(tex.id(), inner, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    }
}

/// The selected page in its titled box: the Styles gallery, Blending Options or an effect's
/// groups, with Make Default / Reset to Default under an effect's.
fn page(app: &mut PrismApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, selected: &str, invalid: Option<&str>, style_click: &mut Option<String>) {
    let t = Tokens::get(ui.ctx());
    // The page's box fills the fixed page height, whatever the page shows.
    let min_h = PAGE_H;
    if selected == STYLES_PAGE {
        group_box(ui, "Styles", min_h, |ui| styles_gallery(app, ui, style_click));
        return;
    }
    // Bevel & Emboss › Contour / Texture: their rows, editing the bevel.
    if let (id, Some(part)) = split_part(selected)
        && let Some(&(_, title, rows)) = BEVEL_PARTS.iter().find(|p| p.0 == part)
    {
        group_box(ui, title, min_h, |ui| {
            if let Some(error) = invalid {
                ui.colored_label(t.danger, error);
            } else if entry(f, id).is_some() {
                effect_page(ui, f, id, "bevelEmboss", spec(rows));
            }
        });
        return;
    }
    let sel_kind: String = if selected == BLENDING {
        BLENDING.to_string()
    } else {
        entry(f, selected).and_then(|e| e.get("kind").and_then(Value::as_str)).unwrap_or("").to_string()
    };
    let title = KINDS.iter().find(|k| k.0 == sel_kind).map(|k| k.1).unwrap_or(if selected == BLENDING { "Blending Options" } else { "" });
    group_box(ui, title, min_h, |ui| {
        // Only reachable through `ui.dialog.set`: say what is wrong once, keep the bad value.
        if let Some(error) = invalid {
            ui.colored_label(t.danger, error);
            return;
        }
        if selected == BLENDING {
            let names = channel_names_of(f);
            let key = format!("p:{BLENDING}");
            let Some(mut p) = f.get(&key).filter(|v| v.is_object()).cloned() else { return };
            blending_page(ui, &mut p, &names);
            f.insert(key, p);
            return;
        }
        if entry(f, selected).is_none() {
            return;
        }
        effect_page(ui, f, selected, &sel_kind, spec(&sel_kind));
        // Make Default stores this page as the user default; Reset to Default restores it (or
        // the factory defaults before the first Make). Centred under the page, as in Photoshop.
        box_gap(ui);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let w = 112.0 + 132.0 + ui.spacing().item_spacing.x;
            ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
            if pill_button(ui, tl!("Make Default"), 112.0, false).clicked() {
                let disp = page_values(f, selected, &sel_kind);
                let _ = app.run("layer.layerStyle.makeDefault", json!({"kind": sel_kind, "params": disp}));
            }
            if pill_button(ui, tl!("Reset to Default"), 132.0, false).clicked()
                && let Ok(v) = app.run("layer.layerStyle.defaultFor", json!({"kind": sel_kind}))
                && let Some(e) = entry_mut(f, selected)
            {
                e["params"] = v["params"].clone();
            }
        });
    });
}

/// The Styles page: the preset library as swatches, by group.
fn styles_gallery(app: &PrismApp, ui: &mut egui::Ui, style_click: &mut Option<String>) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    // The swatches scroll inside the page's box: its content height, set by `group_box`.
    let height = ui.min_rect().height().max(ROW_H);
    egui::ScrollArea::vertical().max_height(height).auto_shrink([false, true]).id_salt("layer-style-presets-main").show(ui, |ui| {
        for group in &app.session.presets.styles {
            ui.label(RichText::new(&group.name).size(12.0).color(t.text_dim));
            ui.add_space(3.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for st in &group.items {
                    let tex = crate::preset_panels::style_texture(app, &ctx, st);
                    let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
                    ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    let (stroke, kind) =
                        if resp.hovered() { (Stroke::new(1.5, t.accent), egui::StrokeKind::Outside) } else { (Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside) };
                    ui.painter().rect_stroke(rect, 2.0, stroke, kind);
                    if resp.on_hover_text(&st.name).clicked() {
                        *style_click = Some(st.name.clone());
                    }
                }
            });
            ui.add_space(10.0);
        }
    });
}

/// An instance's page values: its params over the kind's factory defaults (only real edits
/// reach the stored params).
fn page_values(f: &Map<String, Value>, id: &str, kind: &str) -> Value {
    let mut disp = entry(f, id).and_then(|e| e.get("params")).filter(|p| p.is_object()).cloned().unwrap_or_else(|| json!({}));
    if let (Some(o), Some(b)) = (disp.as_object_mut(), defaults(kind).as_object()) {
        for (k, v) in b {
            o.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    disp
}

/// An effect's page: its rows in Photoshop's titled boxes (Structure, Quality, …).
fn effect_page(ui: &mut egui::Ui, f: &mut Map<String, Value>, selected: &str, sel_kind: &str, rows: &[(&str, &str, P)]) {
    // Split the rows at each group marker; each run of rows is drawn in its box.
    let mut start = 0;
    while start < rows.len() {
        let (title, from) = match rows.get(start) {
            Some(&(_, title, P::Group)) => (title, start + 1),
            _ => ("", start),
        };
        let end = rows.iter().skip(from).position(|r| matches!(r.2, P::Group)).map_or(rows.len(), |n| from + n);
        if start > 0 {
            box_gap(ui);
        }
        group_box(ui, title, 0.0, |ui| {
            for &(key, label, kind_p) in rows.get(from..end).unwrap_or(&[]) {
                effect_row(ui, f, selected, sel_kind, key, label, kind_p);
            }
        });
        start = end.max(start + 1);
    }
}

/// One row of an effect page; edits go to the instance's params.
fn effect_row(ui: &mut egui::Ui, f: &mut Map<String, Value>, selected: &str, sel_kind: &str, key: &str, label: &str, kind_p: P) {
    let disp = page_values(f, selected, sel_kind);
    let mut edits: Vec<(&str, Value)> = Vec::new();
    match kind_p {
        P::Group => {}
        P::Angle(check) => {
            let mut v = disp.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let mut on = check.and_then(|(k, _)| disp.get(k)).and_then(Value::as_bool).unwrap_or(false);
            let (resp, toggled) = angle_row(ui, label, &mut v, check.map(|(_, l)| (l, &mut on)));
            if resp.changed() {
                edits.push((key, json!(v)));
            }
            if toggled && let Some((k, _)) = check {
                edits.push((k, json!(on)));
            }
        }
        P::Slider(min, max, unit) => {
            let mut v = disp.get(key).and_then(Value::as_f64).unwrap_or(min as f64) as f32;
            if slider_field(ui, label, &mut v, min..=max, unit) {
                edits.push((key, json!(v.round())));
            }
        }
        P::Color => {
            let hex = disp.get(key).and_then(Value::as_str).unwrap_or("#000000").to_string();
            if let Some(v) = field_row(ui, label, |ui| hex_swatch(ui, &hex)) {
                edits.push((key, v));
            }
        }
        P::Blend(color) => {
            let mut cur = disp.get(key).and_then(Value::as_str).unwrap_or("Normal").to_string();
            let swatch = color.map(|k| (k, disp.get(k).and_then(Value::as_str).unwrap_or("#000000").to_string()));
            field_row(ui, label, |ui| {
                if blend_dropdown(ui, &format!("fx-blend-{sel_kind}"), &mut cur) {
                    edits.push((key, json!(cur)));
                }
                if let Some((k, hex)) = &swatch
                    && let Some(v) = hex_swatch(ui, hex)
                {
                    edits.push((k, v));
                }
            });
        }
        P::Choice(options) => {
            let first = options.first().map_or("", |o| o.0);
            let mut cur = disp.get(key).and_then(Value::as_str).unwrap_or(first).to_string();
            let opts: Vec<(String, &str)> = options.iter().map(|(v, l)| (v.to_string(), *l)).collect();
            if field_row(ui, label, |ui| widgets::dropdown(ui, &format!("fx-{sel_kind}-{key}"), &mut cur, &opts, 132.0)) {
                edits.push((key, json!(cur)));
            }
        }
        P::Radio(options) => {
            let cur = disp.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
            field_row(ui, label, |ui| {
                for (v, l) in options {
                    if ui.radio(cur == *v, tl!(l)).clicked() && cur != *v {
                        edits.push((key, json!(v)));
                    }
                }
            });
        }
        P::Pattern => {
            let mut list: Vec<(String, String)> = f
                .get("patternList")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|e| Some((e.get(0)?.as_str()?.to_string(), e.get(1)?.as_str()?.to_string()))).collect())
                .unwrap_or_default();
            let stored = disp.get(key).and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_string);
            // A pattern the library doesn't have (an imported file's) is still listed by its name.
            if let Some(cur) = &stored
                && !list.iter().any(|(id, name)| id == cur || name == cur)
            {
                list.insert(0, (cur.clone(), cur.clone()));
            }
            // A pattern that is off (`false`: Bevel's Texture) stays off until its box is ticked.
            let off = disp.get(key) == Some(&Value::Bool(false));
            let mut cur = stored.clone().or_else(|| list.first().map(|l| l.0.clone())).unwrap_or_default();
            let opts: Vec<(String, &str)> = list.iter().map(|(id, n)| (id.clone(), n.as_str())).collect();
            if field_row(ui, label, |ui| widgets::dropdown(ui, &format!("fx-{sel_kind}-{key}"), &mut cur, &opts, 160.0)) || (stored.is_none() && !off) {
                edits.push((key, json!(cur)));
            }
        }
        P::Gradient => {
            let from = disp.get("from").and_then(Value::as_str).unwrap_or("#000000").to_string();
            let to = disp.get("to").and_then(Value::as_str).unwrap_or("#ffffff").to_string();
            field_row(ui, label, |ui| {
                if let Some(v) = hex_swatch(ui, &from) {
                    edits.push(("from", v));
                }
                let (r, _) = ui.allocate_exact_size(vec2(110.0, 20.0), Sense::hover());
                let (a, b) = (parse_hex(&from), parse_hex(&to));
                let mut mesh = egui::Mesh::default();
                mesh.colored_vertex(r.left_top(), a);
                mesh.colored_vertex(r.right_top(), b);
                mesh.colored_vertex(r.right_bottom(), b);
                mesh.colored_vertex(r.left_bottom(), a);
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                ui.painter().add(mesh);
                ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, Tokens::get(ui.ctx()).field_border), StrokeKind::Inside);
                if let Some(v) = hex_swatch(ui, &to) {
                    edits.push(("to", v));
                }
            });
        }
        P::Check => {
            let mut b = disp.get(key).and_then(Value::as_bool).unwrap_or(false);
            if check_row(ui, &mut b, label) {
                edits.push((key, json!(b)));
            }
        }
    }
    for (k, v) in edits {
        set_param(f, selected, k, v);
    }
}

/// Colour channel names of a document mode, as Blending Options lists them (Channels, Blend If).
pub fn channel_names(mode: prism_color::ColorMode) -> Vec<&'static str> {
    use prism_color::ColorMode as M;
    match mode {
        M::Rgb | M::Indexed | M::Multichannel => vec!["R", "G", "B"],
        M::Cmyk => vec!["C", "M", "Y", "K"],
        M::Lab => vec!["L", "a", "b"],
        _ => vec!["Gray"],
    }
}

const KNOCKOUT: &[(&str, &str)] = &[("none", "None"), ("shallow", "Shallow"), ("deep", "Deep")];

/// (parameter key, label) of the Advanced Blending check boxes, in Photoshop's order.
const ADVANCED_CHECKS: &[(&str, &str)] = &[
    ("blendInteriorEffectsAsGroup", "Blend Interior Effects as Group"),
    ("blendClippedLayersAsGroup", "Blend Clipped Layers as Group"),
    ("transparencyShapesLayer", "Transparency Shapes Layer"),
    ("layerMaskHidesEffects", "Layer Mask Hides Effects"),
    ("vectorMaskHidesEffects", "Vector Mask Hides Effects"),
];

/// The Blending Options page's fields for `layer` in a `mode` document (the parameters of
/// `layer.layerStyle.blendingOptions`, plus the page's Blend If channel), and the channel names.
pub fn set_blending_fields(f: &mut Map<String, Value>, layer: &Layer, mode: prism_color::ColorMode) {
    let names = channel_names(mode);
    let n = names.len();
    let a = &layer.advanced;
    // Blend If entries in the command's form: index 0 is Gray for colour documents; a one-channel
    // document's Gray is its channel's own entry (index 1).
    let first = if n == 1 { 1 } else { 0 };
    let blend_if: Vec<Value> = (first..=n)
        .map(|i| {
            let [this, under] = layer.blend_if.get(i);
            json!({"channel": i, "thisLayer": this.to_bytes(), "underlying": under.to_bytes()})
        })
        .collect();
    f.insert(
        format!("p:{BLENDING}"),
        json!({
            "blend": layer.blend.label(),
            "opacity": (layer.opacity * 100.0).round(),
            "fillOpacity": (layer.fill_opacity * 100.0).round(),
            "channels": (0..n).map(|i| layer.excluded_channels & (1 << i) == 0).collect::<Vec<_>>(),
            "knockout": a.knockout.name(),
            "blendInteriorEffectsAsGroup": a.blend_interior,
            "blendClippedLayersAsGroup": a.blend_clipped,
            "transparencyShapesLayer": a.transparency_shapes,
            "layerMaskHidesEffects": a.layer_mask_hides_effects,
            "vectorMaskHidesEffects": a.vector_mask_hides_effects,
            "blendIfChannel": 0,
            "blendIf": blend_if,
        }),
    );
    f.insert("channelNames".into(), json!(names));
}

fn channel_names_of(f: &Map<String, Value>) -> Vec<String> {
    f.get("channelNames")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect::<Vec<_>>())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| ["R", "G", "B"].iter().map(|s| (*s).to_string()).collect())
}

/// The Blending Options page: General Blending, Advanced Blending and Blend If, each in its box
/// as in Photoshop.
fn blending_page(ui: &mut egui::Ui, p: &mut Value, names: &[String]) {
    let t = Tokens::get(ui.ctx());
    let num = |p: &Value, k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
    group_box(ui, "General Blending", 0.0, |ui| {
        let mut cur = p.get("blend").and_then(Value::as_str).unwrap_or("Normal").to_string();
        if field_row(ui, "Blend Mode", |ui| blend_dropdown(ui, "fx-blend-blendingOptions", &mut cur)) {
            p["blend"] = json!(cur);
        }
        let mut v = num(p, "opacity", 100.0);
        if slider_field(ui, "Opacity", &mut v, 0.0..=100.0, "%") {
            p["opacity"] = json!(v.round());
        }
    });
    box_gap(ui);
    group_box(ui, "Advanced Blending", 0.0, |ui| {
        let mut v = num(p, "fillOpacity", 100.0);
        if slider_field(ui, "Fill Opacity", &mut v, 0.0..=100.0, "%") {
            p["fillOpacity"] = json!(v.round());
        }
        // Channels: one check box per colour channel (checked = blends), spread across the row
        // as Photoshop spaces them.
        let mut ch: Vec<bool> =
            p.get("channels").and_then(Value::as_array).map(|a| a.iter().map(|v| v.as_bool().unwrap_or(true)).collect()).unwrap_or_default();
        ch.resize(names.len(), true);
        let changed = field_row(ui, "Channels", |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let pitch = (ui.available_width() / names.len().max(1) as f32).min(63.0);
            let mut changed = false;
            for (i, name) in names.iter().enumerate() {
                if let Some(c) = ch.get_mut(i) {
                    let r = check_box(ui, c, name);
                    changed |= r.changed();
                    ui.add_space((pitch - r.rect.width()).max(6.0));
                }
            }
            changed
        });
        if changed {
            p["channels"] = json!(ch);
        }
        let mut cur = p.get("knockout").and_then(Value::as_str).unwrap_or("none").to_string();
        let opts: Vec<(String, &str)> = KNOCKOUT.iter().map(|(v, l)| (v.to_string(), *l)).collect();
        if field_row(ui, "Knockout", |ui| widgets::dropdown(ui, "fx-blendingOptions-knockout", &mut cur, &opts, 72.0)) {
            p["knockout"] = json!(cur);
        }
        for &(key, label) in ADVANCED_CHECKS {
            let default = matches!(key, "blendClippedLayersAsGroup" | "transparencyShapesLayer");
            let mut b = p.get(key).and_then(Value::as_bool).unwrap_or(default);
            if check_row(ui, &mut b, label) {
                p[key] = json!(b);
            }
        }
    });
    // Blend If: its channel in the box's title, then This Layer / Underlying Layer split sliders
    // (Alt-drag splits).
    let entries = p.get("blendIf").and_then(Value::as_array).map_or(0, Vec::len);
    if entries == 0 {
        return;
    }
    let sel = p.get("blendIfChannel").and_then(Value::as_u64).map_or(0, |v| v as usize).min(entries - 1);
    // A colour document lists Gray first; a one-channel document only its channel (Gray).
    let label = |i: usize| -> String {
        if entries == names.len() {
            names.get(i).cloned().unwrap_or_default()
        } else if i == 0 {
            "Gray".to_string()
        } else {
            names.get(i - 1).cloned().unwrap_or_default()
        }
    };
    let labels: Vec<String> = (0..entries).map(label).collect();
    let opts: Vec<(usize, &str)> = labels.iter().enumerate().map(|(i, l)| (i, l.as_str())).collect();
    let mut cur = sel;
    let mut picked = false;
    box_gap(ui);
    titled_box(
        ui,
        0.0,
        |ui| {
            ui.add(egui::Label::new(RichText::new(format!("{}:", tl!("Blend If"))).font(label_font()).color(t.text)).selectable(false));
            picked = widgets::dropdown(ui, "fx-blendingOptions-blendIf", &mut cur, &opts, 64.0);
        },
        |ui| {
            for (i, (key, label)) in [("thisLayer", "This Layer:"), ("underlying", "Underlying Layer:")].into_iter().enumerate() {
                if i > 0 {
                    ui.add_space(4.0);
                }
                let mut q = blend_quad(p.get("blendIf").and_then(|a| a.get(sel)).and_then(|e| e.get(key)));
                // The label in the label column, then the black and the white point at fixed
                // places, as Photoshop prints them ("0", or "0 / 20" once split).
                let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), FIELD_H), Sense::hover());
                let side = |lo: u8, hi: u8| if lo == hi { lo.to_string() } else { format!("{lo} / {hi}") };
                let painter = ui.painter();
                painter.text(egui::pos2(row.left() + LABEL_W, row.center().y), egui::Align2::RIGHT_CENTER, tl!(label), label_font(), t.text_dim);
                painter.text(egui::pos2(row.left() + LABEL_W + 27.0, row.center().y), egui::Align2::LEFT_CENTER, side(q[0], q[1]), label_font(), t.text);
                painter.text(egui::pos2(row.left() + LABEL_W + 92.0, row.center().y), egui::Align2::LEFT_CENTER, side(q[2], q[3]), label_font(), t.text);
                if blend_if_bar(ui, ui.id().with(("blendIf", key)), &mut q)
                    && let Some(e) = p.get_mut("blendIf").and_then(|a| a.get_mut(sel))
                {
                    e[key] = json!(q);
                }
            }
        },
    );
    if picked {
        p["blendIfChannel"] = json!(cur);
    }
}

/// A Blend If range from the dialog's JSON (`[blackLo, blackHi, whiteLo, whiteHi]`), full when absent.
fn blend_quad(v: Option<&Value>) -> [u8; 4] {
    let mut q = [0u8, 0, 255, 255];
    if let Some(a) = v.and_then(Value::as_array) {
        for (dst, x) in q.iter_mut().zip(a) {
            *dst = x.as_f64().map_or(*dst, |f| f.clamp(0.0, 255.0).round() as u8);
        }
    }
    // The sliders never cross.
    for i in 1..4 {
        q[i] = q[i].max(q[i - 1]);
    }
    q
}

/// A Blend If slider: Photoshop's black-to-white ramp with a dark and a light pointer under it,
/// each split in two halves by Alt-dragging (the layer fades between them). Returns whether `q`
/// changed.
fn blend_if_bar(ui: &mut egui::Ui, id: egui::Id, q: &mut [u8; 4]) -> bool {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width().clamp(120.0, 264.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click_and_drag());
    let ramp = egui::Rect::from_min_size(rect.min + vec2(6.0, 0.0), vec2(w - 12.0, 12.0));
    let x_of = |v: u8| ramp.left() + ramp.width() * f32::from(v) / 255.0;
    let v_of = |x: f32| (((x - ramp.left()) / ramp.width().max(1.0)) * 255.0).round().clamp(0.0, 255.0) as u8;
    // The ramp: one smooth mesh, grey levels set every 1/16 so it reads evenly.
    let steps = 16u32;
    let mut mesh = egui::Mesh::default();
    for s in 0..=steps {
        let f = s as f32 / steps as f32;
        let x = ramp.left() + ramp.width() * f;
        let g = Color32::from_gray((f * 255.0).round() as u8);
        mesh.colored_vertex(egui::pos2(x, ramp.top()), g);
        mesh.colored_vertex(egui::pos2(x, ramp.bottom()), g);
    }
    for s in 0..steps {
        let i = s * 2;
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i + 1, i + 3, i + 2);
    }
    ui.painter().add(mesh);
    let mut changed = false;
    let drag_key = id.with("handle");
    if resp.drag_started()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let near = (0..4).min_by(|a, b| (x_of(q[*a]) - pos.x).abs().total_cmp(&(x_of(q[*b]) - pos.x).abs())).unwrap_or(0);
        // Between two coinciding halves, pick by the side of the pointer.
        let near = match near {
            0 | 1 if q[0] == q[1] => usize::from(pos.x > x_of(q[0])),
            2 | 3 if q[2] == q[3] => 2 + usize::from(pos.x > x_of(q[2])),
            n => n,
        };
        ui.data_mut(|d| d.insert_temp(drag_key, near));
    }
    if resp.dragged()
        && let (Some(h), Some(pos)) = (ui.data(|d| d.get_temp::<usize>(drag_key)), resp.interact_pointer_pos())
        && let Some(cur) = q.get(h).copied()
    {
        let target = v_of(pos.x);
        let split = ui.input(|i| i.modifiers.alt);
        let mut n = *q;
        let pair = if h < 2 { 0 } else { 2 };
        if split {
            n[h] = target;
        } else {
            // Unsplit: the pair moves together, keeping its spread.
            let delta = i16::from(target) - i16::from(cur);
            for v in &mut n[pair..pair + 2] {
                *v = (i16::from(*v) + delta).clamp(0, 255) as u8;
            }
        }
        // Keep the order black low <= black high <= white low <= white high, pushing the others.
        for i in 0..4 {
            if i < h {
                n[i] = n[i].min(n[h]);
            } else if i > h {
                n[i] = n[i].max(n[h]);
            }
        }
        for i in 1..4 {
            n[i] = n[i].max(n[i - 1]);
        }
        if n != *q {
            *q = n;
            changed = true;
        }
    }
    // Pointers: Photoshop's house shapes under the ramp, the tip on the value, with a groove down
    // the middle; a split pointer shows its two halves apart.
    let (top, shoulder, bottom) = (ramp.bottom(), ramp.bottom() + 6.0, ramp.bottom() + 11.5);
    let edge = Stroke::new(1.0, t.canvas);
    let house = |x: f32, half: i8, fill: Color32| {
        let pts = match half {
            -1 => vec![egui::pos2(x, top), egui::pos2(x, bottom), egui::pos2(x - 6.0, bottom), egui::pos2(x - 6.0, shoulder)],
            1 => vec![egui::pos2(x, top), egui::pos2(x + 6.0, shoulder), egui::pos2(x + 6.0, bottom), egui::pos2(x, bottom)],
            _ => vec![egui::pos2(x, top), egui::pos2(x + 6.0, shoulder), egui::pos2(x + 6.0, bottom), egui::pos2(x - 6.0, bottom), egui::pos2(x - 6.0, shoulder)],
        };
        ui.painter().add(egui::Shape::convex_polygon(pts, fill, edge));
        if half == 0 {
            ui.painter().vline(x, (shoulder - 1.0)..=(bottom - 2.5), Stroke::new(1.0, t.text_faint));
        }
    };
    for (pair, fill) in [(0usize, t.card), (2, t.text_dim)] {
        let (a, b) = (q[pair], q[pair + 1]);
        if a == b {
            house(x_of(a), 0, fill);
        } else {
            house(x_of(a), -1, fill);
            house(x_of(b), 1, fill);
        }
    }
    changed
}

/// Light direction uses document angles: counter-clockwise from three o'clock. The dial, its
/// number field and, with `check`, a check box beside them (Use Global Light), as in Photoshop.
/// Returns the angle's response and whether the check box toggled.
fn angle_row(ui: &mut egui::Ui, label: &str, angle: &mut f32, check: Option<(&str, &mut bool)>) -> (egui::Response, bool) {
    let t = Tokens::get(ui.ctx());
    field_row_h(ui, label, 30.0, |ui| {
        let (rect, mut dial) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click_and_drag());
        dial.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, ui.is_enabled(), tl!(label)));
        if (dial.clicked() || dial.dragged())
            && let Some(pos) = dial.interact_pointer_pos()
        {
            let delta = pos - rect.center();
            if delta.length_sq() > 1.0 {
                *angle = (-delta.y).atan2(delta.x).to_degrees().round();
                dial.mark_changed();
            }
        }
        let painter = ui.painter();
        let radius = rect.width() / 2.0 - 1.0;
        // Photoshop's dial: a dark disc with a light rim and a needle from the centre to the rim.
        painter.circle(rect.center(), radius, t.field, Stroke::new(1.0, t.text_faint));
        let radians = angle.to_radians();
        let tip = rect.center() + vec2(radians.cos(), -radians.sin()) * (radius - 1.0);
        painter.line_segment([rect.center(), tip], Stroke::new(1.5, t.text));
        painter.circle_filled(rect.center(), 1.5, t.text);
        if dial.has_focus() {
            painter.circle_stroke(rect.center(), radius + 2.0, Stroke::new(1.0, t.accent));
            let step = ui.input(|i| {
                i.num_presses(egui::Key::ArrowRight) as f32 + i.num_presses(egui::Key::ArrowUp) as f32
                    - i.num_presses(egui::Key::ArrowLeft) as f32
                    - i.num_presses(egui::Key::ArrowDown) as f32
            });
            if step != 0.0 {
                *angle = (*angle + step + 180.0).rem_euclid(360.0) - 180.0;
                dial.mark_changed();
            }
        }
        let field = widgets::value_field_look(ui, angle, -180.0..=180.0, "", 42.0, &field_look());
        ui.add_space(-4.0);
        ui.label(RichText::new("°").font(label_font()).color(t.text_dim));
        if field.changed() {
            dial.mark_changed();
        }
        let toggled = check.is_some_and(|(label, on)| {
            ui.add_space(4.0);
            check_box(ui, on, label).changed()
        });
        (dial | field, toggled)
    })
}

fn parse_hex(s: &str) -> Color32 {
    let s = s.trim_start_matches('#');
    let b = |i: usize| u8::from_str_radix(s.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0);
    Color32::from_rgb(b(0), b(2), b(4))
}

trait MaxAlpha {
    fn max_alpha(self, other: Color32) -> Color32;
}

impl MaxAlpha for Color32 {
    /// Fall back to `other` when this colour is fully transparent (non-Pro themes have no row colour).
    fn max_alpha(self, other: Color32) -> Color32 {
        if self.a() == 0 { other } else { self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_doc::StrokeFx;

    fn session() -> prism_engine::Session {
        let mut s = prism_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s
    }

    fn strokes_of(s: &prism_engine::Session) -> Vec<StrokeFx> {
        let d = s.active().unwrap();
        let id = d.active_layer.unwrap();
        d.doc
            .layer(id)
            .unwrap()
            .effects
            .items
            .iter()
            .filter_map(|e| match e {
                Effect::Stroke(st) => Some(st.clone()),
                _ => None,
            })
            .collect()
    }

    /// #1357: every effect row has its checkbox from the start, also for the effects not on the
    /// layer yet (an empty box), not only after one was turned on and off again.
    #[test]
    fn every_effect_row_shows_a_checkbox() {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        let l = Layer::raster("x", prism_doc::PixelFormat::RGBA8);
        let fields = initial_fields(&l, Some(BLENDING), 120.0);
        let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
            move |ui, app: &mut PrismApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                let mut f = fields.clone();
                body(app, ui, &mut f);
            },
            app,
        );
        PrismApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);
        let boxes: Vec<egui::Rect> = h
            .output()
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.rect.size() == vec2(CHECK, CHECK) && r.stroke.width > 0.0 => Some(r.rect),
                _ => None,
            })
            .collect();
        // The effect list is the leftmost column; the selected pane draws checkboxes of its own.
        let column = boxes.iter().map(|r| r.min.x).fold(f32::INFINITY, f32::min);
        let rows = boxes.iter().filter(|r| (r.min.x - column).abs() < 0.5).count();
        assert_eq!(rows, KINDS.len(), "one empty checkbox per effect row");
    }

    #[test]
    fn every_kind_has_spec_and_defaults() {
        for &(kind, _) in KINDS {
            assert!(!spec(kind).is_empty(), "{kind}");
            let d = defaults(kind);
            for &(key, _, p) in spec(kind) {
                let keys: Vec<&str> = match p {
                    P::Group => vec![],
                    P::Blend(Some(color)) => vec![key, color],
                    P::Angle(Some((check, _))) => vec![key, check],
                    P::Gradient => vec!["from", "to"],
                    _ => vec![key],
                };
                for key in keys {
                    assert!(d.get(key).is_some(), "{kind}.{key} has no default");
                }
            }
        }
        // Bevel's Contour and Texture pages edit bevel params.
        let d = defaults("bevelEmboss");
        for &(_, _, rows) in BEVEL_PARTS {
            for &(key, _, p) in spec(rows) {
                assert!(matches!(p, P::Group) || d.get(key).is_some(), "bevelEmboss.{key} has no default");
            }
        }
        // The page's new rows read the layer's own values back, so opening the dialog never resets
        // them to the factory value.
        for (kind, key, value) in [
            ("outerGlow", "technique", json!("precise")),
            ("innerGlow", "technique", json!("precise")),
            ("gradientOverlay", "dither", json!(true)),
            ("bevelEmboss", "technique", json!("chiselHard")),
            ("bevelEmboss", "glossContour", json!("Cone")),
        ] {
            let fx = prism_engine::layer_style::effect_from_params(kind, &json!({key: value.clone()})).unwrap();
            assert_eq!(values_of(&fx, 120.0)[key], value, "{kind}.{key}");
        }
    }

    #[test]
    fn initial_fields_lists_every_instance_and_selects() {
        let l = Layer::raster("x", prism_doc::PixelFormat::RGBA8);
        let f = initial_fields(&l, Some("stroke"), 120.0);
        assert_eq!(f["globalLight"], json!(120.0));
        // The kind isn't on the layer yet: selecting it creates the instance.
        assert_eq!(f["effects"].as_array().unwrap().len(), 1);
        assert_eq!(f["effects"][0]["kind"], json!("stroke"));
        assert_eq!(f["effects"][0]["on"], json!(true));
        assert_eq!(f["selected"], json!("fx1"));
        // Opening the blending page selects no instance.
        let f = initial_fields(&l, Some(BLENDING), 120.0);
        assert_eq!(f["selected"], json!(BLENDING));
        assert!(effects_of(&f).is_empty());
    }

    #[test]
    fn open_and_confirm_preserves_two_strokes() {
        // The manga/typesetting case: a 3 px black stroke under a 7 px white one.
        let mut s = session();
        s.execute("layer.layerStyle.stroke", json!({"size": 3, "color": "#000000"})).unwrap();
        s.execute("layer.layerStyle.stroke", json!({"size": 7, "color": "#ffffff", "add": true})).unwrap();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
        assert_eq!(effects_of(&f).len(), 2, "both instances load");
        // Confirm without touching anything: both strokes, same order and params.
        apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        let strokes = strokes_of(&s);
        assert_eq!(strokes.len(), 2);
        assert_eq!((strokes[0].size, strokes[1].size), (3.0, 7.0));
        assert_eq!(strokes[0].paint, FxPaint::Color(prism_color::Color::BLACK));
        assert_eq!(strokes[1].paint, FxPaint::Color(prism_color::Color::WHITE));
    }

    #[test]
    fn editing_one_instance_leaves_its_sibling_alone() {
        let mut s = session();
        s.execute("layer.layerStyle.stroke", json!({"size": 3, "color": "#000000"})).unwrap();
        s.execute("layer.layerStyle.stroke", json!({"size": 7, "color": "#ffffff", "add": true})).unwrap();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let mut f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
        set_param(&mut f, "fx1", "size", json!(5));
        apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        let strokes = strokes_of(&s);
        assert_eq!(strokes.len(), 2);
        assert_eq!(strokes[0].size, 5.0);
        assert_eq!(strokes[1].size, 7.0, "the second stroke is untouched");
        assert_eq!(strokes[1].paint, FxPaint::Color(prism_color::Color::WHITE));
    }

    #[test]
    fn add_and_remove_instances() {
        let mut s = session();
        s.execute("layer.layerStyle.stroke", json!({"size": 3, "color": "#000000"})).unwrap();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let mut f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
        add_instance(&mut f, "stroke");
        add_instance(&mut f, "stroke");
        assert_eq!(effects_of(&f).len(), 3);
        assert_eq!(f["selected"], json!("fx3"));
        remove_instance(&mut f, "fx2");
        assert_eq!(effects_of(&f).len(), 2);
        assert_eq!(f["selected"], json!("fx3"), "selection moves to a sibling");
        remove_instance(&mut f, "fx3");
        assert_eq!(f["selected"], json!("fx1"), "the last sibling takes over");
        apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        let strokes = strokes_of(&s);
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].size, 3.0);
    }

    #[test]
    fn disabled_instances_are_kept() {
        let mut s = session();
        s.execute("layer.layerStyle.colorOverlay", json!({"color": "#ff0000"})).unwrap();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let mut f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
        if let Some(e) = entry_mut(&mut f, "fx1") {
            e["on"] = json!(false);
        }
        apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        let d = s.active().unwrap();
        let items = &d.doc.layer(id).unwrap().effects.items;
        assert_eq!(items.len(), 1, "configured but switched off stays");
        assert!(!items[0].enabled());
    }

    #[test]
    fn gradient_strokes_survive_the_dialog() {
        let mut s = session();
        s.execute("layer.layerStyle.stroke", json!({"size": 4, "from": "#ff0000", "to": "#0000ff"})).unwrap();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
        // The dialog doesn't model the gradient, so no colour key is stored for it.
        assert!(effects_of(&f)[0]["params"].get("color").is_none());
        apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        let strokes = strokes_of(&s);
        assert_eq!(strokes.len(), 1);
        assert!(
            matches!(&strokes[0].paint, FxPaint::Gradient(g) if g.stops.len() == 2
                && g.stops[0].1 == prism_color::Color::rgb(1.0, 0.0, 0.0)
                && g.stops[1].1 == prism_color::Color::rgb(0.0, 0.0, 1.0)),
            "the gradient is carried, not reset",
        );
    }

    #[test]
    fn preview_applies_the_style_without_touching_the_document() {
        let s = session();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let mut f = initial_fields(st.doc.layer(id).unwrap(), Some("colorOverlay"), st.doc.global_light.angle);
        add_instance(&mut f, "stroke");
        let h = preview_hash(&f);
        f.insert("selected".into(), json!("fx2"));
        assert_eq!(preview_hash(&f), h, "switching pages doesn't re-render");
        set_param(&mut f, "fx2", "size", json!(5));
        assert_ne!(preview_hash(&f), h);
        let shown = preview_document(&st.doc, &s.patterns, &f).unwrap();
        let fx = |doc: &prism_doc::Document| doc.layer(id).unwrap().effects.items.len();
        assert_eq!((fx(&shown), fx(&st.doc)), (2, 0));
    }

    #[test]
    fn preview_rejects_non_object_effect_params() {
        let mut s = prism_engine::Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let st = s.active().unwrap();
        let mut f = initial_fields(st.doc.layer(st.active_layer.unwrap()).unwrap(), Some("colorOverlay"), st.doc.global_light.angle);
        let id = f["selected"].as_str().unwrap().to_string();
        entry_mut(&mut f, &id).unwrap()["params"] = json!("invalid");

        assert!(preview_document(&st.doc, &s.patterns, &f).is_err());
    }

    #[test]
    fn apply_rejects_non_object_params_before_running_commands() {
        let mut f = Map::new();
        f.insert("effects".into(), json!([{"id": "fx1", "kind": "stroke", "on": true, "params": [1, 2]}]));
        let mut calls = 0;

        let result = apply(&f, |_, _| {
            calls += 1;
            Ok(Value::Null)
        });

        assert!(result.is_err());
        assert_eq!(calls, 0, "invalid params must not partially apply the style");
    }

    #[test]
    fn body_keeps_invalid_params_and_renders_without_panicking() {
        let ctx = egui::Context::default();
        PrismApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        for bad in [json!(7), json!(null), json!("x")] {
            let mut fields = Map::new();
            fields.insert("selected".into(), json!("fx1"));
            fields.insert("effects".into(), json!([{"id": "fx1", "kind": "dropShadow", "on": true, "params": bad.clone()}]));

            let mut out = ctx.run_ui(Default::default(), |ui| {
                body(&mut app, ui, &mut fields);
            });
            out.textures_delta.clear();

            assert_eq!(entry(&fields, "fx1").unwrap()["params"], bad);
        }
        for bad in [json!(7), json!([1])] {
            let mut fields = Map::new();
            fields.insert("selected".into(), json!(BLENDING));
            fields.insert("p:blendingOptions".into(), bad.clone());
            fields.insert("effects".into(), bad.clone());
            let mut out = ctx.run_ui(Default::default(), |ui| {
                body(&mut app, ui, &mut fields);
            });
            out.textures_delta.clear();
            assert_eq!(fields.get("p:blendingOptions"), Some(&bad));
        }
    }

    #[test]
    fn blending_page_renders_for_every_mode_and_odd_fields() {
        let ctx = egui::Context::default();
        PrismApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        let mut layer = prism_doc::Layer::new("L", prism_doc::LayerContent::Adjustment(prism_doc::Adjustment::Invert));
        layer.advanced.knockout = prism_doc::Knockout::Deep;
        for mode in [prism_color::ColorMode::Rgb, prism_color::ColorMode::Cmyk, prism_color::ColorMode::Lab, prism_color::ColorMode::Grayscale] {
            let mut f = initial_fields(&layer, Some(BLENDING), 120.0);
            set_blending_fields(&mut f, &layer, mode);
            let mut out = ctx.run_ui(Default::default(), |ui| {
                body(&mut app, ui, &mut f);
            });
            out.textures_delta.clear();
            assert_eq!(f["p:blendingOptions"]["knockout"], "deep", "{mode:?}");
            // Hostile dialog state (only reachable through `ui.dialog.set`) still renders.
            let bo = f.get_mut("p:blendingOptions").unwrap();
            bo["blendIfChannel"] = json!(99);
            bo["channels"] = json!("x");
            bo["blendIf"] = json!([{"thisLayer": [300, -5, "a"]}]);
            f.insert("channelNames".into(), json!([]));
            let mut out = ctx.run_ui(Default::default(), |ui| {
                body(&mut app, ui, &mut f);
            });
            out.textures_delta.clear();
        }
    }

    #[test]
    fn confirm_rejects_invalid_params_and_applies_valid_effects() {
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let layer = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().clone();
        let mut invalid = initial_fields(&layer, Some("colorOverlay"), 0.0);
        let id = invalid["selected"].as_str().unwrap().to_string();
        entry_mut(&mut invalid, &id).unwrap()["params"] = json!(null);
        assert!(confirm(&mut app, &invalid).is_err());
        assert!(app.session.active().unwrap().doc.layer(layer.id).unwrap().effects.items.is_empty());

        let valid = initial_fields(&layer, Some("colorOverlay"), 0.0);
        confirm(&mut app, &valid).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layer(layer.id).unwrap().effects.items.len(), 1);
    }

    #[test]
    fn blending_page_round_trips_advanced_blending_and_blend_if() {
        use prism_doc::{BlendRange, Knockout};
        for mode in ["rgb", "cmyk", "grayscale"] {
            let mut s = prism_engine::Session::new();
            s.execute("file.new", json!({"width": 16, "height": 16, "mode": mode})).unwrap();
            s.execute("layer.new.layer", json!({})).unwrap();
            let st = s.active().unwrap();
            let id = st.active_layer.unwrap();
            let mut l = st.doc.layer(id).unwrap().clone();
            l.advanced.knockout = Knockout::Shallow;
            l.advanced.blend_clipped = false;
            l.excluded_channels = 0b1;
            let gray = if st.doc.mode.color_channels() == 1 { 1 } else { 0 };
            l.blend_if.set(gray, [BlendRange { black: [10, 30], white: [255, 255] }, BlendRange::FULL]);
            let mut f = initial_fields(&l, None, 120.0);
            set_blending_fields(&mut f, &l, st.doc.mode);
            let names = channel_names(st.doc.mode);
            assert_eq!(f["channelNames"], json!(names), "{mode}");
            let p = &f[&format!("p:{BLENDING}")];
            assert_eq!(p["knockout"], "shallow");
            assert_eq!(p["blendClippedLayersAsGroup"], false);
            assert_eq!(p["transparencyShapesLayer"], true);
            assert_eq!(p["channels"].as_array().unwrap().len(), names.len());
            assert_eq!(p["channels"][0], false);
            assert_eq!(p["blendIf"][0]["thisLayer"], json!([10, 30, 255, 255]), "{mode}");
            // Applying the page sets every switch through the engine command.
            let mut f2 = f.clone();
            let bo = f2.get_mut(&format!("p:{BLENDING}")).unwrap();
            bo["knockout"] = json!("deep");
            bo["vectorMaskHidesEffects"] = json!(true);
            bo["blendIfChannel"] = json!(names.len().min(1));
            let shown = preview_document(&st.doc, &s.patterns, &f2).unwrap();
            let got = shown.layer(id).unwrap();
            assert_eq!(got.advanced.knockout, Knockout::Deep, "{mode}");
            assert!(got.advanced.vector_mask_hides_effects);
            assert!(!got.advanced.blend_clipped);
            assert_eq!(got.excluded_channels, 0b1);
            assert_eq!(got.blend_if.get(gray)[0], BlendRange { black: [10, 30], white: [255, 255] }, "{mode}");
        }
    }

    #[test]
    fn blend_if_values_stay_ordered() {
        assert_eq!(blend_quad(Some(&json!([200, 10, 5, 300]))), [200, 200, 200, 255]);
        assert_eq!(blend_quad(None), [0, 0, 255, 255]);
        assert_eq!(blend_quad(Some(&json!(["x", null]))), [0, 0, 255, 255]);
    }

    #[test]
    fn percent_fields_round_trip_through_the_engine() {
        let mut s = session();
        s.execute("layer.layerStyle.outerGlow", json!({"spread": 6, "range": 40})).unwrap();
        s.execute("layer.layerStyle.dropShadow", json!({"spread": 12, "add": true})).unwrap();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
        let fx = effects_of(&f);
        assert_eq!(fx.len(), 2);
        assert_eq!(fx[0]["params"]["spread"], json!(6.0));
        assert_eq!(fx[0]["params"]["range"], json!(40.0));
        assert_eq!(fx[1]["params"]["spread"], json!(12.0));
    }

    #[test]
    fn drop_shadow_angle_reaches_the_effect() {
        // #350: the Angle slider was dead — the dialog never sent `useGlobalLight`, so the engine
        // defaulted it to true and the compositor used the fixed global light angle, ignoring the slider.
        let mut s = session();
        let shadow = |s: &prism_engine::Session| {
            let d = s.active().unwrap();
            let id = d.active_layer.unwrap();
            d.doc.layer(id).unwrap().effects.items.iter().find_map(|e| match e {
                Effect::DropShadow(sh) => Some(sh.clone()),
                _ => None,
            })
        };

        // Use Global Light off: the per-effect angle is stored and used.
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let mut f = initial_fields(st.doc.layer(id).unwrap(), Some("dropShadow"), st.doc.global_light.angle);
        set_param(&mut f, "fx1", "angle", json!(45.0));
        set_param(&mut f, "fx1", "useGlobalLight", json!(false));
        apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        let eff = shadow(&s).unwrap();
        assert!(!eff.use_global_light, "Use Global Light off keeps the per-effect angle");
        assert_eq!(eff.angle, 45.0);

        // Use Global Light on: the Angle slider drives the document's shared light angle.
        let st = s.active().unwrap();
        let mut f = initial_fields(st.doc.layer(id).unwrap(), Some("dropShadow"), st.doc.global_light.angle);
        set_param(&mut f, "fx1", "useGlobalLight", json!(true));
        set_param(&mut f, "fx1", "angle", json!(30.0));
        apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        assert_eq!(s.active().unwrap().doc.global_light.angle, 30.0, "the Angle slider moves the shared light");
        assert!(shadow(&s).unwrap().use_global_light);

        // Reopening the dialog shows the effective angle and the checkbox state.
        let st = s.active().unwrap();
        let f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
        let fx = effects_of(&f);
        let shadow_entry = fx.iter().find(|e| e["kind"] == json!("dropShadow")).unwrap();
        assert_eq!(shadow_entry["params"]["useGlobalLight"], json!(true));
        assert_eq!(shadow_entry["params"]["angle"], json!(30.0));
    }

    #[test]
    fn bevel_angle_change_is_not_overridden_by_unchanged_drop_shadow_angle() {
        let mut s = session();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let mut f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
        add_instance(&mut f, "bevelEmboss");
        set_param(&mut f, "fx1", "angle", json!(45.0));
        add_instance(&mut f, "dropShadow");
        apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        assert_eq!(s.active().unwrap().doc.global_light.angle, 45.0);
        let d = s.active().unwrap();
        assert_eq!(d.doc.layer(id).unwrap().effects.items.len(), 2);
    }

    #[test]
    fn unchanged_global_light_angle_is_not_applied() {
        let layer = Layer::raster("x", prism_doc::PixelFormat::RGBA8);
        let mut f = initial_fields(&layer, None, 47.5);
        add_instance(&mut f, "bevelEmboss");
        assert_eq!(effects_of(&f)[0]["params"]["angle"].as_f64(), Some(47.5), "a fresh effect starts from the shared light",);
        let mut commands = Vec::new();
        apply(&f, |cmd, _| {
            commands.push(cmd.to_string());
            Ok(Value::Null)
        })
        .unwrap();
        assert!(!commands.iter().any(|cmd| cmd == "layer.layerStyle.globalLight"));
    }

    #[test]
    fn last_angle_edit_wins_across_global_light_instances_and_preview() {
        let mut s = session();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let original = st.doc.clone();
        let mut f = initial_fields(st.doc.layer(id).unwrap(), Some("dropShadow"), st.doc.global_light.angle);
        add_instance(&mut f, "innerShadow");
        add_instance(&mut f, "bevelEmboss");
        set_param(&mut f, "fx2", "angle", json!(30.0));
        set_param(&mut f, "fx1", "angle", json!(-90.0));
        let preview = preview_document(&original, &s.patterns, &f).unwrap();
        assert_eq!(preview.global_light.angle, -90.0, "a later instance must not override the edited shadow");
        for e in effects_of(&f) {
            assert_eq!(e["params"]["angle"], json!(-90.0));
        }
        assert_eq!(s.active().unwrap().doc.global_light.angle, original.global_light.angle, "preview does not mutate the document");
        // Adding an effect and disabling the edited effect must preserve the pending light.
        add_instance(&mut f, "dropShadow");
        assert_eq!(effects_of(&f)[3]["params"]["angle"], json!(-90.0));
        entry_mut(&mut f, "fx1").unwrap()["on"] = json!(false);
        apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        assert_eq!(s.active().unwrap().doc.global_light.angle, -90.0);
        // Moving back to the opening angle also refreshes the preview.
        let before = preview_hash(&f);
        set_param(&mut f, "fx2", "angle", json!(original.global_light.angle));
        assert_ne!(preview_hash(&f), before);
        assert_eq!(preview_document(&original, &s.patterns, &f).unwrap().global_light.angle, original.global_light.angle);
    }

    #[test]
    fn local_angles_stay_independent_and_enabling_global_light_uses_shared_angle() {
        let layer = Layer::raster("x", prism_doc::PixelFormat::RGBA8);
        let mut f = initial_fields(&layer, Some("dropShadow"), 120.0);
        add_instance(&mut f, "innerShadow");
        set_param(&mut f, "fx2", "useGlobalLight", json!(false));
        set_param(&mut f, "fx2", "angle", json!(45.0));
        set_param(&mut f, "fx1", "angle", json!(60.0));
        assert_eq!(entry(&f, "fx2").unwrap()["params"]["angle"], json!(45.0));
        set_param(&mut f, "fx2", "useGlobalLight", json!(true));
        assert_eq!(entry(&f, "fx2").unwrap()["params"]["angle"], json!(60.0));
    }

    #[test]
    fn preview_moves_shadow_pixels_when_shared_angle_changes() {
        let mut s = session();
        s.execute("select.rect", json!({"x": 20, "y": 20, "width": 16, "height": 16})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let st = s.active().unwrap();
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
        let mut f = initial_fields(layer, Some("dropShadow"), st.doc.global_light.angle);
        set_param(&mut f, "fx1", "distance", json!(8.0));
        set_param(&mut f, "fx1", "size", json!(0.0));
        set_param(&mut f, "fx1", "opacity", json!(100.0));
        let pixel = |f: &Map<String, Value>, x, y| {
            let preview = preview_document(&st.doc, &s.patterns, f).unwrap();
            prism_compose::flatten(&preview).get(x, y)
        };
        set_param(&mut f, "fx1", "angle", json!(0.0));
        assert!(pixel(&f, 14, 28)[0] < 0.01, "light from right casts shadow left");
        set_param(&mut f, "fx1", "angle", json!(90.0));
        assert!(pixel(&f, 14, 28)[0] > 0.99, "the old shadow disappears");
        assert!(pixel(&f, 28, 40)[0] < 0.01, "light from above casts shadow below");
    }

    #[test]
    fn angle_dial_clicks_follow_light_direction() {
        use egui_kittest::kittest::Queryable;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(300.0, 100.0)).build_ui_state(
            |ui, angle| {
                angle_row(ui, "Angle", angle, None);
            },
            120.0_f32,
        );
        h.run();
        let dial = h.query_all_by_label("Angle").last().unwrap();
        let rect = dial.rect();
        for (delta, expected) in [(vec2(15.0, 0.0), 0.0), (vec2(0.0, -15.0), 90.0), (vec2(0.0, 15.0), -90.0)] {
            let pos = rect.center() + delta;
            h.event(egui::Event::PointerMoved(pos));
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
            h.step();
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            h.run();
            assert_eq!(*h.state(), expected);
        }
    }

    #[test]
    fn preview_off_shows_the_original_until_reenabled() {
        let mut s = session();
        s.execute("layer.layerStyle.colorOverlay", json!({"color": "#ff0000"})).unwrap();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let mut f = initial_fields(st.doc.layer(id).unwrap(), Some("colorOverlay"), st.doc.global_light.angle);
        // Preview off is the dialog default's complement; edits still change the state.
        f.insert("preview".into(), json!(false));
        let h_off = preview_hash(&f);
        set_param(&mut f, "fx1", "color", json!("#00ff00"));
        assert_ne!(preview_hash(&f), h_off, "editing while preview is off still updates the dialog state");
        // Re-enabling preview applies the pending state (the canvas reads `preview`; the
        // scratch-session apply is the same one the checkbox-on path uses).
        f.insert("preview".into(), json!(true));
        let shown = preview_document(&st.doc, &s.patterns, &f).unwrap();
        let d = s.active().unwrap();
        let color = |doc: &prism_doc::Document| match &doc.layer(id).unwrap().effects.items[0] {
            Effect::ColorOverlay { color, .. } => *color,
            other => panic!("{other:?}"),
        };
        assert_eq!(color(&shown), prism_color::Color::rgb(0.0, 1.0, 0.0));
        assert_ne!(color(&d.doc), color(&shown), "the document itself is untouched until OK");
        // OK commits regardless of the checkbox.
        f.insert("preview".into(), json!(false));
        apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        let d = s.active().unwrap();
        assert_eq!(color(&d.doc), prism_color::Color::rgb(0.0, 1.0, 0.0), "OK commits with Preview off");
    }

    #[test]
    fn new_style_saves_pending_state_and_a_swatch_apply_restores_it() {
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let id = app.session.active().unwrap().active_layer.unwrap();
        let mut f = initial_fields(app.session.active().unwrap().doc.layer(id).unwrap(), Some("stroke"), 120.0);
        set_param(&mut f, "fx1", "size", json!(9));
        save_new_style(&mut app, &f);
        let preset = app.session.presets.styles.iter().flat_map(|g| g.items.iter()).find(|st| st.name == "Style").expect("the preset is created");
        assert_eq!(preset.effects.len(), 1, "only the enabled instance is saved");
        // A second layer gets the pending style back through the swatch path.
        app.run("layer.new.layer", json!({})).unwrap();
        let mut f2 = initial_fields(app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap(), None, 120.0);
        f2.insert("layer".into(), json!(app.session.active().unwrap().active_layer.unwrap().0));
        apply_style_preset(&app, &mut f2, "Style");
        // Swatch clicks update the dialog fields only; document is untouched until confirm.
        assert_eq!(effects_of(&f2).len(), 1);
        assert_eq!(effects_of(&f2)[0]["params"]["size"], json!(9.0));
        // Confirming applies the style to the layer.
        confirm(&mut app, &f2).unwrap();
        let st = app.session.active().unwrap();
        let strokes: Vec<f32> = st
            .doc
            .layer(app.session.active().unwrap().active_layer.unwrap())
            .unwrap()
            .effects
            .items
            .iter()
            .filter_map(|e| match e {
                Effect::Stroke(s) => Some(s.size),
                _ => None,
            })
            .collect();
        assert_eq!(strokes, vec![9.0], "confirming commits the style preset to the layer");
    }

    #[test]
    fn style_click_does_not_mutate_document_until_confirm() {
        let mut app = PrismApp::new(prism_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let id = app.session.active().unwrap().active_layer.unwrap();
        let mut f = initial_fields(app.session.active().unwrap().doc.layer(id).unwrap(), Some(STYLES_PAGE), 120.0);
        apply_style_preset(&app, &mut f, "Black Stroke");
        // Document itself still has 0 effects.
        assert_eq!(app.session.active().unwrap().doc.layer(id).unwrap().effects.items.len(), 0);
        // Dialog fields now have the preset loaded.
        assert_eq!(effects_of(&f).len(), 1);
        // Cancel discards dialog; document remains with 0 effects.
        // Confirm commits the style.
        confirm(&mut app, &f).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layer(id).unwrap().effects.items.len(), 1);
    }

    /// The up and down arrows swap an instance with its neighbour of the same kind only.
    #[test]
    fn move_arrows_reorder_instances_of_one_kind() {
        let mut s = session();
        s.execute("layer.layerStyle.stroke", json!({"size": 3})).unwrap();
        s.execute("layer.layerStyle.dropShadow", json!({})).unwrap();
        s.execute("layer.layerStyle.stroke", json!({"size": 7, "add": true})).unwrap();
        let st = s.active().unwrap();
        let mut f = initial_fields(st.doc.layer(st.active_layer.unwrap()).unwrap(), None, 120.0);
        let ids = |f: &Map<String, Value>| effects_of(f).iter().map(|e| e["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        let strokes: Vec<String> = effects_of(&f).iter().filter(|e| e["kind"] == "stroke").map(|e| e["id"].as_str().unwrap().to_string()).collect();
        assert_eq!(strokes.len(), 2);
        let (first, second) = (strokes[0].clone(), strokes[1].clone());
        assert!(sibling(&f, &first, true).is_none(), "the top stroke can't move up");
        let shadow = effects_of(&f).iter().find(|e| e["kind"] == "dropShadow").unwrap()["id"].as_str().unwrap().to_string();
        assert!(sibling(&f, &shadow, true).is_none() && sibling(&f, &shadow, false).is_none(), "a lone instance has nowhere to go");
        let before = ids(&f);
        move_instance(&mut f, &first, false);
        let after = ids(&f);
        assert_ne!(before, after);
        let pos = |v: &[String], id: &str| v.iter().position(|x| x == id).unwrap();
        assert!(pos(&after, &second) < pos(&after, &first), "the two strokes swapped");
        assert_eq!(pos(&after, &shadow), pos(&before, &shadow), "other kinds stay put");
        // A move of an unknown id changes nothing.
        move_instance(&mut f, "nope", true);
        assert_eq!(ids(&f), after);
        // The order reaches the layer on OK.
        apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        assert_eq!(strokes_of(&s).iter().map(|st| st.size).collect::<Vec<_>>(), vec![7.0, 3.0]);
    }

    /// Photoshop's fx menu and a click on an effect's name both show the page with the effect on.
    #[test]
    fn showing_a_kind_turns_its_first_instance_on() {
        let mut s = session();
        s.execute("layer.layerStyle.satin", json!({})).unwrap();
        let st = s.active().unwrap();
        let mut f = initial_fields(st.doc.layer(st.active_layer.unwrap()).unwrap(), Some(BLENDING), 120.0);
        let satin = effects_of(&f)[0]["id"].as_str().unwrap().to_string();
        entry_mut(&mut f, &satin).unwrap()["on"] = json!(false);
        show_kind(&mut f, "satin");
        assert_eq!(f["selected"], json!(satin));
        assert_eq!(entry(&f, &satin).unwrap()["on"], json!(true));
        // A kind not on the layer gets a new instance.
        show_kind(&mut f, "outerGlow");
        assert_eq!(effects_of(&f).len(), 2);
        assert_eq!(entry(&f, f["selected"].as_str().unwrap()).unwrap()["kind"], json!("outerGlow"));
    }

    /// The Name field renames the layer on OK; an unchanged or blank name leaves it alone.
    #[test]
    fn name_field_renames_the_layer_on_ok() {
        let mut s = session();
        let id = s.active().unwrap().active_layer.unwrap();
        let name = |s: &prism_engine::Session| s.active().unwrap().doc.layer(id).unwrap().name.clone();
        let original = name(&s);
        let mut f = initial_fields(s.active().unwrap().doc.layer(id).unwrap(), Some(BLENDING), 120.0);
        assert_eq!(f["name"], json!(original));
        f.insert("name".into(), json!("   "));
        apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        assert_eq!(name(&s), original, "a blank name is ignored");
        f.insert("name".into(), json!("  Title  "));
        apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        assert_eq!(name(&s), "Title");
    }

    /// The swatch beside the buttons shows the pending (enabled) instances, not the layer's.
    #[test]
    fn pending_style_has_the_enabled_instances_with_their_edits() {
        let l = Layer::raster("x", prism_doc::PixelFormat::RGBA8);
        let mut f = initial_fields(&l, Some("stroke"), 120.0);
        set_param(&mut f, "fx1", "size", json!(11));
        add_instance(&mut f, "dropShadow");
        let shadow = f["selected"].as_str().unwrap().to_string();
        entry_mut(&mut f, &shadow).unwrap()["on"] = json!(false);
        let st = pending_style(&f);
        assert_eq!(st.effects.len(), 1, "the disabled shadow is left out");
        assert!(matches!(&st.effects[0], Effect::Stroke(s) if s.size == 11.0));
        // Odd fields never panic: a non-object params entry is skipped as empty params.
        f.insert("effects".into(), json!([{"id": "fx9", "kind": "satin", "on": true, "params": 3}, {"on": true}, 7]));
        assert_eq!(pending_style(&f).effects.len(), 1);
    }

    /// Bevel & Emboss › Contour and Texture: their boxes turn the elements on and off, their pages
    /// edit the bevel, and OK puts them on the layer.
    #[test]
    fn bevel_contour_and_texture_rows_reach_the_layer() {
        let mut s = session();
        s.execute("layer.layerStyle.bevelEmboss", json!({})).unwrap();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let mut f = initial_fields(st.doc.layer(id).unwrap(), Some("bevelEmboss"), 120.0);
        let bevel = f["selected"].as_str().unwrap().to_string();
        assert!(!part_on(&f, &bevel, "contour") && !part_on(&f, &bevel, "texture"));
        assert_eq!(entry(&f, &bevel).unwrap()["params"]["texture"], json!(false), "the layer's texture is read back as off");
        // Without patterns Texture can't turn on; with one it takes the first.
        set_part(&mut f, &bevel, "texture", true);
        assert!(!part_on(&f, &bevel, "texture"));
        f.insert("patternList".into(), json!([["Bubbles", "Bubbles"]]));
        set_part(&mut f, &bevel, "texture", true);
        set_part(&mut f, &bevel, "contour", true);
        set_param(&mut f, &bevel, "contourRange", json!(80));
        set_param(&mut f, &bevel, "textureDepth", json!(-150));
        assert!(part_on(&f, &bevel, "contour") && part_on(&f, &bevel, "texture"));
        apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        let layer = s.active().unwrap().doc.layer(id).unwrap().clone();
        let Effect::BevelEmboss(b) = &layer.effects.items[0] else { panic!() };
        assert_eq!(b.contour.as_ref().map(|c| c.range), Some(0.8));
        assert_eq!(b.texture.as_ref().map(|t| (t.id.as_str(), t.depth)), Some(("Bubbles", -1.5)));
        // Reopened, the pages show what the layer has; unticking Texture removes it.
        let mut f = initial_fields(&layer, Some("bevelEmboss"), 120.0);
        let bevel = f["selected"].as_str().unwrap().to_string();
        assert_eq!(entry(&f, &bevel).unwrap()["params"]["contourRange"], json!(80.0));
        assert_eq!(entry(&f, &bevel).unwrap()["params"]["texture"], json!("Bubbles"));
        set_part(&mut f, &bevel, "texture", false);
        apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        let Effect::BevelEmboss(b) = &s.active().unwrap().doc.layer(id).unwrap().effects.items[0] else { panic!() };
        assert!(b.texture.is_none() && b.contour.is_some());
    }

    /// A part page id names its bevel; deleting that bevel leaves no dangling page selected, and
    /// the page renders for odd ids without panicking.
    #[test]
    fn part_pages_follow_their_bevel() {
        assert_eq!(split_part("fx2#contour"), ("fx2", Some("contour")));
        assert_eq!(split_part("fx2#nope"), ("fx2#nope", None));
        assert_eq!(split_part("dropShadow"), ("dropShadow", None));
        let l = Layer::raster("x", prism_doc::PixelFormat::RGBA8);
        let mut f = initial_fields(&l, Some("bevelEmboss"), 120.0);
        let bevel = f["selected"].as_str().unwrap().to_string();
        f.insert("selected".into(), json!(format!("{bevel}#texture")));
        remove_instance(&mut f, &bevel);
        assert_eq!(f["selected"], json!(STYLES_PAGE));
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        PrismApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        for sel in ["fx9#contour", "fx1#texture", "#", "fx1#"] {
            let mut f = initial_fields(&l, Some("bevelEmboss"), 120.0);
            f.insert("selected".into(), json!(sel));
            let mut out = ctx.run_ui(Default::default(), |ui| {
                body(&mut app, ui, &mut f);
            });
            out.textures_delta.clear();
        }
    }
}
