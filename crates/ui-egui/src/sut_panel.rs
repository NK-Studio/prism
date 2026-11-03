//! CSP presentation metadata never participates in brush rendering.
use egui::{Color32, Sense, vec2};
use prism_engine::{BrushSettings, paint::sut::SutBrushMetadata};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{PrismApp, brush_panel, brush_preview, icons, theme::Tokens};

/// Shared view preferences, separate from each preset's exported visibility.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SutPanelState {
    pub details: bool,
    pub texture_scale_expanded: bool,
}

// Canonical order observed in CSP; ShowParam is membership, not a reorder instruction.
const ITEMS: &[(u32, &str, &str, &str)] = &[
    (1000, "Brush Size", "BrushSize", "Brush Size"),
    (1002, "Specify by size on screen", "BrushSizeSyncViewScale", "Brush Size"),
    (1003, "At least 1 pixel", "BrushAtLeast1Pixel", "Brush Size"),
    (10, "Opacity", "Opacity", "Ink"),
    (30, "Blending mode", "CompositeMode", "Ink"),
    (1020, "Brush density", "BrushFlow", "Ink"),
    (1081, "Amount of paint", "BrushMixColor", "Ink"),
    (1083, "Density of paint", "BrushMixAlpha", "Ink"),
    (1085, "Color stretch", "BrushMixColorExtension", "Ink"),
    (20, "Anti-aliasing", "AntiAlias", "Anti-aliasing"),
    (1030, "Hardness", "BrushHardness", "Brush tip"),
    (1050, "Thickness", "BrushThickness", "Brush tip"),
    (1052, "Thickness direction", "BrushVerticalThicknes", "Brush tip"),
    (1060, "Direction", "BrushRotation", "Brush tip"),
    (1071, "Tip order", "BrushPatternOrderType", "Brush tip"),
    (1074, "Flip horizontal", "BrushPatternReverseHorizontal", "Brush tip"),
    (1075, "Flip vertical", "BrushPatternReverseVertical", "Brush tip"),
    (1110, "Ribbon", "BrushRibbon", "Brush tip"),
    (1040, "Spacing", "BrushInterval", "Brush tip"),
    (1042, "Spacing type", "BrushAutoIntervalType", "Brush tip"),
    (1130, "Spray effect", "BrushUseSpray", "Spray"),
    (1131, "Particle size", "BrushSpraySize", "Spray"),
    (1133, "Particle density", "BrushSprayDensity", "Spray"),
    (1135, "Spray deviation", "BrushSprayBias", "Spray"),
    (1061, "Direction of particle", "BrushRotationInSpray", "Spray"),
    (1043, "Continuous spraying", "BrushContinuousPlot", "Spray"),
    (1102, "Change brush tip color", "BrushChangePatternColor", "Color jitter"),
    (1103, "Hue", "BrushHueChange", "Color jitter"),
    (1105, "Saturation", "BrushSaturationChange", "Color jitter"),
    (1107, "Luminosity", "BrushValueChange", "Color jitter"),
    (1270, "Change color per stroke", "BrushChangeStrokeColor", "Color jitter"),
    (1080, "Color mixing", "BrushWaterColor", "Ink"),
    (1162, "Starting and ending: effect", "BrushInOutTarget", "Starting and ending"),
    (1171, "Starting", "BrushInLength", "Starting and ending"),
    (1181, "Ending", "BrushOutLength", "Starting and ending"),
    (1910, "Dual brush", "UseDualBrush", "Dual brush"),
    (51000, "Dual brush size", "DualSize", "Dual brush"),
    (100, "Texture", "TextureImage", "Texture"),
    (110, "Texture density", "TextureDensity", "Texture"),
    (120, "Scale ratio", "TextureScale2", "Texture"),
    (1201, "Watercolor edge", "BrushWaterEdgeRadius", "Watercolor edge"),
    (1202, "Opacity influence", "BrushWaterEdgeAlphaPower", "Watercolor edge"),
    (1203, "Darkness influence", "BrushWaterEdgeValuePower", "Watercolor edge"),
    (40, "Stabilization", "FlickerReduction", "Correction"),
    (41, "Adjust by speed", "FlickerReductionBySpeed", "Correction"),
    (42, "Speed adjustment type", "FlickerReductionBySpeedType", "Correction"),
    (50, "Stickiness", "Stickness", "Correction"),
    (1151, "Post correction", "BrushRevision", "Correction"),
    (1250, "Refer to other layers", "BrushUseReferLayer", "Correction"),
    (1241, "Vector magnet", "BrushVectorMagnetPower", "Correction"),
    (1500, "Adjust position", "StampAdjustPosition", "Copy stamp"),
];

pub fn active(app: &PrismApp) -> Option<(&str, &SutBrushMetadata)> {
    let name = app.session.tools.brush_preset.as_deref()?;
    let preset = app.session.tools.presets.iter().find(|p| p.name.eq_ignore_ascii_case(name))?;
    Some((&preset.name, preset.sut.as_ref()?))
}

fn source(m: &SutBrushMetadata, key: &str) -> f32 {
    m.parameters.get(key).copied().filter(|n| n.is_finite()).unwrap_or(0.0).clamp(-1e6, 1e6) as f32
}

fn available(m: &SutBrushMetadata, id: u32, key: &str) -> bool {
    m.original.visible.contains(&id)
        || m.effective().visible.contains(&id)
        || (m.kind().is_verified_brush() && (m.parameters.contains_key(key) || (id == 100 && m.parameters.contains_key("TextureDensity"))))
}

/// A two-line row leaves room for translated names and explicit units in narrow panels.
fn number(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str) -> bool {
    let label = ui.label(tl!(label));
    ui.add(egui::Slider::new(value, range).suffix(unit)).labelled_by(label.id).changed()
}

fn unsupported(ui: &mut egui::Ui, label: &str, value: String, conditional: bool) {
    ui.add_enabled_ui(false, |ui| {
        ui.label(tl!(label));
        ui.label(value);
    });
    ui.small(if conditional { tl!("Disabled by the imported CSP settings") } else { tl!("This CSP effect is not supported in Prism yet") });
}

fn control(ui: &mut egui::Ui, id: u32, label: &str, key: &str, m: &SutBrushMetadata, b: &mut BrushSettings, details: bool) {
    match id {
        1000 => {
            number(ui, label, &mut b.size, 0.5..=prism_engine::paint::MAX_BRUSH_SIZE, " px");
        }
        10 => {
            let mut percent = b.opacity * 100.0;
            if number(ui, label, &mut percent, 0.0..=100.0, " %") {
                b.opacity = percent / 100.0;
            }
        }
        30 => {
            // The imported CSP code decides (prism_paint::csp_blend): an erasing or CSP-only mode
            // is shown by its CSP name; a mapped one edits the brush's blend mode.
            use prism_engine::paint::csp_blend::{CspBlend, csp_blend};
            match csp_blend(f64::from(source(m, key))) {
                Some((_, CspBlend::Mode { .. })) => {
                    ui.label(tl!(label));
                    let opts: Vec<_> = prism_color::BlendMode::LAYER_MODES.iter().copied().map(|mode| (mode, mode.label())).collect();
                    crate::widgets::dropdown(ui, "sut-blend-mode", &mut b.mode, &opts, ui.available_width().min(250.0));
                }
                Some((name, CspBlend::Erase { .. } | CspBlend::Stroke(_))) => {
                    ui.horizontal(|ui| {
                        ui.label(tl!(label));
                        ui.label(tl!(name));
                    });
                }
                Some((name, CspBlend::Unsupported)) => unsupported(ui, label, tl!(name).to_string(), false),
                None => unsupported(ui, label, format!("CSP {}", source(m, key)), false),
            }
        }
        20 => {
            let mut level = b.csp_anti_alias.unwrap_or(if b.aliased { 0 } else { 1 });
            let opts = [(0u8, "None"), (1, "Weak"), (2, "Medium"), (3, "Strong")];
            ui.horizontal(|ui| {
                ui.label(tl!(label));
                crate::widgets::dropdown(ui, "sut-anti-alias", &mut level, &opts, ui.available_width().min(160.0));
            });
            if b.csp_anti_alias != Some(level) {
                b.csp_anti_alias = Some(level);
            }
        }
        1050 => {
            let vertical = thickness_vertical(m, b);
            let [x, y] = b.csp_tip_scale.unwrap_or([1.0, 1.0]);
            let mut percent = if vertical { y } else { x } * 100.0;
            if number(ui, label, &mut percent, 1.0..=1000.0, " %") {
                b.csp_tip_scale = Some(if vertical { [x, percent / 100.0] } else { [percent / 100.0, y] });
            }
        }
        1040 | 1042 => spacing(ui, id, label, m, b),
        1002 => {
            ui.checkbox(&mut b.csp_size_by_view, tl!(label));
        }
        1003 => {
            ui.checkbox(&mut b.csp_at_least_1px, tl!(label));
        }
        1074 => {
            ui.checkbox(&mut b.flip_x, tl!(label));
        }
        1075 => {
            ui.checkbox(&mut b.flip_y, tl!(label));
        }
        1052 => {
            // The thickness squashes the tip's width (horizontal) or its height (vertical).
            let vertical = thickness_vertical(m, b);
            let mut picked = vertical;
            ui.horizontal(|ui| {
                ui.label(tl!(label));
                crate::widgets::dropdown(ui, "sut-thickness-axis", &mut picked, &[(false, "Horizontal"), (true, "Vertical")], ui.available_width().min(160.0));
            });
            if picked != vertical {
                let [x, y] = b.csp_tip_scale.unwrap_or([1.0, 1.0]);
                let thickness = if vertical { y } else { x };
                b.csp_tip_scale = Some(if picked { [1.0, thickness] } else { [thickness, 1.0] });
            }
        }
        1071 => {
            use prism_engine::paint::csp_tip::CspTipOrder;
            if matches!(&b.tip, prism_engine::paint::TipShape::Sequence(tips) if tips.len() > 1) {
                let mut order = b.csp_tip_order.unwrap_or_default();
                let opts =
                    [(CspTipOrder::Repeat, "Repeat"), (CspTipOrder::Reverse, "Reverse"), (CspTipOrder::Once, "Do not repeat"), (CspTipOrder::Random, "Random")];
                ui.horizontal(|ui| {
                    ui.label(tl!(label));
                    crate::widgets::dropdown(ui, "sut-tip-order", &mut order, &opts, ui.available_width().min(180.0));
                });
                // Repeat is the renderer's default: stored as `None`, as the importer does.
                let next = (order != CspTipOrder::Repeat).then_some(order);
                if b.csp_tip_order != next {
                    b.csp_tip_order = next;
                }
            } else {
                unsupported(ui, label, tl!("One tip").to_string(), true);
            }
        }
        1043 => {
            ui.checkbox(&mut b.build_up, tl!(label));
            ui.small(tl!("Sprays while the pen holds still; CSP's rate is not traced"));
        }
        50 => match &mut b.csp_stabilizer {
            Some(st) => {
                let mut v = st.stickiness.min(prism_engine::paint::csp_stabilizer::MAX_LEVEL) as f32;
                if number(ui, label, &mut v, 0.0..=prism_engine::paint::csp_stabilizer::MAX_LEVEL as f32, "") {
                    st.stickiness = (v.round().max(0.0) as u32).min(prism_engine::paint::csp_stabilizer::MAX_LEVEL);
                }
            }
            None => unsupported(ui, label, format!("{:.0}", source(m, key)), true),
        },
        1202 | 1203 => match &mut b.csp_water_edge {
            Some(e) => {
                let value = if id == 1202 { &mut e.opacity } else { &mut e.darkness };
                let mut percent = if value.is_finite() { (*value * 100.0).clamp(0.0, 100.0) } else { 0.0 };
                if number(ui, label, &mut percent, 0.0..=100.0, " %") {
                    *value = (percent / 100.0).clamp(0.0, 1.0);
                }
            }
            None => unsupported(ui, label, format!("{:.0}", source(m, key)), true),
        },
        1270 => {
            // Once per stroke (`per_tip: false`); per-tip variation (1102) and per-stroke together
            // use the per-tip one, as on import.
            let cd = &mut b.color_dynamics;
            let mut on = cd.enabled && !cd.per_tip;
            if ui.checkbox(&mut on, tl!(label)).changed() {
                cd.enabled = on;
                cd.per_tip = !on;
            }
            if cd.enabled && cd.per_tip {
                ui.small(tl!("Change brush tip color is on: the colour varies per tip"));
            }
        }
        1080 => {
            let mut on = b.csp_mixing.is_some();
            if ui.checkbox(&mut on, tl!(label)).changed() {
                b.csp_mixing = on.then(prism_engine::paint::CspColorMixing::default);
            }
            if let Some(mixing) = &mut b.csp_mixing {
                ui.checkbox(&mut mixing.perceptual, tl!("Perceptual color mixing"));
                if source(m, key) >= 2.0 {
                    ui.small(tl!("CSP mixing type 2 is approximated by blending"));
                }
            }
        }
        1162 => in_out_targets(ui, label, b),
        1171 | 1181 => in_out_length(ui, id, label, m, b),
        1910 => {
            ui.checkbox(&mut b.dual_brush.enabled, tl!(label));
        }
        51000 => {
            ui.add_enabled_ui(b.dual_brush.enabled, |ui| {
                let mut px = if b.dual_brush.size.is_finite() { b.dual_brush.size } else { 1.0 };
                if number(ui, label, &mut px, 0.5..=prism_engine::paint::MAX_BRUSH_SIZE, " px") {
                    b.dual_brush.size = px.clamp(0.5, prism_engine::paint::MAX_BRUSH_SIZE);
                }
            });
        }
        1110 => {
            // CSP draws ribbons only with tip images and without spraying.
            let usable = matches!(b.tip, prism_engine::paint::TipShape::Sampled(_) | prism_engine::paint::TipShape::Sequence(_)) && b.csp_spray.is_none();
            let mut on = b.csp_ribbon.is_some();
            let changed = ui.add_enabled_ui(usable || on, |ui| ui.checkbox(&mut on, tl!(label)).changed()).inner;
            if changed {
                let direction = if b.angle.is_finite() { f64::from(-b.angle) } else { 0.0 };
                b.csp_ribbon = on.then(|| prism_engine::paint::csp_ribbon::CspRibbon::from_rotation(direction));
            }
            if !usable {
                ui.small(tl!("Ribbons need a tip image and no spray"));
            }
        }
        1151 | 1250 | 1500 => {
            let on = source(m, key) != 0.0 || (id == 1151 && source(m, "BrushUseRevision") != 0.0);
            let value = if id == 1151 && source(m, "BrushUseRevision") == 0.0 { tl!("Off").to_string() } else { format!("{:.0}", source(m, key)) };
            unsupported(ui, label, value, false);
            if on {
                ui.small(tl!("The stroke is drawn without this CSP effect"));
            }
        }
        1130 => {
            let mut on = b.csp_spray.is_some();
            if ui.checkbox(&mut on, tl!(label)).changed() {
                b.csp_spray = on.then(|| spray_from_source(m));
            }
        }
        1135 | 1061 => match &mut b.csp_spray {
            Some(spray) if id == 1135 => {
                let mut percent = if spray.bias.is_finite() { spray.bias * 100.0 } else { 0.0 };
                if number(ui, label, &mut percent, -100.0..=100.0, "") {
                    spray.bias = (percent / 100.0).clamp(-1.0, 1.0);
                }
            }
            Some(spray) => {
                // CSP's particle direction is clockwise; the spray angle counter-clockwise.
                let mut degrees = if spray.angle.is_finite() { (-spray.angle + 180.0).rem_euclid(360.0) - 180.0 } else { 0.0 };
                if number(ui, label, &mut degrees, -180.0..=180.0, "°") {
                    spray.angle = -degrees.clamp(-180.0, 180.0);
                }
            }
            None => unsupported(ui, label, format!("{:.0}", source(m, key)), true),
        },
        1102 => {
            // Per-tip colour jitter (CSP's per-stroke variation, 1270, is separate).
            let cd = &mut b.color_dynamics;
            let mut on = cd.enabled && cd.per_tip;
            if ui.checkbox(&mut on, tl!(label)).changed() {
                cd.enabled = on;
                cd.per_tip = true;
            }
            if !b.csp_tip_colors.is_empty() {
                ui.small(tl!("Colour tips keep their own colours"));
            }
        }
        1103 | 1105 | 1107 => {
            let cd = &mut b.color_dynamics;
            let on = cd.enabled && cd.per_tip;
            ui.add_enabled_ui(on, |ui| {
                // Prism's hue jitter j spreads +-180 degrees times j: CSP's hue change is 180 j.
                let (value, scale, unit) = match id {
                    1103 => (&mut cd.hue_jitter, 180.0, "°"),
                    1105 => (&mut cd.saturation_jitter, 100.0, ""),
                    _ => (&mut cd.brightness_jitter, 100.0, ""),
                };
                let mut shown = if value.is_finite() { (*value * scale).clamp(0.0, scale) } else { 0.0 };
                if number(ui, label, &mut shown, 0.0..=scale, unit) {
                    *value = (shown / scale).clamp(0.0, 1.0);
                }
            });
            if !on {
                ui.small(tl!("Turn on Change brush tip color to use it"));
            }
        }
        1020 => {
            let mut percent = b.flow * 100.0;
            if number(ui, label, &mut percent, 0.0..=100.0, " %") {
                b.flow = (percent / 100.0).clamp(0.0, 1.0);
            }
        }
        1030 => {
            // Automatic spacing follows the hardness, as on import (`prism_paint::sut::auto_spacing`).
            let (image_tip, thickness) = spacing_inputs(m, b);
            let kind = prism_engine::paint::sut::auto_spacing_kind(b.spacing, b.hardness, image_tip, thickness);
            let mut percent = b.hardness * 100.0;
            if number(ui, label, &mut percent, 0.0..=100.0, " %") {
                b.hardness = (percent / 100.0).clamp(0.0, 1.0);
                if let Some(s) = prism_engine::paint::sut::auto_spacing(kind, b.hardness, image_tip, thickness) {
                    b.spacing = s;
                }
            }
        }
        1060 => {
            // CSP's direction is clockwise; Prism's angle counter-clockwise. A ribbon snaps its tip
            // to quarter turns of the same direction, so it follows the edit.
            let mut degrees = if b.angle.is_finite() { (-b.angle + 180.0).rem_euclid(360.0) - 180.0 } else { 0.0 };
            if number(ui, label, &mut degrees, -180.0..=180.0, "°") {
                let degrees = degrees.clamp(-180.0, 180.0);
                b.angle = -degrees;
                if b.csp_ribbon.is_some() {
                    b.csp_ribbon = Some(prism_engine::paint::csp_ribbon::CspRibbon::from_rotation(f64::from(degrees)));
                }
            }
        }
        1131 | 1133 => {
            let size = b.size;
            match &mut b.csp_spray {
                Some(spray) if id == 1131 => {
                    let mut px = spray.size;
                    if number(ui, label, &mut px, 0.5..=prism_engine::paint::MAX_BRUSH_SIZE, " px") {
                        spray.size = px.clamp(0.5, prism_engine::paint::MAX_BRUSH_SIZE);
                        // A particle size synchronized with the brush size keeps the new ratio.
                        if spray.size_ratio.is_some() && size > 0.0 {
                            spray.size_ratio = Some(spray.size / size);
                        }
                    }
                    if source(m, "BrushSpraySizeUnit") != 0.0 {
                        ui.small(tl!("Shown in pixels; the CSP size unit is not converted yet"));
                    }
                }
                Some(spray) => {
                    let max = prism_engine::paint::csp_spray::MAX_PARTICLES;
                    let mut count = spray.count.clamp(1, max) as f32;
                    if number(ui, label, &mut count, 1.0..=max as f32, "") {
                        spray.count = (count.round() as u32).clamp(1, max);
                    }
                }
                None => unsupported(ui, label, format!("{:.1}", source(m, key)), true),
            }
        }
        100 => {
            ui.label(tl!(label));
            ui.add_enabled(
                false,
                egui::Button::new(if !b.texture.enabled {
                    tl!("None")
                } else if m.texture_name.is_empty() {
                    tl!("Imported texture")
                } else {
                    &m.texture_name
                }),
            );
        }
        110 | 120 => {
            ui.add_enabled_ui(b.texture.enabled, |ui| {
                if id == 110 {
                    let mut percent = b.texture.depth * 100.0;
                    if number(ui, label, &mut percent, 0.0..=100.0, " %") {
                        b.texture.depth = percent / 100.0;
                    }
                } else {
                    let factor = source(m, "PrismTextureScaleFactor").max(1.0);
                    let mut percent = b.texture.scale * 100.0 / factor;
                    if number(ui, label, &mut percent, 1.0 / factor..=1000.0 / factor, " %") {
                        b.texture.scale = percent / 100.0 * factor;
                    }
                }
            });
            if !b.texture.enabled {
                ui.small(tl!("Texture is disabled or unavailable"));
            }
        }
        1201 => {
            use prism_engine::paint::csp_water_edge::{CspWaterEdge, MAX_BLUR, MAX_RADIUS};
            let mut on = b.csp_water_edge.is_some();
            if ui.checkbox(&mut on, tl!(label)).changed() {
                let radius = source(m, key);
                b.csp_water_edge = on.then(|| CspWaterEdge { radius: if radius > 0.0 { radius.min(MAX_RADIUS) } else { 2.0 }, ..Default::default() });
            }
            if let Some(e) = &mut b.csp_water_edge {
                number(ui, "Edge range", &mut e.radius, 0.1..=MAX_RADIUS, " px");
                if details {
                    for (name, value) in [("Opacity influence", &mut e.opacity), ("Darkness influence", &mut e.darkness)] {
                        let mut percent = *value * 100.0;
                        if number(ui, name, &mut percent, 0.0..=100.0, " %") {
                            *value = percent / 100.0;
                        }
                    }
                    ui.checkbox(&mut e.after_drag, tl!("Process after drag"));
                    ui.add_enabled_ui(e.after_drag, |ui| {
                        number(ui, "Edge blur", &mut e.blur, 0.0..=MAX_BLUR.min(100.0), " px");
                    });
                }
                if e.after_drag {
                    ui.small(tl!("Applied when the stroke ends"));
                }
            }
        }
        1081 | 1083 | 1085 => match &mut b.csp_mixing {
            Some(mixing) => {
                let value = match id {
                    1081 => &mut mixing.amount,
                    1083 => &mut mixing.density,
                    _ => &mut mixing.stretch,
                };
                let mut percent = *value * 100.0;
                if number(ui, label, &mut percent, 0.0..=100.0, "") {
                    *value = (percent / 100.0).clamp(0.0, 1.0);
                }
            }
            None => unsupported(ui, label, format!("{:.0}", source(m, key)), source(m, "BrushUseWaterColor") == 0.0 && source(m, "BrushUseWaterColor2") == 0.0),
        },
        40..=42 => {
            use prism_engine::paint::csp_stabilizer::{CspStabilizer, MAX_LEVEL};
            // An imported level of 0 has no stabilizer; start from the stored speed flag.
            let mut st =
                b.csp_stabilizer.clone().unwrap_or(CspStabilizer { adjust_by_speed: source(m, "FlickerReductionBySpeed") != 0.0, ..Default::default() });
            let changed = match id {
                40 => {
                    let mut level = st.level as f32;
                    let changed = number(ui, label, &mut level, 0.0..=MAX_LEVEL as f32, "");
                    st.level = level.round().clamp(0.0, MAX_LEVEL as f32) as u32;
                    changed
                }
                41 => ui.checkbox(&mut st.adjust_by_speed, tl!(label)).changed(),
                _ => {
                    // Type 0 follows the pointer speed; any other stored type follows the path.
                    let mut kind = st.speed_type.min(1);
                    let opts = [(0u32, "Pointer speed"), (1, "Path speed and straightness")];
                    let changed = ui
                        .add_enabled_ui(st.adjust_by_speed, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(tl!(label));
                                crate::widgets::dropdown(ui, "sut-speed-type", &mut kind, &opts, ui.available_width().min(220.0))
                            })
                            .inner
                        })
                        .inner;
                    if changed {
                        st.speed_type = kind;
                    }
                    changed
                }
            };
            if changed {
                b.csp_stabilizer = Some(st);
            }
        }
        _ => unsupported(ui, label, format!("{:.0}", source(m, key)), false),
    }
}

/// Spacing (1040) and its type (1042, `BrushAutoIntervalType`): fixed spacing edits the
/// fraction of the diameter; the automatic types set it from hardness and thickness as the import
/// does (`prism_paint::sut::auto_spacing`). The shown type is the one the brush's spacing matches.
/// Starting and ending: which parameters taper (`CspInOut::targets`), each with its minimum.
fn in_out_targets(ui: &mut egui::Ui, label: &str, b: &mut BrushSettings) {
    use prism_engine::paint::{CspInOutParam as P, CspInOutTarget};
    ui.label(tl!(label));
    let Some(io) = &mut b.csp_in_out else {
        ui.small(tl!("Turn on Starting or Ending to choose what they affect"));
        return;
    };
    let params = [
        (P::Size, "Brush Size"),
        (P::Flow, "Brush density"),
        (P::TextureDensity, "Texture density"),
        (P::SpraySize, "Particle size"),
        (P::SprayDensity, "Particle density"),
    ];
    for (param, name) in params {
        let at = io.targets.iter().position(|t| t.param == param);
        let mut on = at.is_some();
        ui.horizontal(|ui| {
            if ui.checkbox(&mut on, tl!(name)).changed() {
                match (on, at) {
                    (true, None) => io.targets.push(CspInOutTarget { param, minimum: 0.0 }),
                    (false, Some(i)) => {
                        io.targets.remove(i);
                    }
                    _ => {}
                }
            }
            if let Some(t) = io.targets.iter_mut().find(|t| t.param == param) {
                let mut percent = if t.minimum.is_finite() { (t.minimum * 100.0).clamp(0.0, 100.0) } else { 0.0 };
                let r = ui.add(egui::DragValue::new(&mut percent).range(0.0..=100.0).suffix(" %")).on_hover_text(tl!("Minimum"));
                if r.changed() {
                    t.minimum = (percent / 100.0).clamp(0.0, 1.0);
                }
            }
        });
    }
}

/// Starting (1171) or Ending (1181): on/off and the length, in pixels or percent of the stroke.
fn in_out_length(ui: &mut egui::Ui, id: u32, label: &str, m: &SutBrushMetadata, b: &mut BrushSettings) {
    use prism_engine::paint::{CspInOut, CspInOutMode, CspInOutParam, CspInOutTarget, csp_in_out::MAX_LENGTH};
    let start = id == 1171;
    let current = b.csp_in_out.as_ref().and_then(|io| if start { io.start } else { io.end });
    let mut on = current.is_some();
    if ui.checkbox(&mut on, tl!(label)).changed() {
        let io =
            b.csp_in_out.get_or_insert_with(|| CspInOut { targets: vec![CspInOutTarget { param: CspInOutParam::Size, minimum: 0.0 }], ..Default::default() });
        let value = on.then(|| match io.mode {
            CspInOutMode::Length => {
                let v = f64::from(source(m, if start { "BrushInLength" } else { "BrushOutLength" }));
                if v > 0.0 { v.min(MAX_LENGTH) } else { 20.0 }
            }
            CspInOutMode::Percent => 0.1,
        });
        if start {
            io.start = value;
        } else {
            io.end = value;
        }
        if io.start.is_none() && io.end.is_none() {
            b.csp_in_out = None;
        }
        return;
    }
    let Some(io) = &mut b.csp_in_out else { return };
    let mut mode = io.mode;
    ui.horizontal(|ui| {
        crate::widgets::dropdown(
            ui,
            if start { "sut-in-mode" } else { "sut-out-mode" },
            &mut mode,
            &[(CspInOutMode::Length, "Length"), (CspInOutMode::Percent, "Percent of stroke")],
            ui.available_width().min(180.0),
        );
    });
    if mode != io.mode {
        // Switching keeps a usable taper: 10 % of the stroke, or 20 px.
        let reset = |v: Option<f64>| v.map(|_| if mode == CspInOutMode::Percent { 0.1 } else { 20.0 });
        io.start = reset(io.start);
        io.end = reset(io.end);
        io.mode = mode;
    }
    let value = if start { &mut io.start } else { &mut io.end };
    if let Some(v) = value {
        let (scale, max, unit) = if io.mode == CspInOutMode::Percent { (100.0, 100.0, " %") } else { (1.0, 1000.0, " px") };
        let mut shown = if v.is_finite() { (*v * scale).clamp(0.0, max) as f32 } else { 0.0 };
        if ui.add(egui::Slider::new(&mut shown, 0.0..=max as f32).suffix(unit)).changed() {
            *v = (f64::from(shown) / scale).max(1e-3);
        }
    }
}

/// The thickness axis: the brush's own (only one axis below 1 is squashed), else the imported
/// `BrushVerticalThicknes`.
fn thickness_vertical(m: &SutBrushMetadata, b: &BrushSettings) -> bool {
    match b.csp_tip_scale {
        Some([x, y]) if x != y && (x == 1.0 || y == 1.0) => x == 1.0,
        _ => source(m, "BrushVerticalThicknes") != 0.0,
    }
}

/// A spray switched on in the panel: the imported particle settings (CSP's defaults when unset).
fn spray_from_source(m: &SutBrushMetadata) -> prism_engine::paint::CspSpray {
    let value = |key: &str, default: f32| m.parameters.get(key).copied().filter(|v| v.is_finite()).map_or(default, |v| v.clamp(-1e6, 1e6) as f32);
    prism_engine::paint::CspSpray {
        size: value("BrushSpraySize", 2.0).clamp(0.5, prism_engine::paint::MAX_BRUSH_SIZE),
        count: (value("BrushSprayDensity", 8.0).round().max(1.0) as u32).min(prism_engine::paint::csp_spray::MAX_PARTICLES),
        bias: (value("BrushSprayBias", 0.0) / 100.0).clamp(-1.0, 1.0),
        angle: -value("BrushRotationInSpray", 0.0).clamp(-360.0, 360.0),
        ..Default::default()
    }
}

/// Whether the brush has an image tip, and its thickness: the inputs of automatic spacing.
fn spacing_inputs(m: &SutBrushMetadata, b: &BrushSettings) -> (bool, f32) {
    let image_tip = matches!(b.tip, prism_engine::paint::TipShape::Sampled(_) | prism_engine::paint::TipShape::Sequence(_));
    let vertical = thickness_vertical(m, b);
    let [x, y] = b.csp_tip_scale.unwrap_or([1.0, 1.0]);
    (image_tip, if vertical { y } else { x })
}

fn spacing(ui: &mut egui::Ui, id: u32, label: &str, m: &SutBrushMetadata, b: &mut BrushSettings) {
    use prism_engine::paint::sut::{auto_spacing, auto_spacing_kind};
    let (image_tip, thickness) = spacing_inputs(m, b);
    let kind = auto_spacing_kind(b.spacing, b.hardness, image_tip, thickness);
    if id == 1042 {
        let mut picked = kind;
        let opts = [(0i64, "Fixed"), (1, "Wide (automatic)"), (2, "Normal (automatic)"), (3, "Narrow (automatic)")];
        ui.horizontal(|ui| {
            ui.label(tl!(label));
            crate::widgets::dropdown(ui, "sut-spacing-type", &mut picked, &opts, ui.available_width().min(200.0));
        });
        if picked != kind {
            b.spacing = match auto_spacing(picked, b.hardness, image_tip, thickness) {
                Some(s) => s,
                // Back to fixed: the imported fixed spacing, else keep the current one.
                None => Some(source(m, "BrushInterval") / 100.0).filter(|s| *s > 0.0).map_or(b.spacing, |s| s.clamp(0.01, 10.0)),
            };
        }
        return;
    }
    let mut percent = b.spacing * 100.0;
    let changed = ui.add_enabled_ui(kind == 0, |ui| number(ui, label, &mut percent, 1.0..=1000.0, " %")).inner;
    if changed {
        b.spacing = (percent / 100.0).clamp(0.01, 10.0);
    }
    if kind != 0 {
        ui.small(tl!("Set by the spacing type"));
    }
}

pub fn show(app: &mut PrismApp, ui: &mut egui::Ui) {
    let Some((name, metadata)) = active(app) else { return };
    let name = name.to_owned();
    let m = metadata.clone();
    let before = app.session.tools.brush.clone();
    let mut b = before.clone();
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(&name).strong());
    if !m.kind().is_verified_brush() {
        ui.small(tl!("Additional settings for this sub-tool type are not mapped yet"));
    }
    let width = ui.available_width().clamp(120.0, 540.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 64.0), Sense::hover());
    ui.painter().rect_filled(rect, t.radius_sm, t.field);
    let tex = brush_preview::stroke_texture(ui.ctx(), "sut-settings-strip", &b, width as u32, 64, t.text);
    ui.painter().image(tex.id(), rect, brush_panel::full_uv(), Color32::WHITE);
    ui.horizontal(|ui| {
        ui.selectable_value(&mut app.ui.sut_panel.details, false, tl!("Tool Settings"));
        ui.selectable_value(&mut app.ui.sut_panel.details, true, tl!("Advanced Settings"));
    });
    let details = app.ui.sut_panel.details;
    let mut categories = m.effective().show_categories;
    if details {
        if ui.checkbox(&mut categories, tl!("Show category")).changed() {
            brush_panel::run_or_status(app, "brush.sutDisplay.set", json!({"name":name,"showCategories":categories}));
        }
        if ui.button(tl!("Restore imported display layout")).clicked() {
            brush_panel::run_or_status(app, "brush.sutDisplay.reset", json!({"name":name}));
        }
    }
    let max_height = (ui.ctx().content_rect().height() - 260.0).clamp(100.0, 560.0);
    egui::ScrollArea::vertical().id_salt("sut-settings-scroll").max_height(max_height).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().slider_width = (width - 110.0).max(70.0);
        let mut category = "";
        for &(id, label, key, group) in ITEMS {
            if !available(&m, id, key) {
                continue;
            }
            let mut visible = m.effective().visible.contains(&id);
            if !details && !visible {
                continue;
            }
            if id == 120 && !details && m.effective().visible.contains(&110) && !app.ui.sut_panel.texture_scale_expanded {
                continue;
            }
            if categories && group != category {
                ui.separator();
                ui.label(egui::RichText::new(tl!(group)).strong());
                category = group;
            }
            ui.push_id(id, |ui| {
                ui.horizontal_top(|ui| {
                    if details {
                        let hint = format!("{}: {}", tl!("Show in Tool Settings"), tl!(label));
                        let eye = icons::button(ui, if visible { "eye" } else { "eye-off" }, 22.0, visible, &hint);
                        eye.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, visible, &hint));
                        if eye.clicked() {
                            visible = !visible;
                            brush_panel::run_or_status(app, "brush.sutDisplay.set", json!({"name":name,"parameter":id,"visible":visible}));
                        }
                    }
                    ui.vertical(|ui| {
                        control(ui, id, label, key, &m, &mut b, details);
                    });
                });
                if id == 110 && !details && m.effective().visible.contains(&120) {
                    ui.toggle_value(&mut app.ui.sut_panel.texture_scale_expanded, tl!("Show texture scale"));
                }
                ui.separator();
            });
        }
        let mut unknown = m.original.visible.clone();
        unknown.extend(m.effective().visible.iter().copied());
        unknown.sort_unstable();
        unknown.dedup();
        for id in unknown.iter().filter(|id| !ITEMS.iter().any(|item| item.0 == **id)) {
            let visible = m.effective().visible.contains(id);
            if !details && !visible {
                continue;
            }
            ui.push_id(("unknown", id), |ui| {
                ui.label(format!("{} {id}", tl!("Unsupported CSP setting")));
                if details && ui.button(if visible { tl!("Hide") } else { tl!("Show") }).clicked() {
                    brush_panel::run_or_status(app, "brush.sutDisplay.set", json!({"name":name,"parameter":id,"visible":!visible}));
                }
            });
        }
        if m.effective().visible.is_empty() && !details {
            ui.label(tl!("No visible settings. Use Advanced Settings to select them."));
        }
    });
    brush_panel::commit_gesture(app, ui.ctx(), &before, &b);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use prism_engine::paint::{BrushPreset, sut::SutDisplay};

    fn harness() -> Harness<'static, PrismApp> {
        let mut app = PrismApp::new(prism_engine::Session::new(), crate::Services::default());
        let brush = BrushSettings { size: 15.0, opacity: 0.9, ..Default::default() };
        app.session.tools.presets.push(BrushPreset {
            name: "Research".into(),
            brush: brush.clone(),
            builtin: false,
            group: String::new(),
            sut: Some(SutBrushMetadata {
                original: SutDisplay { visible: vec![1000, 10], show_categories: false },
                input_op: 10,
                output_op: 10,
                ..Default::default()
            }),
        });
        app.session.tools.brush = brush;
        app.session.tools.brush_preset = Some("Research".into());
        let mut h = Harness::builder().with_size(vec2(360.0, 650.0)).build_ui_state(
            |ui, app| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    show(app, ui);
                }
            },
            app,
        );
        PrismApp::setup_context(&h.ctx, Default::default());
        h.run_steps(4);
        h
    }

    #[test]
    fn spacing_and_its_type_have_controls_instead_of_an_unsupported_row() {
        let mut h = harness();
        let spacing = prism_engine::paint::sut::auto_spacing(2, 0.5, false, 1.0).unwrap();
        {
            let app = h.state_mut();
            app.session.tools.brush.hardness = 0.5;
            app.session.tools.brush.spacing = spacing;
            let preset = app.session.tools.presets.last_mut().unwrap();
            preset.brush = app.session.tools.brush.clone();
            preset.sut.as_mut().unwrap().original.visible = vec![1000, 1040, 1042];
        }
        h.run_steps(3);
        assert!(h.query_by_label_contains("Unsupported CSP setting").is_none());
        assert!(h.query_by_label("Spacing type").is_some());
        if let Ok(image) = h.render() {
            let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sut-spacing-panel.png");
            let _ = image.save(out);
        }
        // The combo box's text is not an accessible label; its value comes from the spacing.
        let b = &h.state().session.tools.brush;
        assert_eq!(prism_engine::paint::sut::auto_spacing_kind(b.spacing, b.hardness, false, 1.0), 2);
    }

    #[test]
    fn p1_rows_have_controls_and_survive_hostile_values() {
        let ids = vec![1000, 1002, 1020, 1030, 1060, 1131, 1133];
        for hostile in [false, true] {
            let mut h = harness();
            {
                let app = h.state_mut();
                let b = &mut app.session.tools.brush;
                b.flow = if hostile { f32::NAN } else { 0.4 };
                b.hardness = if hostile { -3.0 } else { 0.5 };
                b.angle = if hostile { f32::INFINITY } else { -30.0 };
                b.csp_ribbon = Some(prism_engine::paint::csp_ribbon::CspRibbon::from_rotation(30.0));
                b.csp_spray = Some(prism_engine::paint::CspSpray {
                    size: if hostile { f32::NAN } else { 6.0 },
                    count: if hostile { u32::MAX } else { 12 },
                    ..Default::default()
                });
                let brush = b.clone();
                let preset = app.session.tools.presets.last_mut().unwrap();
                preset.brush = brush;
                preset.sut.as_mut().unwrap().original.visible = ids.clone();
            }
            h.run_steps(3);
            assert!(h.query_by_label_contains("Unsupported CSP setting").is_none(), "hostile={hostile}");
            for label in ["Specify by size on screen", "Brush density", "Hardness", "Direction", "Particle size", "Particle density"] {
                assert!(h.query_by_label(label).is_some(), "{label} (hostile={hostile})");
            }
            if !hostile && let Ok(image) = h.render() {
                let _ = image.save(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sut-p1-panel.png"));
            }
        }
        // Without spray the particle rows explain why they are inactive instead of editing nothing.
        let mut h = harness();
        h.state_mut().session.tools.presets.last_mut().unwrap().sut.as_mut().unwrap().original.visible = vec![1131];
        h.run_steps(3);
        assert!(h.query_by_label_contains("Disabled by the imported CSP settings").is_some());
    }

    #[test]
    fn p2_rows_have_controls_and_survive_hostile_values() {
        use prism_engine::paint::{GrayTile, TipShape, csp_tip::CspTipOrder};
        let ids = vec![1052, 1071, 1130, 1135, 1061, 1102, 1103, 1105, 1107];
        for hostile in [false, true] {
            let mut h = harness();
            {
                let app = h.state_mut();
                let b = &mut app.session.tools.brush;
                let v = |ok: f32| if hostile { f32::NAN } else { ok };
                b.tip = TipShape::Sequence(vec![GrayTile::from_fn(2, 2, |_, _| 1.0), GrayTile::from_fn(2, 2, |_, _| 0.5)]);
                b.csp_tip_order = Some(CspTipOrder::Random);
                b.csp_tip_scale = Some(if hostile { [f32::INFINITY, 1.0] } else { [1.0, 0.4] });
                b.csp_spray = Some(prism_engine::paint::CspSpray { bias: v(-0.3), angle: v(-20.0), ..Default::default() });
                b.color_dynamics.enabled = true;
                b.color_dynamics.per_tip = true;
                b.color_dynamics.hue_jitter = v(0.25);
                b.color_dynamics.saturation_jitter = v(0.5);
                b.color_dynamics.brightness_jitter = if hostile { -4.0 } else { 0.1 };
                let brush = b.clone();
                let preset = app.session.tools.presets.last_mut().unwrap();
                preset.brush = brush;
                let sut = preset.sut.as_mut().unwrap();
                sut.original.visible = ids.clone();
                sut.parameters.insert("BrushVerticalThicknes".into(), if hostile { f64::NAN } else { 1.0 });
            }
            h.run_steps(3);
            assert!(h.query_by_label_contains("Unsupported CSP setting").is_none(), "hostile={hostile}");
            for label in [
                "Thickness direction",
                "Tip order",
                "Spray effect",
                "Spray deviation",
                "Direction of particle",
                "Change brush tip color",
                "Hue",
                "Saturation",
                "Luminosity",
            ] {
                assert!(h.query_by_label_contains(label).is_some(), "{label} (hostile={hostile})");
            }
            if !hostile && let Ok(image) = h.render() {
                let _ = image.save(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sut-p2-panel.png"));
            }
        }
    }

    #[test]
    fn spray_switched_on_restores_the_imported_particles_and_thickness_axis_follows_the_brush() {
        let mut m = SutBrushMetadata::default();
        for (k, v) in [("BrushSpraySize", 7.5), ("BrushSprayDensity", 1e9), ("BrushSprayBias", -40.0), ("BrushRotationInSpray", 30.0)] {
            m.parameters.insert(k.into(), v);
        }
        let s = spray_from_source(&m);
        assert_eq!((s.size, s.count, s.bias, s.angle), (7.5, prism_engine::paint::csp_spray::MAX_PARTICLES, -0.4, -30.0));
        m.parameters.insert("BrushSpraySize".into(), f64::NAN);
        assert_eq!(spray_from_source(&m).size, 2.0, "a broken source falls back to CSP's default");
        let mut b = BrushSettings { csp_tip_scale: Some([1.0, 0.5]), ..Default::default() };
        assert!(thickness_vertical(&m, &b));
        b.csp_tip_scale = Some([0.5, 1.0]);
        assert!(!thickness_vertical(&m, &b));
        b.csp_tip_scale = None;
        m.parameters.insert("BrushVerticalThicknes".into(), 1.0);
        assert!(thickness_vertical(&m, &b), "no squash: the imported axis");
    }

    #[test]
    fn remaining_backlog_rows_are_named_and_survive_hostile_values() {
        use prism_engine::paint::{CspInOut, CspInOutParam, CspInOutTarget, GrayTile, TipShape};
        let ids = vec![1043, 50, 1202, 1203, 1270, 1080, 1162, 1171, 1181, 1910, 51000, 1110, 1151, 1250, 1500];
        for hostile in [false, true] {
            let mut h = harness();
            {
                let app = h.state_mut();
                let b = &mut app.session.tools.brush;
                let v = |ok: f32| if hostile { f32::NAN } else { ok };
                b.tip = TipShape::Sampled(GrayTile::from_fn(4, 4, |_, _| 1.0));
                b.csp_stabilizer = Some(prism_engine::paint::csp_stabilizer::CspStabilizer {
                    level: 10,
                    stickiness: if hostile { u32::MAX } else { 4 },
                    ..Default::default()
                });
                b.csp_water_edge = Some(prism_engine::paint::csp_water_edge::CspWaterEdge { opacity: v(0.5), darkness: v(0.2), ..Default::default() });
                b.csp_mixing = Some(prism_engine::paint::CspColorMixing::default());
                b.csp_in_out = Some(CspInOut {
                    start: Some(if hostile { f64::INFINITY } else { 30.0 }),
                    end: Some(40.0),
                    targets: vec![CspInOutTarget { param: CspInOutParam::Size, minimum: v(0.1) }],
                    ..Default::default()
                });
                b.dual_brush.enabled = true;
                b.dual_brush.size = v(12.0);
                let brush = b.clone();
                let preset = app.session.tools.presets.last_mut().unwrap();
                preset.brush = brush;
                let sut = preset.sut.as_mut().unwrap();
                sut.original.visible = ids.clone();
                sut.parameters.insert("BrushRevision".into(), if hostile { f64::NAN } else { 10.0 });
                sut.parameters.insert("BrushUseRevision".into(), 1.0);
            }
            h.run_steps(3);
            assert!(h.query_by_label_contains("Unsupported CSP setting").is_none(), "hostile={hostile}");
            for label in [
                "Continuous spraying",
                "Stickiness",
                "Change color per stroke",
                "Color mixing",
                "Starting",
                "Ending",
                "Dual brush",
                "Ribbon",
                "Post correction",
                "Refer to other layers",
                "Adjust position",
            ] {
                assert!(h.query_all_by_label_contains(label).next().is_some(), "{label} (hostile={hostile})");
            }
            if !hostile && let Ok(image) = h.render() {
                let _ = image.save(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sut-p3-panel.png"));
            }
        }
    }

    #[test]
    fn flip_and_at_least_1px_rows_toggle_the_brush() {
        let mut h = harness();
        h.state_mut().session.tools.presets.last_mut().unwrap().sut.as_mut().unwrap().original.visible = vec![1003, 1074, 1075];
        h.run_steps(2);
        assert!(h.query_by_label_contains("Unsupported CSP setting").is_none());
        for label in ["At least 1 pixel", "Flip horizontal", "Flip vertical"] {
            h.get_by_label(label).click();
            h.run_steps(2);
        }
        let b = &h.state().session.tools.brush;
        assert!(b.csp_at_least_1px && b.flip_x && b.flip_y);
        assert_eq!(h.state().session.journal.last().unwrap().0, "tools.setBrush");
        crate::i18n::with_language(crate::i18n::Lang::from_pref("ko"), || {
            for label in ["At least 1 pixel", "Flip horizontal", "Flip vertical"] {
                assert_ne!(tl!(label), label, "{label}");
            }
        });
    }

    #[test]
    fn ending_switched_off_last_removes_the_taper_and_on_starts_with_a_size_target() {
        let mut h = harness();
        h.state_mut().session.tools.presets.last_mut().unwrap().sut.as_mut().unwrap().original.visible = vec![1171];
        h.run_steps(2);
        assert!(h.state().session.tools.brush.csp_in_out.is_none());
        h.get_by_label("Starting").click();
        h.run_steps(2);
        let io = h.state().session.tools.brush.csp_in_out.clone().expect("starting turned on");
        assert_eq!((io.start, io.end), (Some(20.0), None));
        assert_eq!(io.targets.first().map(|t| t.param), Some(prism_engine::paint::CspInOutParam::Size));
        h.get_by_label("Starting").click();
        h.run_steps(2);
        assert!(h.state().session.tools.brush.csp_in_out.is_none(), "no start and no end: no taper");
    }

    #[test]
    fn eye_toggle_and_restore_only_change_display() {
        let mut h = harness();
        h.run();
        let before = h.state().session.tools.brush.clone();
        let pixels = brush_preview::preview_pixels(&before, 120, 40, [1.0; 4]);
        h.get_by_label("Advanced Settings").click();
        h.run();
        h.get_by_label("Show in Tool Settings: Brush Size").click();
        h.run();
        assert!(!active(h.state()).unwrap().1.effective().visible.contains(&1000));
        assert_eq!(before, h.state().session.tools.brush);
        assert_eq!(pixels, brush_preview::preview_pixels(&h.state().session.tools.brush, 120, 40, [1.0; 4]));
        h.get_by_label("Restore imported display layout").click();
        h.run();
        assert!(active(h.state()).unwrap().1.custom.is_none());
    }

    #[test]
    fn origin_is_explicit_and_view_preferences_round_trip() {
        let mut h = harness();
        assert!(active(h.state()).is_some());
        h.state_mut().session.tools.presets.last_mut().unwrap().sut = None;
        assert!(active(h.state()).is_none());
        let view = SutPanelState { details: true, texture_scale_expanded: true };
        assert_eq!(serde_json::from_str::<SutPanelState>(&serde_json::to_string(&view).unwrap()).unwrap(), view);
    }

    #[test]
    fn tool_switch_keeps_origin_with_its_brush() {
        let mut h = harness();
        h.state_mut().ui.tool = crate::state::Tool::Eraser;
        crate::paint_mouse::sync_tool_brush(h.state_mut());
        h.state_mut().session.tools.brush_preset = None;
        h.state_mut().ui.tool = crate::state::Tool::Brush;
        crate::paint_mouse::sync_tool_brush(h.state_mut());
        assert!(active(h.state()).is_some());
        h.state_mut().ui.tool = crate::state::Tool::Eraser;
        crate::paint_mouse::sync_tool_brush(h.state_mut());
        assert!(active(h.state()).is_none());
    }

    #[test]
    fn pen_sub_tools_are_a_verified_brush_family_and_fill_sub_tools_leave_the_brush_list() {
        let mut h = harness();
        let metadata = h.state_mut().session.tools.presets.last_mut().unwrap().sut.as_mut().unwrap();
        // CSP pens (11/11) offer their hidden properties like the brush family (10/10).
        (metadata.input_op, metadata.output_op) = (11, 11);
        metadata.parameters.insert("BrushThickness".into(), 240.0);
        assert!(available(metadata, 1050, "BrushThickness"));
        h.run();
        assert!(h.query_by_label("Additional settings for this sub-tool type are not mapped yet").is_none());
        // A fill sub-tool is not a brush: the brush library leaves it out.
        let count =
            |h: &Harness<'static, PrismApp>| crate::brush_panel::grouped_presets(&h.state().session.tools.presets).iter().map(|(_, v)| v.len()).sum::<usize>();
        let before = count(&h);
        let metadata = h.state_mut().session.tools.presets.last_mut().unwrap().sut.as_mut().unwrap();
        (metadata.input_op, metadata.output_op) = (50, 50);
        assert_eq!(count(&h), before - 1);
    }

    #[test]
    fn opacity_edit_dispatches_brush_command_and_changes_pixels() {
        let mut h = harness();
        // An older SUT may have no known operator IDs. Its verified visible
        // property IDs must still edit the actual imported brush, not stale source values.
        let metadata = h.state_mut().session.tools.presets.last_mut().unwrap().sut.as_mut().unwrap();
        metadata.input_op = 0;
        metadata.output_op = 0;
        metadata.parameters.insert("BrushThickness".into(), 240.0);
        assert!(!available(metadata, 1050, "BrushThickness"));
        h.run();
        let before = brush_preview::preview_pixels(&h.state().session.tools.brush, 120, 40, [1.0; 4]);
        let slider = h.get_by_role_and_label(egui::accesskit::Role::Slider, "Opacity").rect();
        let point = egui::pos2(slider.left() + slider.width() * 0.3, slider.center().y);
        h.hover_at(point);
        h.drag_at(point);
        h.run();
        h.drop_at(point);
        h.run_steps(3);
        assert!(h.state().session.tools.brush.opacity < 0.8);
        assert_eq!(h.state().session.journal.last().unwrap().0, "tools.setBrush");
        assert_ne!(before, brush_preview::preview_pixels(&h.state().session.tools.brush, 120, 40, [1.0; 4]));
    }

    #[test]
    fn watercolor_edge_toggles_the_rendered_effect_and_shows_advanced_controls() {
        let mut h = harness();
        let metadata = h.state_mut().session.tools.presets.last_mut().unwrap().sut.as_mut().unwrap();
        metadata.original.visible.push(1201);
        metadata.parameters.insert("BrushWaterEdgeRadius".into(), 3.0);
        h.run();
        assert!(h.state().session.tools.brush.csp_water_edge.is_none());
        h.get_by_label("Watercolor edge").click();
        h.run_steps(3);
        let edge = h.state().session.tools.brush.csp_water_edge.clone().expect("enabled");
        assert_eq!(edge.radius, 3.0, "starts from the imported range");
        assert_eq!(h.state().session.journal.last().unwrap().0, "tools.setBrush");
        h.get_by_role_and_label(egui::accesskit::Role::Slider, "Edge range");
        assert!(h.query_by_label("Process after drag").is_none(), "advanced controls stay in Advanced Settings");
        h.get_by_label("Advanced Settings").click();
        h.run();
        h.get_by_label("Process after drag").click();
        h.run_steps(3);
        assert!(h.state().session.tools.brush.csp_water_edge.as_ref().unwrap().after_drag);
        for label in ["Opacity influence", "Darkness influence", "Edge blur"] {
            h.get_by_role_and_label(egui::accesskit::Role::Slider, label);
        }
        h.get_by_label("Tool Settings").click();
        h.run();
        assert!(h.query_by_label("Applied when the stroke ends").is_some());
        h.get_by_label("Watercolor edge").click();
        h.run_steps(3);
        assert!(h.state().session.tools.brush.csp_water_edge.is_none());
        crate::i18n::with_language(crate::i18n::Lang::from_pref("ko"), || {
            for label in ["Edge range", "Opacity influence", "Darkness influence", "Process after drag", "Edge blur", "Applied when the stroke ends"] {
                assert_ne!(tl!(label), label, "{label}");
            }
        });
    }

    #[test]
    fn stabilization_and_adjust_by_speed_edit_the_brush() {
        let mut h = harness();
        let metadata = h.state_mut().session.tools.presets.last_mut().unwrap().sut.as_mut().unwrap();
        metadata.original.visible.extend([40, 41, 42]);
        metadata.parameters.insert("FlickerReductionBySpeed".into(), 1.0);
        h.run();
        assert!(h.state().session.tools.brush.csp_stabilizer.is_none());
        h.get_by_role_and_label(egui::accesskit::Role::Slider, "Stabilization");
        // ID 42 is the speed adjustment type, a recognized control (no unknown-ID warning).
        assert!(prism_engine::paint::sut::is_known_parameter(42));
        h.get_by_label("Speed adjustment type");
        h.get_by_label("Adjust by speed").click();
        h.run_steps(3);
        // Off by default at level 0 with the imported speed flag on: one click turns it off.
        let st = h.state().session.tools.brush.csp_stabilizer.clone().expect("edited");
        assert_eq!((st.level, st.adjust_by_speed), (0, false));
        assert_eq!(h.state().session.journal.last().unwrap().0, "tools.setBrush");
    }

    #[test]
    fn color_mixing_rows_edit_the_brush_only_when_mixing_is_imported() {
        let mut h = harness();
        let metadata = h.state_mut().session.tools.presets.last_mut().unwrap().sut.as_mut().unwrap();
        metadata.original.visible.extend([1081, 1083, 1085]);
        metadata.parameters.insert("BrushMixColor".into(), 80.0);
        h.run();
        assert!(h.query_by_role_and_label(egui::accesskit::Role::Slider, "Amount of paint").is_none(), "inactive mixing stays read-only");
        h.state_mut().session.tools.brush.csp_mixing =
            Some(prism_engine::paint::CspColorMixing { amount: 0.8, density: 0.6, stretch: 0.4, ..Default::default() });
        h.run();
        for label in ["Amount of paint", "Density of paint", "Color stretch"] {
            h.get_by_role_and_label(egui::accesskit::Role::Slider, label);
        }
    }

    #[test]
    fn catalog_labels_and_categories_have_korean_translations() {
        crate::i18n::with_language(crate::i18n::Lang::from_pref("ko"), || {
            for &(_, label, _, category) in ITEMS {
                assert_ne!(tl!(label), label, "{label}");
                assert_ne!(tl!(category), category, "{category}");
            }
        });
    }
}
