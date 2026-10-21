//! Clip Studio enclose-and-fill sub-tools (SUT output operation 51) in the Clip Studio
//! workspace: they run on the Lasso, and closing the outline runs `paint.encloseFill` instead
//! of making a selection. Tolerance, reference layers and anti-aliasing are tool options (set
//! from the SUT by `sub_tool_panel::apply`); close gap and expansion come from the sub-tool's
//! settings, which `csp_tool_property` shows and edits.
use serde_json::{Value, json};

use crate::PrismApp;
use crate::csp_tools::CspTool;
use crate::state::Tool;
use crate::sub_tool_panel::Source;

/// The imported enclose-and-fill sub-tool in use, if the Lasso is running one.
pub fn preset(app: &PrismApp) -> Option<String> {
    if app.ui.tool != Tool::Lasso || app.ui.csp_tool != CspTool::Fill || !crate::csp_tools::active(app) {
        return None;
    }
    match crate::sub_tool_panel::current(app)?.source {
        Source::Imported { preset, tool: Tool::Lasso } => Some(preset),
        _ => None,
    }
}

pub fn active(app: &PrismApp) -> bool {
    preset(app).is_some()
}

/// Close gap and expansion params of `preset`. CLIP STUDIO converts the stored lengths to whole
/// pixels (rounded; a close gap at least 1) before filling; its units other than pixels are read
/// as pixels. See docs/csp-subtool-gaps.md.
pub fn sut_params(app: &PrismApp, preset: &str) -> Value {
    let params = app.session.tools.presets.iter().find(|p| p.name == preset).and_then(|p| p.sut.as_ref()).map(|m| m.parameters.clone()).unwrap_or_default();
    let get = |k: &str| params.get(k).copied().filter(|v| v.is_finite());
    let on = |k: &str| get(k).is_some_and(|v| v != 0.0);
    let max = f64::from(prism_algo::enclose_fill::MAX_DISTANCE);
    let close_gap = if on("FillUseCloseGap") { get("FillCloseGap").unwrap_or(0.0).round().clamp(1.0, max) } else { 0.0 };
    // The fill already covers the line art, so only a shrink changes it (any expand type).
    let expand = if on("FillUseExpand") { get("FillExpandLength").unwrap_or(0.0).round().clamp(-max, max) } else { 0.0 };
    json!({"closeGap": close_gap, "expand": expand})
}

/// Fill the enclosed areas inside the lasso outline `pts` (document coordinates).
pub fn finish(app: &mut PrismApp, pts: &[[f64; 2]]) {
    let Some(preset) = preset(app) else { return };
    if pts.len() < 3 {
        return;
    }
    let o = &app.ui.tool_options;
    let mut p = sut_params(app, &preset);
    p["points"] = json!(pts);
    p["tolerance"] = json!(o.tolerance);
    p["sampleAllLayers"] = json!(o.sample_all_layers);
    p["antiAlias"] = json!(o.anti_alias);
    p["opacity"] = json!(o.fill_opacity);
    p["target"] = crate::canvas::paint_target(app);
    match app.run("paint.encloseFill", p) {
        Ok(r) if r["filled"] == false => app.ui.status = tl!("No closed area inside the lasso to fill").to_string(),
        Ok(_) => {}
        Err(e) => app.ui.status = e,
    }
}

#[cfg(test)]
#[path = "enclose_fill_ui_tests.rs"]
mod tests;
