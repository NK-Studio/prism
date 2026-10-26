//! CLIP STUDIO PAINT's Tool Property items for imported sub-tools: for each property ID a
//! sub-tool lists in `VariantShowParam`, the sub-tool settings (column keys) it edits, the kind of
//! control CLIP STUDIO draws, its range and unit, and CLIP STUDIO's English label (the UI
//! translates it with CLIP STUDIO's own terms). Brush-type sub-tools keep the brush Tool
//! Property, which edits the brush itself; this catalogue covers the items the other families
//! show (fill, auto select, selection, gradient, figure, text, eyedropper, operation), including
//! the brush items they reuse. Evidence and the per-family list: docs/csp-subtool-gaps.md.
//!
//! IDs and keys come from CLIP STUDIO's settings writers (each reads property *n* and stores it
//! under key *k*). Composite items without a writer key (3600, 3700–3702, 5401/5402, 1900, 7002)
//! are mapped from the columns the default sub-tools store and the labels CLIP STUDIO shows;
//! those inferences are marked on the item.

/// The unit a value is shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    None,
    Percent,
    /// A length; when the item's unit key is not 0 (pixels) CLIP STUDIO converts it.
    Px,
    Degree,
}

impl Unit {
    pub fn suffix(self) -> &'static str {
        match self {
            Unit::None => "",
            Unit::Percent => " %",
            Unit::Px => " px",
            Unit::Degree => "°",
        }
    }
}

/// How CLIP STUDIO draws a setting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// A number in `min..=max` (`int`: whole numbers; `log`: a logarithmic bar).
    Slider { min: f64, max: f64, unit: Unit, int: bool, log: bool },
    /// On (1) or off (0).
    Check,
    /// One of the listed codes.
    List(&'static [(i64, &'static str)]),
    /// One of the listed codes as icon buttons: (code, label, Prism icon name).
    Icons(&'static [(i64, &'static str, &'static str)]),
    /// CLIP STUDIO's five-step indicator over a slider range (Close gap).
    Indicator { steps: &'static [f64], min: f64, max: f64, unit: Unit },
    /// A colour: this field's key holds red, `green` and `blue` the others, each a 32-bit channel
    /// (0–4294967295).
    Color { green: &'static str, blue: &'static str },
    /// A code whose meanings are not traced: shown and edited as a whole number.
    Code { max: i64 },
    /// A font family name (text); `unique` holds CLIP STUDIO's unique name of the same font.
    Font { unique: Option<&'static str> },
    /// The gradient's stops (`SutBrushMetadata::gradient`).
    Gradient,
    /// Not a number (a curve, a target list): not imported.
    Data,
}

/// One stored setting of an item.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    pub key: &'static str,
    /// Its own label; empty uses the item's.
    pub label: &'static str,
    pub kind: Kind,
}

/// One Tool Property item.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Item {
    pub id: u32,
    pub label: &'static str,
    /// Translation context for labels CLIP STUDIO translates differently in this item ("" for the
    /// shared Tool Property terms).
    pub ctx: &'static str,
    /// A checkbox (labelled with the item's label) that switches the fields on.
    pub toggle: Option<&'static str>,
    pub fields: &'static [Field],
    /// The unit key of the item's length (0 = pixels).
    pub unit_key: Option<&'static str>,
    /// Items drawn inside this one (indented) when the sub-tool shows them.
    pub children: &'static [u32],
    /// Items that belong to this one's control and have no row of their own.
    pub parts: &'static [u32],
}

/// The largest setting value kept (32-bit colour channels and flag sets reach 2³²−1).
pub const MAX_SETTING_VALUE: f64 = 4_294_967_295.0;

const BASE: Item = Item { id: 0, label: "", ctx: "", toggle: None, fields: &[], unit_key: None, children: &[], parts: &[] };

const fn f(key: &'static str, label: &'static str, kind: Kind) -> Field {
    Field { key, label, kind }
}
const fn slider(min: f64, max: f64, unit: Unit) -> Kind {
    Kind::Slider { min, max, unit, int: false, log: false }
}
const fn whole(min: f64, max: f64, unit: Unit) -> Kind {
    Kind::Slider { min, max, unit, int: true, log: false }
}
const fn log(min: f64, max: f64, unit: Unit) -> Kind {
    Kind::Slider { min, max, unit, int: false, log: true }
}

/// An item with one field labelled by the item.
macro_rules! one {
    ($id:expr, $label:expr, $key:expr, $kind:expr $(, $name:ident: $val:expr)* $(,)?) => {
        Item { id: $id, label: $label, fields: &[f($key, "", $kind)] $(, $name: $val)*, ..BASE }
    };
}
/// An item with several fields.
macro_rules! many {
    ($id:expr, $label:expr, [$($field:expr),* $(,)?] $(, $name:ident: $val:expr)* $(,)?) => {
        Item { id: $id, label: $label, fields: &[$($field),*] $(, $name: $val)*, ..BASE }
    };
}

const ANTI_ALIAS: &[(i64, &str)] = &[(0, "None"), (1, "Weak"), (2, "Medium"), (3, "Strong")];
const SNAP_ANGLES: &[(i64, &str)] = &[(1, "1°"), (5, "5°"), (10, "10°"), (15, "15°"), (30, "30°"), (45, "45°"), (60, "60°"), (90, "90°")];
const CURVES: &[(i64, &str)] = &[(0, "Straight line"), (1, "Spline"), (2, "Quadratic Bezier"), (3, "Cubic Bezier")];
const LENGTH_OR_PERCENT: &[(i64, &str)] = &[(0, "Specify length"), (1, "By percentage")];
/// CLIP STUDIO's default Close gap indicator steps, in pixels.
pub const CLOSE_GAP_STEPS: [f64; 5] = [1.0, 3.0, 5.0, 10.0, 20.0];

/// CLIP STUDIO's Gap types (`BrushAutoIntervalType`), in its button order.
const GAPS: &[(i64, &str)] = &[(0, "Fixed"), (1, "Wide"), (2, "Normal"), (3, "Narrow")];
/// Every catalogued item, in no particular order (a sub-tool's own list gives the order).
pub const ITEMS: &[Item] = &[
    // Common.
    one!(10, "Opacity", "Opacity", whole(0.0, 100.0, Unit::Percent)),
    one!(20, "Anti-aliasing", "AntiAlias", Kind::List(ANTI_ALIAS)),
    // CLIP STUDIO's blending mode codes beyond Normal (0) are not traced.
    one!(30, "Blending mode", "CompositeMode", Kind::Code { max: 255 }),
    one!(40, "Stabilization", "FlickerReduction", whole(0.0, 100.0, Unit::None)),
    one!(41, "Adjust by speed", "FlickerReductionBySpeed", Kind::Check),
    one!(
        42,
        "Stabilization mode",
        "FlickerReductionBySpeedType",
        Kind::List(&[(0, "Reduce stabilization when drawing quickly"), (1, "Increase stabilization when drawing slowly")])
    ),
    // Brush items figures and the selection pen show.
    one!(1000, "Brush Size", "BrushSize", log(0.1, 2000.0, Unit::Px), unit_key: Some("BrushSizeUnit")),
    one!(1002, "Specify by size on screen", "BrushSizeSyncViewScale", Kind::Check),
    one!(1020, "Brush density", "BrushFlow", whole(0.0, 100.0, Unit::Percent)),
    one!(1030, "Hardness", "BrushHardness", whole(0.0, 100.0, Unit::Percent)),
    many!(
        1160,
        "Starting and ending",
        [f("BrushInOutType", "How to specify", Kind::List(&[(0, "Specify length"), (1, "By percentage"), (2, "Fade")])), f("BrushInOutTarget", "Effect", Kind::Data)],
        parts: &[1162],
    ),
    one!(1171, "Starting", "BrushInLength", slider(0.0, 1000.0, Unit::Px), unit_key: Some("BrushInLengthUnit")),
    one!(1181, "Ending", "BrushOutLength", slider(0.0, 1000.0, Unit::Px), unit_key: Some("BrushOutLengthUnit")),
    one!(1190, "Sharp angles", "BrushSharpenCorner", Kind::Check),
    one!(1241, "Vector magnet", "BrushVectorMagnetPower", whole(0.0, 100.0, Unit::None)),
    // The figure's brush shape (a preset picker); its settings are the brush items above.
    many!(1900, "Brush shape", []),
    // Selection.
    one!(
        3000,
        "Selection mode",
        "SelectCreateType",
        Kind::Icons(&[(0, "New selection", "square"), (1, "Add to selection", "plus"), (2, "Remove from selection", "minus"), (3, "Select from selection", "squares-subtract")]),
        ctx: "cspTool.select",
    ),
    one!(3010, "Interlock selection and image", "SelectMoveImage", Kind::Check),
    // Figure and selection shapes.
    one!(3200, "Line/Fill", "ShapeStrokeAndFillType", Kind::List(&[(0, "Create fill"), (1, "Create line"), (2, "Create both line and fill")])),
    one!(3212, "Adjust angle after fixed", "RotateShape", Kind::Check),
    one!(3220, "Start from center", "ShapeStartCenter", Kind::Check),
    one!(3231, "Snap angle", "LineLockAngleStep", Kind::List(SNAP_ANGLES), toggle: Some("LineLockAngle")),
    one!(3211, "Snap angle", "ShapeLockAngleStep", Kind::List(SNAP_ANGLES), toggle: Some("ShapeLockAngle")),
    one!(3240, "Enable snapping", "ShapeEnableSnap", Kind::Check),
    one!(3300, "Figure", "FixedShapeKind", Kind::List(&[(0, "Rectangle"), (1, "Ellipse"), (2, "Polygon")])),
    one!(3301, "Number of corners", "FixedShapeVertexNumber", whole(3.0, 128.0, Unit::None)),
    one!(3310, "Aspect type", "FixedShapeAspectType", Kind::List(&[(0, "Free"), (1, "Specified ratio"), (2, "Specify length")])),
    one!(3311, "W", "FixedShapeWidth", log(0.1, 10000.0, Unit::Px), unit_key: Some("FixedShapeWidthUnit")),
    one!(3312, "H", "FixedShapeHeight", log(0.1, 10000.0, Unit::Px), unit_key: Some("FixedShapeHeightUnit")),
    one!(3321, "How to specify", "FixedShapeRoundValueType", Kind::List(LENGTH_OR_PERCENT)),
    one!(
        3322,
        "Roundness of corner",
        "FixedShapeRoundSize",
        slider(0.0, 1000.0, Unit::Px),
        toggle: Some("FixedShapeUseRoundCorner"),
        unit_key: Some("FixedShapeRoundSizeUnit"),
    ),
    one!(3400, "Curve", "LineCurveKind", Kind::List(CURVES)),
    one!(3500, "Curve", "PolylineCurveKind", Kind::List(CURVES)),
    one!(3510, "Close line", "PolylineClose", Kind::Check),
    one!(3520, "How to specify", "PolylineDesignationType", Kind::List(&[(0, "By click"), (1, "By drag")])),
    one!(3521, "Show line preview", "PolylineShowPrediction", Kind::Check),
    one!(3530, "Add/delete control points while drawing", "PolylineAddAndDeletePoint", Kind::Check),
    // Inferred: the lasso's stabilization group (the columns lasso tools store under it).
    many!(3600, "Stabilization", [f("FlickerReduction", "", whole(0.0, 100.0, Unit::None)), f("FlickerReductionBySpeed", "Adjust by speed", Kind::Check)]),
    // Inferred: the lasso groups, from the Select* columns and the labels in this order.
    one!(3700, "Lasso selection mode", "SelectOperationMethod", Kind::List(&[(0, "Single stroke"), (1, "Multiple input (freehand and lasso)")])),
    one!(3701, "Magnetic lasso", "SelectSnapSize", log(1.0, 1000.0, Unit::Px), toggle: Some("SelectUseSnap"), unit_key: Some("SelectSnapSizeUnit")),
    many!(
        3702,
        "Magnet settings",
        [
            f("SelectSnapOffCondition", "", Kind::List(&[(0, "All"), (1, "Freehand only"), (2, "Polyline only")])),
            f("SelectSnapToVectorCenter", "Snap to vector paths", Kind::Check),
        ]
    ),
    // Operation: move layer, select layer, object.
    one!(
        4001,
        "Mode",
        "LayerMoveTarget",
        Kind::List(&[(0, "Layer"), (1, "Halftone dots of tone"), (2, "Rotate the halftone dots of tone"), (3, "Grid/Ruler Bar")]),
        ctx: "cspTool.move",
    ),
    one!(4010, "Move the object at the clicked position", "LayerMoveClicked", Kind::Check),
    one!(4011, "Move layers in the selected area", "LayerMoveInSelectArea", Kind::Check),
    one!(4020, "Moved layers become selected", "LayerMoveSelectLayer", Kind::Check),
    one!(4030, "Keep original image", "LayerMoveRemainOriginal", Kind::Check),
    one!(4200, "Exclude text", "SelectLayerNotSelectText", Kind::Check, ctx: "cspTool.select"),
    one!(4201, "Exclude draft layers", "SelectLayerNotSelectDraft", Kind::Check, ctx: "cspTool.select"),
    one!(4202, "Exclude locked layers", "SelectLayerNotSelectLocked", Kind::Check, ctx: "cspTool.select"),
    one!(4204, "Exclude fill layers", "SelectLayerNotSelectMonochromeFill", Kind::Check, ctx: "cspTool.select"),
    // Selectable object is a set of flags (2³¹−1: every kind); the flag meanings are not traced.
    one!(4501, "Selectable object", "ObjectSelectable", Kind::Code { max: 4_294_967_295 }),
    // Inferred: the object tool's first item, Operation of transparent part.
    one!(4521, "Operation of transparent part", "ObjectOperationAtBlank", Kind::Code { max: 16 }),
    one!(4600, "Snap to other frame borders", "ObjectComicFrameSnap", Kind::Check),
    one!(4601, "Keep gutters aligned", "ObjectComicFrameLink", Kind::List(&[(0, "None"), (1, "Horizontal & adjacent"), (2, "All")]), ctx: "cspTool.frame"),
    one!(4650, "Change thickness while scaling", "ObjectVectorChangeWidth", Kind::Check),
    one!(4651, "Mode", "ObjectVectorOperation", Kind::Code { max: 16 }),
    one!(4652, "Mode", "ObjectResizableTransformOperation", Kind::Code { max: 16 }),
    // Eyedropper.
    one!(4100, "Reference", "SpuitReferType", Kind::List(&[(0, "Current Layer"), (1, "Top layer"), (2, "Image")])),
    one!(4110, "Average color", "SpuitAroundPixel", slider(1.0, 100.0, Unit::Px), toggle: Some("SpuitUseAroundPixel"), unit_key: Some("SpuitAroundPixelUnit")),
    one!(4130, "Exclude text", "SpuitNotReferText", Kind::Check),
    one!(4131, "Exclude draft layers", "SpuitNotReferDraft", Kind::Check),
    one!(4132, "Exclude locked layers", "SpuitNotReferLocked", Kind::Check),
    one!(4133, "Exclude paper layer", "SpuitNotReferPaper", Kind::Check),
    one!(4140, "Show color picker circle", "SpuitShowColorCursorByPen", Kind::Check),
    // Fill and auto select.
    one!(
        5001,
        "Refer multiple",
        "FillReferLayerType",
        Kind::Icons(&[(0, "All layers", "layers"), (1, "Reference layer", "compass"), (2, "Selected layer", "check"), (3, "Layer in folder", "folder")]),
        toggle: Some("FillReferMultiLayer"),
        children: &[5010, 5012, 5013, 5014, 5015, 5020, 5021, 5030, 5040],
    ),
    one!(5010, "Exclude text", "FillNotReferText", Kind::Check),
    one!(5012, "Exclude draft layers", "FillNotReferDraft", Kind::Check),
    one!(5013, "Exclude editing layer", "FillNotReferCurrent", Kind::Check),
    one!(5014, "Exclude paper layer", "FillNotReferPaper", Kind::Check),
    one!(5015, "Exclude locked layers", "FillNotReferLocked", Kind::Check),
    one!(5020, "Fill up to vector path", "FillReferVectorCenter", Kind::Check),
    one!(5021, "Include vector path", "FillIncludeVectorCenter", Kind::Check),
    one!(5030, "Refer to image border", "FillReferFrame", Kind::Check),
    one!(5040, "Reference overflow frame", "FillReferDrawFrame", Kind::Check),
    one!(
        5101,
        "Close gap",
        "FillCloseGap",
        Kind::Indicator { steps: &CLOSE_GAP_STEPS, min: 0.1, max: 50.0, unit: Unit::Px },
        toggle: Some("FillUseCloseGap"),
        unit_key: Some("FillCloseGapUnit"),
    ),
    one!(5102, "Fill narrow areas", "FillSoakIntoNarrowArea", Kind::Check),
    one!(
        5111,
        "Area scaling",
        "FillExpandLength",
        slider(-50.0, 50.0, Unit::Px),
        toggle: Some("FillUseExpand"),
        unit_key: Some("FillExpandLengthUnit"),
    ),
    one!(5112, "Scaling mode", "FillExpandType", Kind::List(&[(0, "Rectangle"), (1, "Round"), (2, "To darkest pixel")]), ctx: "cspTool.scale"),
    one!(5120, "Tolerance", "FillColorMargin", slider(0.0, 100.0, Unit::None)),
    one!(5130, "Anti-aliasing", "FillAntiAlias", Kind::Check),
    one!(5200, "Apply to connected pixels only", "FillContinuousPixel", Kind::Check),
    one!(
        5300,
        "Target color",
        "CloseFillReferColorType",
        Kind::List(&[
            (0, "Target all colors"),
            (1, "Only transparent"),
            (2, "Area surrounded by transparent"),
            (3, "Only black"),
            (4, "Area surrounded by black"),
            (5, "Only white and transparent"),
            (6, "Area surrounded by white and transparent"),
            (7, "Treat semi-transparent as transparent"),
            (8, "All enclosed areas except transparency"),
            (9, "All enclosed areas including transparency"),
        ])
    ),
    one!(5500, "Dust size", "DustSize", log(0.1, 100.0, Unit::Px), unit_key: Some("DustSizeUnit")),
    one!(
        5510,
        "Mode",
        "DustMode",
        Kind::List(&[
            (0, "Remove dust from transparency"),
            (1, "Remove dust from white background"),
            (2, "Fill transparent gaps with surrounding color"),
            (3, "Fill transparent gaps with foreground color"),
        ])
    ),
    one!(5511, "Mode", "DustModeForSelect", Kind::List(&[(0, "Select dust on transparency"), (1, "Select dust on white background"), (2, "Select transparent hole")])),
    one!(5600, "Black", "ContourFillUseBlack", Kind::List(&[(0, "Do not cover black"), (1, "Cover black"), (2, "Replace black with drawing color")])),
    one!(
        5601,
        "Transparent color",
        "ContourFillUseTrans",
        Kind::List(&[(0, "Do not cover transparency"), (1, "Cover transparency"), (2, "Replace transparency with drawing color")])
    ),
    one!(5602, "Output Settings", "ContourFillBlendGraph", Kind::Data),
    // Gradient. 5401 and 5402 are the gradient bar's colour and stop editors.
    many!(5400, "Gradient", [f("GradationData", "", Kind::Gradient), f("GradationReverseSaved", "Flip", Kind::Check)], parts: &[5401, 5402]),
    one!(5410, "Shape", "GradationShapeKind", Kind::List(&[(0, "Straight line"), (1, "Circle"), (2, "Ellipse")])),
    one!(5420, "Edge process", "GradationRepeatType", Kind::List(&[(0, "Do not repeat"), (1, "Repeat"), (2, "Reverse"), (3, "Not draw")])),
    one!(5430, "Where to create", "GradationMakeLayer", Kind::List(&[(0, "Draw on editing layer"), (1, "Create gradient layer")])),
    one!(5440, "Dithering", "GradationDither", Kind::Check),
    // Text.
    one!(7000, "Font", "TextFontName", Kind::Font { unique: Some("TextFontUniqueName") }),
    one!(7001, "Size", "TextFontSize", log(0.1, 1000.0, Unit::None)),
    // Inferred: Style holds the font style flags (bold, italic) as one value.
    one!(7002, "Style", "TextFontStyle", Kind::Code { max: 255 }),
    one!(7003, "Text direction", "TextVertical", Kind::List(&[(0, "Horizontal"), (1, "Vertical")])),
    many!(
        7004,
        "Alignment",
        [f("TextAlignment", "", Kind::List(&[(0, "Left/Top"), (1, "Center"), (2, "Right/Bottom")])), f("TextAlignmentJustify", "Justify", Kind::Code { max: 8 })]
    ),
    one!(7005, "Line space", "TextLineSpace", slider(0.0, 1000.0, Unit::None)),
    one!(7006, "Auto TateChuYoko (Horizontal in Vertical)", "TextTateChuYoko", whole(0.0, 4.0, Unit::None)),
    one!(7007, "Reading size (%)", "TextRubySize", whole(1.0, 100.0, Unit::Percent)),
    one!(7009, "Character spacing", "TextWordSpace", slider(-100.0, 1000.0, Unit::None)),
    one!(7010, "Show resize handle", "TextEnableHandle", Kind::Check),
    one!(7011, "Display quality while editing", "TextFastPreview", Kind::List(&[(0, "High quality"), (1, "Fast")])),
    one!(7012, "How to add", "TextInsertType", Kind::List(&[(0, "Create layer always"), (1, "Add to selected text"), (2, "Detect position")]), ctx: "cspTool.text"),
    one!(7013, "Outline", "TextOutline", Kind::Code { max: 8 }, toggle: Some("TextOutlineEnable")),
    one!(7014, "Reading font", "TextRubyFontName", Kind::Font { unique: None }),
    one!(7015, "Reading position", "TextRubyAlignment", Kind::List(&[(0, "Align to start"), (1, "Align to center"), (2, "Align to end")])),
    one!(7016, "Adjust reading", "TextRubyPosition", slider(-1000.0, 1000.0, Unit::None)),
    one!(7017, "Space between reading and main text", "TextRubyDistance", slider(-1000.0, 1000.0, Unit::None)),
    one!(7018, "Reading space", "TextRubyWordSpaceJustify", Kind::Code { max: 8 }),
    one!(7019, "Character spacing", "TextRubyWordSpace", slider(-100.0, 1000.0, Unit::None)),
    one!(7021, "How to specify", "TextLineSpaceType", Kind::List(LENGTH_OR_PERCENT)),
    one!(7024, "Anti-aliasing", "TextAntiAlias", Kind::List(&[(0, "Canvas setting"), (1, "On"), (2, "Off")])),
    many!(
        7025,
        "Text color",
        [f("TextColorType", "Type", Kind::Code { max: 8 }), f("TextColorRed", "Color", Kind::Color { green: "TextColorGreen", blue: "TextColorBlue" })]
    ),
    one!(7027, "Horizontal ratio", "TextHorizontalRatio", slider(1.0, 1000.0, Unit::Percent)),
    one!(7028, "Vertical ratio", "TextVerticalRatio", slider(1.0, 1000.0, Unit::Percent)),
    one!(7029, "Condense text", "TextTsume", slider(0.0, 100.0, Unit::None)),
    one!(7031, "Use half-width punctuation marks", "TextAlternateHalfMetrics", Kind::Check),
    one!(7032, "Wrap text at frame", "TextWrapping", Kind::Check),
    one!(
        7034,
        "Background color",
        "TextBackgroundColorRed",
        Kind::Color { green: "TextBackgroundColorGreen", blue: "TextBackgroundColorBlue" },
        toggle: Some("TextBackgroundEnabled"),
    ),
    one!(7035, "Opacity", "TextBackgroundAlpha", whole(0.0, 100.0, Unit::Percent)),
    one!(7036, "Position in frame", "TextTextAlignment", Kind::Code { max: 8 }),
    many!(
        7038,
        "Edge",
        [f("TextEdgeWidth", "Thickness of edges", slider(0.0, 100.0, Unit::Px))],
        toggle: Some("TextEdgeEnabled"),
        unit_key: Some("TextEdgeWidthUnit"),
    ),
    one!(7039, "Edge color", "TextEdgeColorRed", Kind::Color { green: "TextEdgeColorGreen", blue: "TextEdgeColorBlue" }),
    one!(7040, "Rotation angle", "TextRotation", slider(-360.0, 360.0, Unit::Degree)),
    one!(7041, "Skew (horizontal)", "TextHorizontalSkewAngle", slider(-89.0, 89.0, Unit::Degree)),
    one!(7042, "Skew (vertical)", "TextVerticalSkewAngle", slider(-89.0, 89.0, Unit::Degree)),
    one!(
        7043,
        "Mode",
        "TextTransformType",
        Kind::List(&[(0, "Scale/Rotate"), (1, "Scale"), (2, "Rotate"), (3, "Skew"), (4, "Scale/Rotate/Skew")]),
        ctx: "cspTool.transform",
    ),
    one!(7044, "Keep aspect ratio", "TextFixAspect", Kind::Check),
    many!(7045, "Flip", [f("TextHorizontalReverse", "Horizontal", Kind::Check), f("TextVerticalReverse", "Vertical", Kind::Check)], ctx: "cspTool.text"),
    one!(7051, "Drag", "TextDragOperationType", Kind::List(&[(0, "Create text box"), (1, "Select text")])),
    one!(7052, "Apply to", "TextToolPropertyKind", Kind::List(&[(0, "None"), (1, "New only"), (2, "Selected text")])),
    one!(7053, "Selection mode", "TextSelectType", Kind::List(&[(0, "New selection"), (1, "Add to selection"), (2, "Toggle selection")])),
    one!(7054, "Version", "TextVersion", Kind::Code { max: 64 }),
    one!(7055, "Ligatures", "TextStandardLigature", Kind::Check),
    // Brush Tool Property items without a dedicated brush panel control (the panel draws the
    // ones it has its own control for). The values the brush runs are applied by
    // `crate::sut::apply_brush_setting`.
    one!(1020, "Brush density", "BrushFlow", whole(0.0, 100.0, Unit::Percent)),
    one!(1030, "Hardness", "BrushHardness", whole(0.0, 100.0, Unit::Percent)),
    one!(1040, "Gap", "BrushInterval", log(1.0, 1000.0, Unit::Percent)),
    // The Gap type; its fixed size (1040) is drawn indented under it, as CLIP STUDIO expands it.
    one!(1042, "Gap", "BrushAutoIntervalType", Kind::List(GAPS), ctx: "cspTool.gap", children: &[1040]),
    one!(1060, "Direction", "BrushRotation", slider(-360.0, 360.0, Unit::Degree), ctx: "cspTool.tip"),
    one!(1130, "Spray", "BrushUseSpray", Kind::Check),
    one!(1131, "Particle size", "BrushSpraySize", log(0.5, 2000.0, Unit::Px), unit_key: Some("BrushSpraySizeUnit")),
    one!(1133, "Particle density", "BrushSprayDensity", whole(1.0, 100.0, Unit::None)),
    one!(1135, "Spray deviation", "BrushSprayBias", whole(0.0, 100.0, Unit::None)),
    one!(7057, "Kerning", "TextKerningMethod", Kind::List(&[(0, "None"), (1, "Manual"), (2, "Metrics")])),
    one!(7059, "Contextual alternates", "TextContextualAlternate", Kind::Check),
    one!(7060, "Circular text", "TextLayoutCircularEnabled", Kind::Check),
    one!(7061, "Radius", "TextLayoutCircularRadius", log(1.0, 10000.0, Unit::None)),
    one!(7062, "Direction", "TextLayoutCircularDirection", Kind::List(&[(0, "Clockwise"), (1, "Counter-clockwise")])),
    one!(7063, "Space evenly", "TextLayoutCircularJustify", Kind::Check),
];

/// The catalogued item `id` (5000, Refer multiple's switch, is drawn with 5001).
pub fn item(id: u32) -> Option<&'static Item> {
    let id = if id == 5000 { 5001 } else { id };
    ITEMS.iter().find(|i| i.id == id)
}

/// The item that draws `id` inside itself (a child or a part), if any.
pub fn owner(id: u32) -> Option<&'static Item> {
    ITEMS.iter().find(|i| i.children.contains(&id) || i.parts.contains(&id))
}

/// Every numeric setting key of the catalogue: fields, switches and unit keys.
pub fn numeric_keys() -> impl Iterator<Item = &'static str> {
    ITEMS.iter().flat_map(|i| {
        let fields = i.fields.iter().filter(|fl| !matches!(fl.kind, Kind::Data | Kind::Font { .. } | Kind::Gradient)).flat_map(|fl| {
            let (g, b) = match fl.kind {
                Kind::Color { green, blue } => (Some(green), Some(blue)),
                _ => (None, None),
            };
            std::iter::once(fl.key).chain(g).chain(b)
        });
        fields.chain(i.toggle).chain(i.unit_key)
    })
}

/// Every text setting key of the catalogue: font names and unique names.
pub fn string_keys() -> impl Iterator<Item = &'static str> {
    ITEMS.iter().flat_map(|i| i.fields).flat_map(|fl| match fl.kind {
        Kind::Font { unique } => std::iter::once(fl.key).chain(unique),
        _ => std::iter::once("").chain(None),
    }).filter(|k| !k.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_children_and_parts_are_catalogued() {
        let mut seen = std::collections::HashSet::new();
        for i in ITEMS {
            assert!(seen.insert(i.id), "duplicate {}", i.id);
            assert!(!i.label.is_empty(), "{}", i.id);
            for c in i.children {
                assert!(item(*c).is_some(), "child {c} of {}", i.id);
                assert_eq!(owner(*c).map(|o| o.id), Some(i.id));
            }
        }
        assert_eq!(item(5000).map(|i| i.id), Some(5001));
        assert!(item(424242).is_none());
    }

    #[test]
    fn ranges_lists_and_keys_are_sane() {
        for i in ITEMS {
            for fl in i.fields {
                assert!(!fl.key.is_empty(), "{}", i.id);
                match fl.kind {
                    Kind::Slider { min, max, .. } | Kind::Indicator { min, max, .. } => assert!(min < max, "{}", i.id),
                    Kind::List(l) => assert!(l.len() >= 2 && l.windows(2).all(|w| w[0].0 < w[1].0), "{}", i.id),
                    Kind::Icons(l) => assert!(l.len() >= 2, "{}", i.id),
                    _ => {}
                }
            }
        }
        let keys: Vec<_> = numeric_keys().collect();
        for k in ["FillCloseGap", "FillUseCloseGap", "FillCloseGapUnit", "TextColorGreen", "SelectCreateType", "SpuitReferType"] {
            assert!(keys.contains(&k), "{k}");
        }
        assert!(!keys.contains(&"TextFontName") && !keys.contains(&"GradationData"), "data values are not numbers");
        let strings: Vec<_> = string_keys().collect();
        assert_eq!(strings, ["TextFontName", "TextFontUniqueName", "TextRubyFontName"]);
    }
}
