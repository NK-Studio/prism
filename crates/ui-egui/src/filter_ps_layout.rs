//! How each Photoshop filter dialog differs from the common layout of [`crate::filter_ps_ui`]:
//! options it hides, draws as radio buttons, without a slider, with an angle wheel, in a group or
//! on a shared row; dialogs without a preview; Photoshop's own names for options and choices.
//! Captured from Photoshop 27.4's dialogs (`docs/filter-analysis.md`).

/// How a choice is drawn as radio buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Radio {
    /// Inside a titled frame (Add Noise › Distribution).
    Framed,
    /// Under its label, one per line (Wind › Method).
    Listed,
    /// Side by side after its label (Extrude › Type).
    Row,
}

/// Options shown together in a titled frame. `header` is the title, or `@key` for a checkbox
/// option whose box is the title and which enables the rest (Oil Paint › Lighting).
pub struct Group {
    pub header: &'static str,
    pub keys: &'static [&'static str],
}

const RADIO: &[(&str, &str, Radio)] = &[
    ("filter.noise.addNoise", "distribution", Radio::Framed),
    ("filter.render.lensFlare", "lens", Radio::Listed),
    ("filter.video.deInterlace", "eliminate", Radio::Listed),
    ("filter.video.deInterlace", "createBy", Radio::Listed),
    ("filter.stylize.wind", "method", Radio::Listed),
    ("filter.stylize.wind", "direction", Radio::Listed),
    ("filter.stylize.diffuse", "mode", Radio::Framed),
    ("filter.stylize.traceContour", "edge", Radio::Framed),
    ("filter.stylize.tiles", "fill", Radio::Listed),
    ("filter.stylize.extrude", "type", Radio::Row),
    ("filter.stylize.extrude", "depthMode", Radio::Row),
    ("filter.distort.polarCoordinates", "mode", Radio::Listed),
    ("filter.distort.shear", "undefinedAreas", Radio::Listed),
    ("filter.blur.radialBlur", "method", Radio::Listed),
    ("filter.blur.radialBlur", "quality", Radio::Listed),
    ("filter.other.offset", "undefinedAreas", Radio::Framed),
    ("filter.other.hsbHsl", "inputMode", Radio::Listed),
    ("filter.other.hsbHsl", "rowOrder", Radio::Listed),
];

/// Params Photoshop's dialog does not show (they stay settable through the command). Every
/// `seed` is hidden too: Photoshop has no seed field.
const HIDDEN: &[(&str, &str)] = &[
    ("filter.render.lensFlare", "centerX"),
    ("filter.render.lensFlare", "centerY"),
    ("filter.blur.radialBlur", "centerX"),
    ("filter.blur.radialBlur", "centerY"),
];

/// Commands whose random `seed` Photoshop changes with a Randomize button.
const RANDOMIZE: &[&str] = &["filter.render.fibers"];

/// Dialogs without Photoshop's Preview checkbox (their preview is always on).
const NO_PREVIEW_TOGGLE: &[&str] = &[
    "filter.render.fibers",
    "filter.render.lensFlare",
    "filter.stylize.wind",
    "filter.distort.spherize",
    "filter.distort.polarCoordinates",
    "filter.distort.twirl",
    "filter.distort.pinch",
    "filter.distort.ripple",
    "filter.distort.zigZag",
    "filter.distort.wave",
    "filter.distort.shear",
    "filter.pixelate.mezzotint",
    "filter.pixelate.crystallize",
    "filter.pixelate.pointillize",
    "filter.blur.smartBlur",
];

/// Dialogs with Photoshop's small diagram of the distortion (a grid run through the filter) under
/// the buttons.
const DIAGRAM: &[&str] = &["filter.distort.spherize", "filter.distort.twirl", "filter.distort.pinch", "filter.distort.zigZag"];
/// Dialogs without a preview at all.
const NO_PREVIEW_PANE: &[&str] = &["filter.video.deInterlace", "filter.stylize.extrude", "filter.stylize.tiles", "filter.pixelate.colorHalftone", "filter.other.offset", "filter.blur.radialBlur", "filter.other.hsbHsl"];

/// Dialogs whose Preview checkbox Photoshop shows although they have no preview pane (the preview
/// is the canvas).
const PREVIEW_TOGGLE_ONLY: &[&str] = &["filter.other.offset"];

/// A line of text Photoshop puts above an option: (command, option, text).
const HEADING: &[(&str, &str, &str)] = &[("filter.pixelate.colorHalftone", "channel1", "Screen Angles (Degrees):")];

/// Numbers Photoshop types in a field without a slider.
const NO_SLIDER: &[(&str, &str)] = &[
    ("filter.stylize.extrude", "size"),
    ("filter.stylize.extrude", "depth"),
    ("filter.stylize.tiles", "count"),
    ("filter.stylize.tiles", "maxOffset"),
    ("filter.pixelate.colorHalftone", "maxRadius"),
    ("filter.pixelate.colorHalftone", "channel1"),
    ("filter.pixelate.colorHalftone", "channel2"),
    ("filter.pixelate.colorHalftone", "channel3"),
    ("filter.pixelate.colorHalftone", "channel4"),
];

/// Angles Photoshop sets with a wheel next to the field (and no slider).
const DIAL: &[(&str, &str)] = &[("filter.stylize.emboss", "angle"), ("filter.stylize.oilPaint", "angle"), ("filter.blur.motionBlur", "angle")];

/// Options drawn on the row of the option before them.
const JOIN: &[(&str, &str)] = &[("filter.stylize.extrude", "depthMode")];

const GROUPS: &[(&str, Group)] = &[
    ("filter.stylize.oilPaint", Group { header: "Brush", keys: &["stylization", "cleanliness", "scale", "bristleDetail"] }),
    ("filter.stylize.oilPaint", Group { header: "@lighting", keys: &["lighting", "angle", "shine"] }),
];

/// A point picked by clicking in the preview: (command, x param, y param), as canvas fractions.
const PICK_POINT: &[(&str, &str, &str)] = &[("filter.render.lensFlare", "centerX", "centerY")];

/// Photoshop's Blur Center box (Radial Blur): a square under the buttons, click to place the
/// centre: (command, x param, y param).
const CENTER_BOX: &[(&str, &str, &str)] = &[("filter.blur.radialBlur", "centerX", "centerY")];

/// Options Photoshop only enables while another option has a value: (command, option, other
/// option, value).
const ENABLED_WHEN: &[(&str, &str, &str, &str)] = &[("filter.sharpen.smartSharpen", "angle", "remove", "motionBlur")];

/// Numbers Photoshop shows as whole numbers (others keep one decimal).
const WHOLE: &[(&str, &str)] = &[
    ("filter.noise.median", "radius"),
    ("filter.noise.dustAndScratches", "radius"),
    ("filter.noise.dustAndScratches", "threshold"),
    ("filter.sharpen.unsharpMask", "amount"),
    ("filter.sharpen.unsharpMask", "threshold"),
    ("filter.sharpen.smartSharpen", "amount"),
    ("filter.sharpen.smartSharpen", "reduceNoise"),
    ("filter.sharpen.smartSharpen", "angle"),
    ("filter.render.fibers", "variance"),
    ("filter.render.fibers", "strength"),
    ("filter.render.lensFlare", "brightness"),
    ("filter.stylize.emboss", "angle"),
    ("filter.stylize.emboss", "height"),
    ("filter.stylize.emboss", "amount"),
    ("filter.stylize.traceContour", "level"),
    ("filter.stylize.tiles", "count"),
    ("filter.stylize.tiles", "maxOffset"),
    ("filter.stylize.extrude", "size"),
    ("filter.stylize.extrude", "depth"),
    ("filter.stylize.oilPaint", "angle"),
    ("filter.distort.twirl", "angle"),
    ("filter.distort.pinch", "amount"),
    ("filter.distort.spherize", "amount"),
    ("filter.distort.ripple", "amount"),
    ("filter.distort.zigZag", "amount"),
    ("filter.distort.zigZag", "ridges"),
    ("filter.distort.wave", "generators"),
    ("filter.distort.wave", "wavelengthMin"),
    ("filter.distort.wave", "wavelengthMax"),
    ("filter.distort.wave", "amplitudeMin"),
    ("filter.distort.wave", "amplitudeMax"),
    ("filter.pixelate.mosaic", "cellSize"),
    ("filter.pixelate.crystallize", "cellSize"),
    ("filter.pixelate.pointillize", "cellSize"),
    ("filter.pixelate.colorHalftone", "maxRadius"),
    ("filter.pixelate.colorHalftone", "channel1"),
    ("filter.pixelate.colorHalftone", "channel2"),
    ("filter.pixelate.colorHalftone", "channel3"),
    ("filter.pixelate.colorHalftone", "channel4"),
    ("filter.blur.radialBlur", "amount"),
    ("filter.blur.boxBlur", "radius"),
    ("filter.blur.surfaceBlur", "radius"),
    ("filter.blur.surfaceBlur", "threshold"),
    ("filter.blur.motionBlur", "angle"),
    ("filter.blur.motionBlur", "distance"),
    ("filter.blur.shapeBlur", "radius"),
    ("filter.other.minimum", "radius"),
    ("filter.other.maximum", "radius"),
];

/// Photoshop's English names where Prism's key reads differently: (command, key or choice, name).
/// The UI language's name comes from the catalogue's `filter` context.
const LABELS: &[(&str, &str, &str)] = &[
    ("filter.render.lensFlare", "lens", "Lens Type"),
    ("filter.render.lensFlare", "zoom", "50-300mm Zoom"),
    ("filter.render.lensFlare", "prime35", "35mm Prime"),
    ("filter.render.lensFlare", "prime105", "105mm Prime"),
    ("filter.render.lensFlare", "moviePrime", "Movie Prime"),
    ("filter.video.deInterlace", "createBy", "Create New Fields By"),
    ("filter.stylize.tiles", "count", "Number Of Tiles"),
    ("filter.stylize.tiles", "maxOffset", "Maximum Offset"),
    ("filter.stylize.tiles", "fill", "Fill Empty Area With"),
    ("filter.stylize.tiles", "background", "Background Color"),
    ("filter.stylize.tiles", "foreground", "Foreground Color"),
    ("filter.stylize.tiles", "inverse", "Inverse Image"),
    ("filter.stylize.tiles", "unaltered", "Unaltered Image"),
    ("filter.stylize.extrude", "levelBased", "Level-based"),
    ("filter.stylize.oilPaint", "lighting", "Lighting"),
    ("filter.distort.zigZag", "ridges", "Ridges"),
    ("filter.distort.zigZag", "style", "Style"),
    ("filter.pixelate.colorHalftone", "maxRadius", "Max. Radius"),
    ("filter.pixelate.colorHalftone", "channel1", "Channel 1"),
    ("filter.pixelate.colorHalftone", "channel2", "Channel 2"),
    ("filter.pixelate.colorHalftone", "channel3", "Channel 3"),
    ("filter.pixelate.colorHalftone", "channel4", "Channel 4"),
    ("filter.pixelate.mezzotint", "fineDots", "Fine Dots"),
    ("filter.pixelate.mezzotint", "mediumDots", "Medium Dots"),
    ("filter.pixelate.mezzotint", "grainyDots", "Grainy Dots"),
    ("filter.pixelate.mezzotint", "coarseDots", "Coarse Dots"),
    ("filter.pixelate.mezzotint", "shortLines", "Short Lines"),
    ("filter.pixelate.mezzotint", "mediumLines", "Medium Lines"),
    ("filter.pixelate.mezzotint", "longLines", "Long Lines"),
    ("filter.pixelate.mezzotint", "shortStrokes", "Short Strokes"),
    ("filter.pixelate.mezzotint", "mediumStrokes", "Medium Strokes"),
    ("filter.pixelate.mezzotint", "longStrokes", "Long Strokes"),
    ("filter.blur.radialBlur", "method", "Blur Method"),
    ("filter.blur.radialBlur", "zoom", "Zoom"),
    ("filter.blur.radialBlur", "draft", "Draft"),
    ("filter.blur.smartBlur", "edgeOnly", "Edge Only"),
    ("filter.blur.smartBlur", "overlayEdge", "Overlay Edge"),
    ("filter.other.offset", "horizontal", "Horizontal"),
    ("filter.other.offset", "vertical", "Vertical"),
    ("filter.other.offset", "transparent", "Set to Background"),
    ("filter.other.offset", "repeat", "Repeat Edge Pixels"),
    ("filter.other.offset", "wrap", "Wrap Around"),
    ("filter.other.hsbHsl", "rgb", "RGB"),
    ("filter.other.hsbHsl", "hsb", "HSB"),
    ("filter.other.hsbHsl", "hsl", "HSL"),
    ("filter.other.minimum", "squareness", "Squareness"),
    ("filter.other.maximum", "squareness", "Squareness"),
    ("filter.other.minimum", "roundness", "Roundness"),
    ("filter.other.maximum", "roundness", "Roundness"),
];

fn find<'a, T>(table: &'a [(&str, &str, T)], cmd: &str, key: &str) -> Option<&'a T> {
    table.iter().find(|(c, k, _)| *c == cmd && *k == key).map(|(_, _, v)| v)
}

fn listed(table: &[(&str, &str)], cmd: &str, key: &str) -> bool {
    table.iter().any(|(c, k)| *c == cmd && *k == key)
}

pub fn radio(cmd: &str, key: &str) -> Option<Radio> {
    find(RADIO, cmd, key).copied()
}

pub fn hidden(cmd: &str, key: &str) -> bool {
    key == "seed" || listed(HIDDEN, cmd, key)
}

pub fn randomizes(cmd: &str) -> bool {
    RANDOMIZE.contains(&cmd)
}

pub fn has_preview_pane(cmd: &str) -> bool {
    !NO_PREVIEW_PANE.contains(&cmd)
}

pub fn has_preview_toggle(cmd: &str) -> bool {
    PREVIEW_TOGGLE_ONLY.contains(&cmd) || (has_preview_pane(cmd) && !NO_PREVIEW_TOGGLE.contains(&cmd))
}

pub fn center_box(cmd: &str) -> Option<(&'static str, &'static str)> {
    CENTER_BOX.iter().find(|(c, _, _)| *c == cmd).map(|(_, x, y)| (*x, *y))
}

pub fn heading(cmd: &str, key: &str) -> Option<String> {
    find(HEADING, cmd, key).map(|t| crate::i18n::tr_ctx(crate::i18n::current(), "filter", t).to_string())
}

pub fn diagram(cmd: &str) -> bool {
    DIAGRAM.contains(&cmd)
}

pub fn slider(cmd: &str, key: &str) -> bool {
    !listed(NO_SLIDER, cmd, key) && !listed(DIAL, cmd, key)
}

pub fn dial(cmd: &str, key: &str) -> bool {
    listed(DIAL, cmd, key)
}

pub fn joins_previous(cmd: &str, key: &str) -> bool {
    listed(JOIN, cmd, key)
}

pub fn whole(cmd: &str, key: &str) -> bool {
    listed(WHOLE, cmd, key)
}

pub fn group_of(cmd: &str, key: &str) -> Option<&'static Group> {
    GROUPS.iter().find(|(c, g)| *c == cmd && (g.keys.contains(&key) || g.header.strip_prefix('@') == Some(key))).map(|(_, g)| g)
}

pub fn pick_point(cmd: &str) -> Option<(&'static str, &'static str)> {
    PICK_POINT.iter().find(|(c, _, _)| *c == cmd).map(|(_, x, y)| (*x, *y))
}

pub fn enabled_when(cmd: &str, key: &str) -> Option<(&'static str, &'static str)> {
    ENABLED_WHEN.iter().find(|(c, k, _, _)| *c == cmd && *k == key).map(|(_, _, o, v)| (*o, *v))
}

/// An option's or choice's name as Photoshop writes it, in the UI language.
pub fn label(cmd: &str, key: &str) -> String {
    let english = match find(LABELS, cmd, key) {
        Some(name) => (*name).to_string(),
        None => crate::filter_dialog::source_label(key),
    };
    crate::i18n::tr_ctx(crate::i18n::current(), "filter", &english).to_string()
}

/// The unit Photoshop writes after a number, in the UI language.
pub fn unit(cmd: &str, key: &str, max: f32) -> &'static str {
    match (cmd, key) {
        ("filter.stylize.tiles", "maxOffset") => "%",
        ("filter.stylize.extrude", "depth") | ("filter.stylize.tiles", "count") => "",
        ("filter.pixelate.mosaic", "cellSize") => tl!("square"),
        ("filter.pixelate.colorHalftone", "maxRadius") => tl!("(Pixels)"),
        ("filter.pixelate.colorHalftone", _) => "",
        ("filter.other.offset", "horizontal") => tl!("pixels right"),
        ("filter.other.offset", "vertical") => tl!("pixels down"),
        ("filter.blur.radialBlur", "amount") => "",
        (_, "threshold") => tl!("Levels"),
        ("filter.distort.twirl", "angle") => tl!("degrees"),
        (_, "angle") => "°",
        ("filter.distort.zigZag", "amount") => "",
        (_, "amount") if max <= 500.0 => "%",
        (_, "reduceNoise" | "brightness") => "%",
        (_, k) if crate::filter_dialog::is_pixel_param(k) => tl!("Pixels"),
        _ => "",
    }
}
