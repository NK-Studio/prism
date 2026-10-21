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
    (10, "Opacity", "Opacity", "Ink"),
    (30, "Blending mode", "CompositeMode", "Ink"),
    (1081, "Amount of paint", "BrushMixColor", "Ink"),
    (1083, "Density of paint", "BrushMixAlpha", "Ink"),
    (1085, "Color stretch", "BrushMixColorExtension", "Ink"),
    (20, "Anti-aliasing", "AntiAlias", "Anti-aliasing"),
    (1050, "Thickness", "BrushThickness", "Brush tip"),
    (100, "Texture", "TextureImage", "Texture"),
    (110, "Texture density", "TextureDensity", "Texture"),
    (120, "Scale ratio", "TextureScale2", "Texture"),
    (1201, "Watercolor edge", "BrushWaterEdgeRadius", "Watercolor edge"),
    (40, "Stabilization", "FlickerReduction", "Correction"),
    (41, "Adjust by speed", "FlickerReductionBySpeed", "Correction"),
    (42, "Speed adjustment type", "FlickerReductionBySpeedType", "Correction"),
    (1241, "Vector magnet", "BrushVectorMagnetPower", "Correction"),
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
            let vertical = source(m, "BrushVerticalThicknes") != 0.0;
            let [x, y] = b.csp_tip_scale.unwrap_or([1.0, 1.0]);
            let mut percent = if vertical { y } else { x } * 100.0;
            if number(ui, label, &mut percent, 1.0..=1000.0, " %") {
                b.csp_tip_scale = Some(if vertical { [x, percent / 100.0] } else { [percent / 100.0, y] });
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
        let count = |h: &Harness<'static, PrismApp>| crate::brush_panel::grouped_presets(&h.state().session.tools.presets).iter().map(|(_, v)| v.len()).sum::<usize>();
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
