//! The Clip Studio workspace (Window › Workspace › Clip Studio): Clip Studio Paint's tools in the
//! Tools window, each with its sub-tools (the Sub Tool panel, `sub_tool_panel`). Every CSP tool
//! runs on an existing Prism tool; imported `.sut` sub-tools are placed under the CSP tool they
//! came from (their node icon, else their operation family). See docs/ui-design.md.
use egui::{Sense, Stroke, vec2};
use prism_engine::paint::sut::{SutBrushMetadata, SutToolKind};
use prism_engine::prefs::WorkspaceStyle;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::PrismApp;
use crate::icons;
use crate::state::Tool;
use crate::theme::Tokens;

/// Clip Studio Paint's tools, in its default Tools order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CspTool {
    Zoom,
    Hand,
    MoveLayer,
    Selection,
    AutoSelect,
    Eyedropper,
    #[default]
    Pen,
    Pencil,
    Brush,
    Airbrush,
    Decoration,
    Eraser,
    Blend,
    Fill,
    Gradient,
    Figure,
    Text,
}

/// The Tools window's sections in the Clip Studio workspace.
pub const SECTIONS: &[&[CspTool]] = &[
    &[CspTool::Zoom, CspTool::Hand, CspTool::MoveLayer],
    &[CspTool::Selection, CspTool::AutoSelect, CspTool::Eyedropper],
    &[CspTool::Pen, CspTool::Pencil, CspTool::Brush, CspTool::Airbrush, CspTool::Decoration, CspTool::Eraser, CspTool::Blend],
    &[CspTool::Fill, CspTool::Gradient, CspTool::Figure, CspTool::Text],
];

impl CspTool {
    pub const ALL: [CspTool; 17] = [
        CspTool::Zoom,
        CspTool::Hand,
        CspTool::MoveLayer,
        CspTool::Selection,
        CspTool::AutoSelect,
        CspTool::Eyedropper,
        CspTool::Pen,
        CspTool::Pencil,
        CspTool::Brush,
        CspTool::Airbrush,
        CspTool::Decoration,
        CspTool::Eraser,
        CspTool::Blend,
        CspTool::Fill,
        CspTool::Gradient,
        CspTool::Figure,
        CspTool::Text,
    ];

    /// English name (translated where drawn).
    pub fn label(self) -> &'static str {
        match self {
            CspTool::Zoom => "Zoom",
            CspTool::Hand => "Move",
            CspTool::MoveLayer => "Operation",
            CspTool::Selection => "Selection area",
            CspTool::AutoSelect => "Auto select",
            CspTool::Eyedropper => "Eyedropper",
            CspTool::Pen => "Pen",
            CspTool::Pencil => "Pencil",
            CspTool::Brush => "Brush",
            CspTool::Airbrush => "Airbrush",
            CspTool::Decoration => "Decoration",
            CspTool::Eraser => "Eraser",
            CspTool::Blend => "Blend",
            CspTool::Fill => "Fill",
            CspTool::Gradient => "Gradient",
            CspTool::Figure => "Figure",
            CspTool::Text => "Text",
        }
    }

    /// Clip Studio Paint's default single-key shortcut.
    pub fn key(self) -> char {
        match self {
            CspTool::Zoom => 'Z',
            CspTool::Hand => 'H',
            CspTool::MoveLayer => 'K',
            CspTool::Selection => 'M',
            CspTool::AutoSelect => 'W',
            CspTool::Eyedropper => 'I',
            CspTool::Pen | CspTool::Pencil => 'P',
            CspTool::Brush | CspTool::Airbrush | CspTool::Decoration => 'B',
            CspTool::Eraser => 'E',
            CspTool::Blend => 'J',
            CspTool::Fill | CspTool::Gradient => 'G',
            CspTool::Figure => 'U',
            CspTool::Text => 'T',
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            CspTool::Zoom => "zoom-in",
            CspTool::Hand => "hand",
            CspTool::MoveLayer => "move",
            CspTool::Selection => "square-dashed",
            CspTool::AutoSelect => "wand",
            CspTool::Eyedropper => "pipette",
            CspTool::Pen => "pen-tool",
            CspTool::Pencil => "pencil",
            CspTool::Brush => "brush",
            CspTool::Airbrush => "droplet",
            CspTool::Decoration => "sparkles",
            CspTool::Eraser => "eraser",
            CspTool::Blend => "blend",
            CspTool::Fill => "paint-bucket",
            CspTool::Gradient => "square-dot",
            CspTool::Figure => "pentagon",
            CspTool::Text => "type",
        }
    }

    /// The Prism tool that runs it before a sub-tool picks another.
    pub fn backing(self) -> Tool {
        match self {
            CspTool::Zoom => Tool::Zoom,
            CspTool::Hand => Tool::Hand,
            CspTool::MoveLayer => Tool::Move,
            CspTool::Selection => Tool::RectMarquee,
            CspTool::AutoSelect => Tool::MagicWand,
            CspTool::Eyedropper => Tool::Eyedropper,
            CspTool::Pen | CspTool::Pencil | CspTool::Brush | CspTool::Airbrush | CspTool::Decoration => Tool::Brush,
            CspTool::Eraser => Tool::Eraser,
            CspTool::Blend => Tool::Smudge,
            CspTool::Fill => Tool::PaintBucket,
            CspTool::Gradient => Tool::Gradient,
            CspTool::Figure => Tool::Rectangle,
            CspTool::Text => Tool::Type,
        }
    }

    /// Tools whose sub-tools are brushes (tools.setBrush presets).
    pub fn paints(self) -> bool {
        matches!(self, CspTool::Pen | CspTool::Pencil | CspTool::Brush | CspTool::Airbrush | CspTool::Decoration | CspTool::Eraser | CspTool::Blend)
    }
}

/// The CSP tool an imported sub-tool belongs to: its node icon when it is one of CSP's own
/// (`Settings/PAINT/Tool/*/EditImageTool.todb` groups them by icon), else its operation family.
pub fn placement(m: &SutBrushMetadata) -> Option<CspTool> {
    let by_icon = match m.node_icon {
        101 | 103 => Some(CspTool::Pen),
        102 | 113 => Some(CspTool::Pencil),
        104 | 110 | 112 => Some(CspTool::Brush),
        105 => Some(CspTool::Airbrush),
        106 => Some(CspTool::Eraser),
        109 | 114 => Some(CspTool::Blend),
        120..=130 => Some(CspTool::Decoration),
        // Lasso fill (216) sits among the figure icons.
        216 | 400 | 404..=407 => Some(CspTool::Fill),
        200..=205 => Some(CspTool::Selection),
        210..=218 => Some(CspTool::Figure),
        300 => Some(CspTool::Eyedropper),
        301 | 302 | 350 => Some(CspTool::MoveLayer),
        401 => Some(CspTool::AutoSelect),
        403 => Some(CspTool::Gradient),
        500 => Some(CspTool::Text),
        _ => None,
    };
    by_icon.or(match m.kind() {
        SutToolKind::Pen | SutToolKind::DotPen => Some(CspTool::Pen),
        SutToolKind::Brush | SutToolKind::Unknown => Some(CspTool::Brush),
        SutToolKind::Eraser => Some(CspTool::Eraser),
        SutToolKind::CopyStamp => Some(CspTool::Blend),
        SutToolKind::Selection => Some(CspTool::Selection),
        SutToolKind::Fill => Some(CspTool::Fill),
        SutToolKind::Gradient => Some(CspTool::Gradient),
        SutToolKind::Figure => Some(CspTool::Figure),
        SutToolKind::Text => Some(CspTool::Text),
        SutToolKind::Eyedropper => Some(CspTool::Eyedropper),
        SutToolKind::Operation => Some(CspTool::MoveLayer),
        SutToolKind::Comic | SutToolKind::CorrectLine | SutToolKind::Ruler | SutToolKind::View | SutToolKind::Other => None,
    })
}

/// Whether the workspace shows Clip Studio's tools.
pub fn active(app: &PrismApp) -> bool {
    crate::workspace_ui::style(app) == WorkspaceStyle::ClipStudio
}

/// Number of tool buttons (for the Tools window's one/two column choice).
pub fn slot_count() -> (usize, usize) {
    (SECTIONS.iter().map(|s| s.len()).sum(), SECTIONS.len())
}

/// Pick a CSP tool: its remembered sub-tool, or the first one, or its Prism tool.
pub fn select(app: &mut PrismApp, tool: CspTool) {
    app.ui.csp_tool = tool;
    let subs = crate::sub_tool_panel::sub_tools(app, tool);
    let remembered = app.ui.csp_sub_tools.get(&tool).cloned();
    let pick = remembered.and_then(|id| subs.iter().find(|s| s.id == id && s.usable).cloned()).or_else(|| subs.iter().find(|s| s.usable).cloned());
    match pick {
        Some(sub) => crate::sub_tool_panel::apply(app, tool, &sub),
        None => {
            app.ui.tool = tool.backing();
            crate::paint_mouse::sync_tool_brush(app);
        }
    }
}

/// CSP single-key shortcuts: a key shared by several tools cycles through them.
pub fn shortcut(app: &mut PrismApp, ctx: &egui::Context) -> bool {
    for tool in CspTool::ALL {
        let Some(k) = egui::Key::from_name(&tool.key().to_string()) else { continue };
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, k)) {
            let group: Vec<CspTool> = CspTool::ALL.iter().copied().filter(|t| t.key() == tool.key()).collect();
            let next = match group.iter().position(|t| *t == app.ui.csp_tool) {
                Some(i) => group.get((i + 1) % group.len()).copied().unwrap_or(tool),
                None => tool,
            };
            select(app, next);
            return true;
        }
    }
    false
}

/// The Tools window's buttons in the Clip Studio workspace (same look as Photoshop's slots,
/// one CSP tool per button, no flyouts: sub-tools live in the Sub Tool panel).
pub fn toolbar_slots(app: &mut PrismApp, ui: &mut egui::Ui, double: bool, bx: f32) {
    let t = Tokens::get(ui.ctx());
    ui.spacing_mut().item_spacing = vec2(2.0, 3.0);
    let sections: Vec<Vec<CspTool>> =
        if t.pro { vec![SECTIONS.iter().flat_map(|s| s.iter().copied()).collect()] } else { SECTIONS.iter().map(|s| s.to_vec()).collect() };
    for (si, section) in sections.iter().enumerate() {
        if si > 0 && !t.pro {
            ui.add_space(4.0);
            let (r, _) = ui.allocate_exact_size(vec2(if double { bx * 2.0 + 2.0 } else { bx }, 1.0), Sense::hover());
            ui.painter().line_segment([r.left_center() + vec2(8.0, 0.0), r.right_center() - vec2(8.0, 0.0)], Stroke::new(1.0, t.separator));
            ui.add_space(4.0);
        }
        for row in section.chunks(if double { 2 } else { 1 }) {
            ui.horizontal(|ui| {
                for &tool in row {
                    let tip = format!("{}  ({})", tl!(tool.label()), tool.key());
                    let on = app.ui.csp_tool == tool;
                    let resp = icons::button(ui, tool.icon(), bx, on, &tip);
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, tl!(tool.label())));
                    if resp.clicked() {
                        select(app, tool);
                    }
                }
            });
        }
    }
}

/// Set up the Clip Studio workspace's panels: Sub Tool above Tool Property in a left column.
pub fn apply_workspace_layout(app: &mut PrismApp) {
    use crate::dock_model::{PanelId, Side};
    app.ui.dock.add_column(Side::Left, vec![vec![PanelId::SubTool], vec![PanelId::ToolProperty]], 250.0);
    let tool = app.ui.csp_tool;
    select(app, tool);
}

/// `ui.set {cspTool}` from the control channel.
pub fn set_from_name(app: &mut PrismApp, name: &str) -> Result<serde_json::Value, String> {
    let tool = CspTool::ALL
        .into_iter()
        .find(|t| {
            t.label().eq_ignore_ascii_case(name)
                || serde_json::to_value(t).ok().and_then(|v| v.as_str().map(str::to_owned)).is_some_and(|k| k.eq_ignore_ascii_case(name))
        })
        .ok_or_else(|| format!("unknown Clip Studio tool `{name}`"))?;
    select(app, tool);
    Ok(json!({"cspTool": tool, "tool": format!("{:?}", app.ui.tool)}))
}

#[cfg(test)]
#[path = "csp_tools_tests.rs"]
mod tests;
