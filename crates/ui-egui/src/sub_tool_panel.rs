//! Clip Studio workspace panels: Sub Tool (the current CSP tool's sub-tools: built-in ones and
//! imported `.sut` files) and Tool Property (the selected sub-tool's settings). See `csp_tools`.
use egui::{Sense, vec2};
use serde_json::json;

use crate::PrismApp;
use crate::csp_tools::{CspTool, placement};
use crate::state::Tool;
use crate::theme::Tokens;

/// Built-in sub-tools: Prism tools and options standing in for Clip Studio Paint's defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    Zoom,
    Hand,
    Rotate,
    MoveLayer,
    Rectangle,
    Ellipse,
    Lasso,
    Polyline,
    ReferEditing,
    ReferAll,
    PickColor,
    EraserHard,
    EraserSoft,
    Blur,
    FingerTip,
    FillEditing,
    FillOthers,
    LinearGradient,
    RadialGradient,
    FigureRectangle,
    FigureEllipse,
    FigurePolygon,
    StraightLine,
    Text,
    VerticalText,
}

impl Builtin {
    fn label(self) -> &'static str {
        match self {
            Builtin::Zoom => "Zoom",
            Builtin::Hand => "Hand",
            Builtin::Rotate => "Rotate",
            Builtin::MoveLayer => "Move layer",
            Builtin::Rectangle | Builtin::FigureRectangle => "Rectangle",
            Builtin::Ellipse | Builtin::FigureEllipse => "Ellipse",
            Builtin::Lasso => "Lasso",
            Builtin::Polyline => "Polyline",
            Builtin::ReferEditing => "Refer to editing layer only",
            Builtin::ReferAll => "Refer to all layers",
            Builtin::PickColor => "Pick color from layer",
            Builtin::EraserHard => "Hard",
            Builtin::EraserSoft => "Soft",
            Builtin::Blur => "Blur",
            Builtin::FingerTip => "Finger tip",
            Builtin::FillEditing => "Refer only to editing layer",
            Builtin::FillOthers => "Refer other layers",
            Builtin::LinearGradient => "Linear gradient",
            Builtin::RadialGradient => "Radial gradient",
            Builtin::FigurePolygon => "Polygon",
            Builtin::StraightLine => "Straight line",
            Builtin::Text => "Text",
            Builtin::VerticalText => "Vertical text",
        }
    }

    fn tool(self) -> Tool {
        match self {
            Builtin::Zoom => Tool::Zoom,
            Builtin::Hand => Tool::Hand,
            Builtin::Rotate => Tool::RotateView,
            Builtin::MoveLayer => Tool::Move,
            Builtin::Rectangle => Tool::RectMarquee,
            Builtin::Ellipse => Tool::EllipseMarquee,
            Builtin::Lasso => Tool::Lasso,
            Builtin::Polyline => Tool::PolygonLasso,
            Builtin::ReferEditing | Builtin::ReferAll => Tool::MagicWand,
            Builtin::PickColor => Tool::Eyedropper,
            Builtin::EraserHard | Builtin::EraserSoft => Tool::Eraser,
            Builtin::Blur => Tool::Blur,
            Builtin::FingerTip => Tool::Smudge,
            Builtin::FillEditing | Builtin::FillOthers => Tool::PaintBucket,
            Builtin::LinearGradient | Builtin::RadialGradient => Tool::Gradient,
            Builtin::FigureRectangle => Tool::Rectangle,
            Builtin::FigureEllipse => Tool::EllipseShape,
            Builtin::FigurePolygon => Tool::Polygon,
            Builtin::StraightLine => Tool::Line,
            Builtin::Text => Tool::Type,
            Builtin::VerticalText => Tool::VerticalType,
        }
    }

    fn for_tool(tool: CspTool) -> &'static [Builtin] {
        match tool {
            CspTool::Zoom => &[Builtin::Zoom],
            CspTool::Hand => &[Builtin::Hand, Builtin::Rotate],
            CspTool::MoveLayer => &[Builtin::MoveLayer],
            CspTool::Selection => &[Builtin::Rectangle, Builtin::Ellipse, Builtin::Lasso, Builtin::Polyline],
            CspTool::AutoSelect => &[Builtin::ReferEditing, Builtin::ReferAll],
            CspTool::Eyedropper => &[Builtin::PickColor],
            CspTool::Eraser => &[Builtin::EraserHard, Builtin::EraserSoft],
            CspTool::Blend => &[Builtin::Blur, Builtin::FingerTip],
            CspTool::Fill => &[Builtin::FillEditing, Builtin::FillOthers],
            CspTool::Gradient => &[Builtin::LinearGradient, Builtin::RadialGradient],
            CspTool::Figure => &[Builtin::StraightLine, Builtin::FigureRectangle, Builtin::FigureEllipse, Builtin::FigurePolygon],
            CspTool::Text => &[Builtin::Text, Builtin::VerticalText],
            CspTool::Pen | CspTool::Pencil | CspTool::Brush | CspTool::Airbrush | CspTool::Decoration => &[],
        }
    }

    /// The options this sub-tool sets on its Prism tool.
    fn apply(self, app: &mut PrismApp) {
        let o = &mut app.ui.tool_options;
        match self {
            Builtin::ReferEditing | Builtin::FillEditing => o.sample_all_layers = false,
            Builtin::ReferAll | Builtin::FillOthers => o.sample_all_layers = true,
            Builtin::LinearGradient => o.gradient_style = "linear".into(),
            Builtin::RadialGradient => o.gradient_style = "radial".into(),
            Builtin::EraserHard | Builtin::EraserSoft => {
                // A plain round tip, keeping the eraser's size.
                let brush = prism_engine::BrushSettings {
                    size: app.session.tools.brush.size,
                    hardness: if self == Builtin::EraserHard { 1.0 } else { 0.0 },
                    ..Default::default()
                };
                let _ = app.run("tools.setBrush", json!({"brush": brush}));
            }
            _ => {}
        }
    }
}

/// Where a sub-tool comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// A brush preset (imported `.sut` or Prism's own), picked with `tools.setBrush {preset}`.
    Preset(String),
    /// An imported non-brush sub-tool run by a Prism tool with options from its settings.
    Imported {
        preset: String,
        tool: Tool,
    },
    Builtin(Builtin),
}

/// The Prism tool that runs an imported non-brush sub-tool, if any: chosen by the CSP icon
/// (`EditImageTool.todb`), else the input operation. Fill and auto select take their reference
/// and tolerance settings, gradient its shape and dither, text its size (see [`apply_imported`]).
/// Enclose-and-fill (output 51) runs on the Lasso: closing the outline fills (`enclose_fill_ui`).
pub fn imported_tool(m: &prism_engine::paint::sut::SutBrushMetadata) -> Option<Tool> {
    use prism_engine::paint::sut::SutToolKind as K;
    Some(match (m.kind(), m.node_icon, m.input_op, m.output_op) {
        (K::Fill, ..) if m.output_op == 50 || m.output_op == 54 => Tool::PaintBucket,
        (K::Fill, ..) if m.output_op == 51 => Tool::Lasso,
        (K::Selection, 401, ..) | (K::Selection, _, 50, _) => Tool::MagicWand,
        (K::Selection, 200, ..) => Tool::RectMarquee,
        (K::Selection, 203, ..) => Tool::EllipseMarquee,
        (K::Selection, 201, ..) | (K::Selection, _, 33, _) => Tool::PolygonLasso,
        (K::Selection, 202 | 204 | 205, ..) | (K::Selection, _, 31, _) => Tool::Lasso,
        (K::Selection, _, 30, _) => Tool::RectMarquee,
        (K::Gradient, ..) => Tool::Gradient,
        (K::Text, ..) => {
            if m.parameters.get("TextVertical").is_some_and(|v| *v != 0.0) {
                Tool::VerticalType
            } else {
                Tool::Type
            }
        }
        (K::Figure, 210, ..) | (K::Figure, _, 32, _) => Tool::Line,
        (K::Figure, 217, ..) => Tool::EllipseShape,
        (K::Figure, 218, ..) => Tool::Polygon,
        (K::Figure, 213, ..) | (K::Figure, _, 30, _) => Tool::Rectangle,
        (K::Eyedropper, ..) => Tool::Eyedropper,
        (K::Operation, ..) if m.output_op == 40 => Tool::Move,
        _ => return None,
    })
}

/// Options an imported non-brush sub-tool sets on its Prism tool.
pub(crate) fn apply_imported(app: &mut PrismApp, preset: &str) {
    let Some(params) = app.session.tools.presets.iter().find(|p| p.name == preset).and_then(|p| p.sut.as_ref()).map(|m| m.parameters.clone()) else { return };
    let get = |k: &str| params.get(k).copied().filter(|v| v.is_finite());
    let o = &mut app.ui.tool_options;
    if let Some(v) = get("FillReferMultiLayer") {
        o.sample_all_layers = v != 0.0;
    }
    // CSP's colour margin is a percentage of the channel range.
    if let Some(v) = get("FillColorMargin") {
        o.tolerance = (v.clamp(0.0, 100.0) * 2.55) as f32;
    }
    if let Some(v) = get("FillContinuousPixel") {
        o.contiguous = v != 0.0;
    }
    if let Some(v) = get("FillAntiAlias") {
        o.anti_alias = v != 0.0;
    }
    // A fill sub-tool's opacity is the fill opacity (percent, as in CLIP STUDIO).
    if let Some(v) = get("Opacity").filter(|_| params.contains_key("FillColorMargin")) {
        o.fill_opacity = v.clamp(0.0, 100.0) as f32;
    }
    if let Some(v) = get("GradationShapeKind") {
        o.gradient_style = if v == 0.0 { "linear" } else { "radial" }.into();
    }
    if let Some(v) = get("GradationDither") {
        o.gradient_dither = v != 0.0;
    }
    if let Some(v) = get("GradationReverseSaved") {
        o.gradient_reverse = v != 0.0;
    }
    if let Some(v) = get("TextFontSize") {
        o.type_size = v.clamp(1.0, 1296.0) as f32;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SubTool {
    /// Stable id remembered per CSP tool (`sut:<name>`, `preset:<name>` or `builtin:<label>`).
    pub id: String,
    pub name: String,
    pub source: Source,
    /// Prism can run it (imported fill, selection, text… sub-tools are listed but not yet run).
    pub usable: bool,
}

/// The sub-tools listed for `tool`: built-ins, then imported sub-tools placed there; the Brush
/// tool also lists Prism's own brush presets.
pub fn sub_tools(app: &PrismApp, tool: CspTool) -> Vec<SubTool> {
    let mut out: Vec<SubTool> = Builtin::for_tool(tool)
        .iter()
        .map(|b| SubTool { id: format!("builtin:{}", b.label()), name: b.label().into(), source: Source::Builtin(*b), usable: true })
        .collect();
    for p in &app.session.tools.presets {
        match &p.sut {
            Some(m) if placement(m) == Some(tool) => {
                let (source, usable) = if m.kind().is_brush() {
                    (Source::Preset(p.name.clone()), tool.paints())
                } else {
                    match imported_tool(m) {
                        Some(t) => (Source::Imported { preset: p.name.clone(), tool: t }, true),
                        None => (Source::Preset(p.name.clone()), false),
                    }
                };
                out.push(SubTool { id: format!("sut:{}", p.name), name: p.name.clone(), source, usable });
            }
            None if tool == CspTool::Brush => {
                out.push(SubTool { id: format!("preset:{}", p.name), name: p.name.clone(), source: Source::Preset(p.name.clone()), usable: true })
            }
            _ => {}
        }
    }
    out
}

/// Use `sub` as `tool`'s current sub-tool.
pub fn apply(app: &mut PrismApp, tool: CspTool, sub: &SubTool) {
    if !sub.usable {
        return;
    }
    app.ui.csp_tool = tool;
    app.ui.csp_sub_tools.insert(tool, sub.id.clone());
    app.ui.tool = match &sub.source {
        Source::Builtin(b) => b.tool(),
        Source::Imported { tool: t, .. } => *t,
        Source::Preset(_) => tool.backing(),
    };
    crate::paint_mouse::sync_tool_brush(app);
    match &sub.source {
        Source::Preset(name) => {
            if let Err(e) = app.run("tools.setBrush", json!({"preset": name})) {
                app.ui.status = e;
            }
        }
        Source::Builtin(b) => b.apply(app),
        Source::Imported { preset, .. } => apply_imported(app, preset),
    }
}

/// The selected sub-tool of the current CSP tool.
pub fn current(app: &PrismApp) -> Option<SubTool> {
    let id = app.ui.csp_sub_tools.get(&app.ui.csp_tool)?;
    sub_tools(app, app.ui.csp_tool).into_iter().find(|s| &s.id == id)
}

/// Sub Tool panel body.
pub fn body(app: &mut PrismApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let tool = app.ui.csp_tool;
    if !crate::csp_tools::active(app) {
        ui.small(tl!("Sub tools are listed in the Clip Studio workspace (Window › Workspace › Clip Studio)"));
        return;
    }
    ui.label(egui::RichText::new(tl!(tool.label())).strong());
    let subs = sub_tools(app, tool);
    if subs.is_empty() {
        ui.small(tl!("Import Clip Studio sub-tools (.sut) to fill this list"));
        return;
    }
    let selected = app.ui.csp_sub_tools.get(&tool).cloned();
    let mut pick = None;
    egui::ScrollArea::vertical().id_salt("csp-sub-tools").auto_shrink([false, true]).show(ui, |ui| {
        for sub in &subs {
            let width = ui.available_width();
            let (r, resp) = ui.allocate_exact_size(vec2(width, 24.0), if sub.usable { Sense::click() } else { Sense::hover() });
            let on = selected.as_deref() == Some(sub.id.as_str());
            if on {
                ui.painter().rect_filled(r, t.radius_sm, t.accent_soft);
            } else if resp.hovered() && sub.usable {
                ui.painter().rect_filled(r, t.radius_sm, t.hover);
            }
            let color = if sub.usable { t.text } else { t.text_faint };
            let name = if matches!(sub.source, Source::Builtin(_)) { tl!(&sub.name).to_string() } else { sub.name.clone() };
            ui.painter().text(r.left_center() + vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(12.5), color);
            let resp = resp.on_hover_text(if sub.usable {
                tl!(&sub.name).to_string()
            } else {
                tl!("This Clip Studio sub-tool is not supported in Prism yet").to_string()
            });
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, sub.usable, on, &sub.name));
            if resp.clicked() {
                pick = Some(sub.clone());
            }
        }
    });
    if let Some(sub) = pick {
        apply(app, tool, &sub);
    }
}

/// Tool Property panel body: an imported sub-tool's Clip Studio settings, else the options of
/// the Prism tool behind the sub-tool.
pub fn property_body(app: &mut PrismApp, ui: &mut egui::Ui) {
    if crate::sut_panel::active(app).is_some() && app.ui.tool.is_brushlike() && current(app).is_none_or(|s| matches!(s.source, Source::Preset(_))) {
        crate::sut_panel::show(app, ui);
        return;
    }
    let tool = app.ui.tool;
    let name = current(app).map(|s| if matches!(s.source, Source::Builtin(_)) { tl!(&s.name).to_string() } else { s.name });
    ui.label(egui::RichText::new(name.unwrap_or_else(|| tl!(tool.label()).to_string())).strong());
    // Imported fill and auto select sub-tools show CLIP STUDIO's own Tool Property.
    if let Some(SubTool { source: Source::Imported { preset, .. }, .. }) = current(app) {
        let fill = app.session.tools.presets.iter().find(|p| p.name == preset).and_then(|p| p.sut.as_ref()).and_then(crate::csp_fill_property::runs).is_some();
        if fill {
            crate::csp_fill_property::show(app, ui, &preset);
            if crate::enclose_fill_ui::active(app) {
                ui.small(tl!("Draw a lasso around the areas to fill"));
            }
            return;
        }
    }
    let o = &mut app.ui.tool_options;
    match tool {
        Tool::PaintBucket | Tool::MagicWand => {
            slider_row(ui, tl!("Tolerance"), egui::Slider::new(&mut o.tolerance, 0.0..=255.0).max_decimals(0));
            ui.checkbox(&mut o.contiguous, tl!("Contiguous"));
            ui.checkbox(&mut o.sample_all_layers, tl!("Refer to all layers"));
            ui.checkbox(&mut o.anti_alias, tl!("Anti-aliasing"));
        }
        Tool::Gradient => {
            ui.checkbox(&mut o.gradient_reverse, tl!("Reverse"));
            ui.checkbox(&mut o.gradient_dither, tl!("Dither"));
        }
        Tool::Type | Tool::VerticalType => {
            slider_row(ui, tl!("Size"), egui::Slider::new(&mut o.type_size, 1.0..=1296.0).logarithmic(true));
        }
        t if t.is_brushlike() => {
            let (mut size, mut opacity) = (app.session.tools.brush.size, app.session.tools.brush.opacity * 100.0);
            let a = slider_row(ui, tl!("Brush size"), egui::Slider::new(&mut size, 1.0..=prism_engine::paint::MAX_BRUSH_SIZE).logarithmic(true)).changed();
            let b = slider_row(ui, tl!("Opacity"), egui::Slider::new(&mut opacity, 0.0..=100.0).max_decimals(0)).changed();
            if a || b {
                let _ = app.run("tools.setBrush", json!({"brush": {"size": size, "opacity": opacity / 100.0}}));
            }
        }
        _ => {
            ui.small(tl!("This sub-tool has no settings"));
        }
    }
}

/// A labelled slider that fits the panel: the name on the left, the slider and its value box
/// in the rest of the row (a narrow dock column would otherwise clip a trailing label).
fn slider_row(ui: &mut egui::Ui, label: &str, slider: egui::Slider<'_>) -> egui::Response {
    ui.horizontal(|ui| {
        let w = (ui.available_width() * 0.32).clamp(48.0, 96.0);
        ui.add_sized([w, ui.spacing().interact_size.y], egui::Label::new(label).truncate());
        ui.spacing_mut().slider_width = (ui.available_width() - 64.0).max(40.0);
        ui.add(slider)
    })
    .inner
}

#[cfg(test)]
#[path = "sub_tool_panel_tests.rs"]
mod tests;
