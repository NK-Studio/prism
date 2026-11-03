//! The brush model: tip shape plus Photoshop-style dynamics sections, all plain serde data.
//!
//! Every section is optional (an `enabled` flag, or a neutral default), so a partial JSON object
//! such as `{"size": 40, "scattering": {"enabled": true, "scatter": {"jitter": 2.0}}}` deserialises
//! onto the defaults. Field names are camelCase in JSON, matching the command parameters.
//!
//! Units: sizes are pixels, angles are degrees, every percentage is a 0..1 fraction (scatter and
//! texture scale may exceed 1, like Photoshop's 1000 % scatter).

use prism_color::BlendMode;
use serde::{Deserialize, Serialize};

use crate::mixer::MixerSettings;
use crate::pressure_curve::PressureCurve;
use crate::tile::GrayTile;

/// Largest brush diameter accepted by the rasterizer and brush controls.
pub const MAX_BRUSH_SIZE: f32 = 5000.0;

/// Largest Scatter amount (primary and Dual Brush): 10 = 1000 %, the Brush Settings slider's top.
/// Beyond it a dab could land arbitrarily far away, and an unbounded amount overflowed the dab
/// centre to infinity (#977).
pub const MAX_SCATTER: f32 = 10.0;

/// What drives a dynamic parameter (Photoshop's "Control" pop-ups).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Control {
    #[default]
    Off,
    /// Ramp over `fadeSteps` spacing steps (from full to the minimum).
    Fade,
    PenPressure,
    PenTilt,
    StylusWheel,
    /// Barrel rotation of the stylus.
    Rotation,
    /// Angle only: the direction of the first stroke segment.
    InitialDirection,
    /// Angle only: the current stroke direction.
    Direction,
}

/// An additional CSP altitude response, multiplied independently of pen pressure:
/// `minimum + (maximum - minimum) * curve(input)`. The input runs from a horizontal pen (0) to a
/// vertical pen or mouse (1). `minimum`/`maximum` are the multipliers at curve responses 0 and 1;
/// like the source they need not be ordered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CspTiltValues")]
pub struct CspTilt {
    pub minimum: f32,
    pub maximum: f32,
    pub curve: PressureCurve,
}

#[derive(Deserialize)]
struct CspTiltValues {
    minimum: f32,
    maximum: f32,
    curve: PressureCurve,
}

impl TryFrom<CspTiltValues> for CspTilt {
    type Error = String;
    fn try_from(value: CspTiltValues) -> Result<Self, Self::Error> {
        Self::new(value.minimum, value.maximum, value.curve)
    }
}

impl CspTilt {
    /// Five times the base value is Prism's bounded work policy, not a CSP UI limit.
    pub fn new(minimum: f32, maximum: f32, curve: PressureCurve) -> Result<Self, String> {
        if !(0.0..=5.0).contains(&minimum) || !(0.0..=5.0).contains(&maximum) {
            return Err("CSP tilt limits must be finite and within 0..=5".into());
        }
        Ok(Self { minimum, maximum, curve })
    }

    pub fn multiplier(&self, tilt_x: f32, tilt_y: f32) -> f32 {
        let tilt = if tilt_x.is_finite() && tilt_y.is_finite() { tilt_x.hypot(tilt_y).clamp(0.0, 90.0) } else { 0.0 };
        let response = self.curve.sample(1.0 - tilt / 90.0);
        // Direct Rust callers can construct the public fields without deserializing.
        let minimum = if self.minimum.is_finite() { self.minimum.clamp(0.0, 5.0) } else { 1.0 };
        let maximum = if self.maximum.is_finite() { self.maximum.clamp(0.0, 5.0) } else { minimum };
        minimum + (maximum - minimum) * response
    }
}

/// One dynamic parameter: random jitter plus a controller with a floor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Dynamic {
    /// 0..1 random variation per dab (for scatter: the amount, may exceed 1).
    pub jitter: f32,
    pub control: Control,
    /// Steps for [`Control::Fade`].
    pub fade_steps: u32,
    /// 0..1 floor the control/jitter may reduce the value to (minimum diameter, roundness, depth…).
    pub minimum: f32,
    /// Independent response for this parameter when controlled by pen pressure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pressure_curve: Option<PressureCurve>,
    /// CSP can enable pressure and tilt simultaneously for the same parameter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csp_tilt: Option<CspTilt>,
    /// CSP's random control: the value is also scaled per dab by a random factor between this
    /// minimum (0..1) and 1, on top of any pressure or tilt response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csp_random: Option<f32>,
}

impl Default for Dynamic {
    fn default() -> Self {
        Self { jitter: 0.0, control: Control::Off, fade_steps: 25, minimum: 0.0, pressure_curve: None, csp_tilt: None, csp_random: None }
    }
}

impl Dynamic {
    pub const fn controlled(control: Control) -> Self {
        Self { jitter: 0.0, control, fade_steps: 25, minimum: 0.0, pressure_curve: None, csp_tilt: None, csp_random: None }
    }
    pub const fn jitter(jitter: f32) -> Self {
        Self { jitter, control: Control::Off, fade_steps: 25, minimum: 0.0, pressure_curve: None, csp_tilt: None, csp_random: None }
    }
    pub fn is_active(&self) -> bool {
        self.jitter > 0.0 || self.control != Control::Off || self.csp_tilt.is_some() || self.csp_random.is_some()
    }
}

/// The brush tip bitmap source.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TipShape {
    /// Computed round/elliptical tip (hardness, angle, roundness).
    #[default]
    Round,
    /// Sampled tip: grayscale bitmap where 1 = full paint. Scaled so its larger side equals `size`.
    Sampled(GrayTile),
    /// Sampled tips used cyclically by dab index (CSP multi-tip brushes).
    Sequence(Vec<GrayTile>),
}

/// Shape Dynamics.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShapeDynamics {
    pub enabled: bool,
    /// Size jitter / control; `minimum` is Photoshop's Minimum Diameter.
    pub size: Dynamic,
    /// Tilt Scale, 0..2 (0–200 %): with the size control on Pen Tilt, how much a tilted pen
    /// squashes the tip's height (before the tip angle is applied). At full tilt the height is
    /// scaled by `1 − tiltScale / 2`, so 0 leaves the tip alone and 200 % flattens it.
    pub tilt_scale: f32,
    /// Angle jitter (fraction of ±180°) / control.
    pub angle: Dynamic,
    /// Roundness jitter / control; `minimum` is Minimum Roundness.
    pub roundness: Dynamic,
    pub flip_x_jitter: bool,
    pub flip_y_jitter: bool,
    /// Brush Projection: the pen's tilt and barrel rotation are applied to the tip as a
    /// projection (the tip is foreshortened along the tilt direction and turns with the pen).
    pub brush_projection: bool,
}

/// Scattering.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Scattering {
    pub enabled: bool,
    /// `jitter` = scatter amount (1.0 = 100 %: up to one radius of displacement); control scales it.
    pub scatter: Dynamic,
    /// Scatter radially instead of only perpendicular to the stroke.
    pub both_axes: bool,
    /// Dabs per spacing step (1..16).
    pub count: u32,
    /// Count jitter / control (reduces the count towards 1).
    pub count_jitter: Dynamic,
}

impl Default for Scattering {
    fn default() -> Self {
        Self { enabled: false, scatter: Dynamic::default(), both_axes: false, count: 1, count_jitter: Dynamic::default() }
    }
}

/// How a grayscale mask (texture or dual tip) combines with the brush coverage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MaskMode {
    #[default]
    Multiply,
    Subtract,
    Darken,
    Overlay,
    ColorDodge,
    ColorBurn,
    LinearBurn,
    HardMix,
    LinearHeight,
    Height,
}

/// A CLIP STUDIO stroke blending mode Prism runs without a [`prism_color::BlendMode`] equivalent
/// (codes and inferred formulas: [`crate::csp_blend`]). Each composites the stroke's alpha `k`
/// (coverage × opacity × selection) against the pre-stroke pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CspStrokeMode {
    /// Background (29): paints under the existing alpha (`dst` over the stroke).
    Behind,
    /// Replace alpha (31): the brush colour at the stroke opacity replaces the pixel, weighted
    /// by the coverage.
    ReplaceAlpha,
    /// Compare density (34): alpha `max(dst, k)`, the denser of the two colours.
    CompareDensity,
    /// Erase (compare) (35): alpha `min(dst, 1 − k)`; the brush erases with any tool.
    EraseCompare,
}

impl CspStrokeMode {
    /// The mode erases (the brush is an eraser with whichever tool runs it).
    pub fn erases(self) -> bool {
        self == Self::EraseCompare
    }
}

/// A CLIP STUDIO dual brush mode that adds the second brush's opacity outside the first (codes:
/// [`crate::csp_blend::CSP_DUAL_MODES`]): stroke coverage `c` and dual coverage `t` combine into
/// `Normal`/`Screen` `1 − (1 − c)(1 − t)`, `Add` `min(1, c + t)`, `Lighten` `max(c, t)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CspDualAdd {
    Normal,
    Add,
    Lighten,
    Screen,
}

impl CspDualAdd {
    /// Combine stroke coverage `c` with dual coverage `t` (both clamped to 0..1).
    pub fn combine(self, c: f32, t: f32) -> f32 {
        let (c, t) = (c.clamp(0.0, 1.0), t.clamp(0.0, 1.0));
        let r = match self {
            Self::Normal | Self::Screen => 1.0 - (1.0 - c) * (1.0 - t),
            Self::Add => c + t,
            Self::Lighten => c.max(t),
        };
        if r.is_finite() { r.clamp(0.0, 1.0) } else { 0.0 }
    }
}

/// Clip Studio texture compositing. These modes have their own density semantics;
/// similarly named Photoshop masks are not interchangeable at partial density.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CspTextureMode {
    Normal,
    Multiply,
    Subtract,
    Compare,
    Outline,
    Overlay,
    ColorDodge,
    ColorBurn,
    HardMix,
    Height,
}

/// Procedural pattern styles (generated deterministically; no bundled pattern files).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PatternStyle {
    /// Smooth value noise.
    #[default]
    Noise,
    /// Woven canvas.
    Canvas,
    /// Fibrous paper tooth.
    Paper,
    /// Halftone-like dots.
    Dots,
}

/// A texture pattern: a procedural generator or an explicit grayscale tile (tiled infinitely).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Pattern {
    Procedural { style: PatternStyle, size: u32, seed: u32 },
    Tile(GrayTile),
}

impl Default for Pattern {
    fn default() -> Self {
        Pattern::Procedural { style: PatternStyle::Paper, size: 128, seed: 1 }
    }
}

/// Texture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Texture {
    pub enabled: bool,
    pub pattern: Pattern,
    pub invert: bool,
    /// Pattern scale (1 = 100 %).
    pub scale: f32,
    /// -1..1 (Photoshop's -150..150 mapped to ±1).
    pub brightness: f32,
    /// -1..1.
    pub contrast: f32,
    /// Apply the texture to every dab (enables depth jitter) instead of once to the stroke.
    pub each_tip: bool,
    pub mode: MaskMode,
    /// Source-specific texture behavior. When present, replaces `mode`; absent in older presets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csp_mode: Option<CspTextureMode>,
    /// 0..1.
    pub depth: f32,
    /// Depth jitter / control (per tip only); `minimum` is Minimum Depth.
    pub depth_jitter: Dynamic,
}

impl Default for Texture {
    fn default() -> Self {
        Self {
            enabled: false,
            pattern: Pattern::default(),
            invert: false,
            scale: 1.0,
            brightness: 0.0,
            contrast: 0.0,
            each_tip: false,
            mode: MaskMode::Multiply,
            csp_mode: None,
            depth: 1.0,
            depth_jitter: Dynamic::default(),
        }
    }
}

/// Dual Brush: a second tip stroked along the same path; the stroke is masked by it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DualBrush {
    pub enabled: bool,
    pub mode: MaskMode,
    pub tip: TipShape,
    pub size: f32,
    pub hardness: f32,
    pub roundness: f32,
    pub angle: f32,
    pub spacing: f32,
    pub scatter: f32,
    pub both_axes: bool,
    pub count: u32,
    pub flip: bool,
    /// Imported CSP adding mode: the second brush adds opacity outside the first instead of
    /// masking it (`mode` is then unused).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csp_add: Option<CspDualAdd>,
}

impl Default for DualBrush {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: MaskMode::Multiply,
            tip: TipShape::Round,
            size: 20.0,
            hardness: 1.0,
            roundness: 1.0,
            angle: 0.0,
            spacing: 0.25,
            scatter: 0.0,
            both_axes: false,
            count: 1,
            flip: false,
            csp_add: None,
        }
    }
}

/// Color Dynamics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ColorDynamics {
    pub enabled: bool,
    /// Vary every dab (true) or once per stroke (false).
    pub per_tip: bool,
    /// Foreground/background mix jitter / control.
    pub fg_bg: Dynamic,
    /// 0..1 fraction of ±half the hue wheel.
    pub hue_jitter: f32,
    pub saturation_jitter: f32,
    pub brightness_jitter: f32,
    /// -1 (fully desaturated) .. 1 (fully saturated).
    pub purity: f32,
}

impl Default for ColorDynamics {
    fn default() -> Self {
        Self { enabled: false, per_tip: true, fg_bg: Dynamic::default(), hue_jitter: 0.0, saturation_jitter: 0.0, brightness_jitter: 0.0, purity: 0.0 }
    }
}

/// Transfer: per-dab opacity ceiling and flow, plus the Mixer Brush's wetness and mix.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Transfer {
    pub enabled: bool,
    pub opacity: Dynamic,
    pub flow: Dynamic,
    /// Wetness Jitter / control (Mixer Brush only): scales the Wet setting per dab.
    pub wetness: Dynamic,
    /// Mix Jitter / control (Mixer Brush only): scales the Mix setting per dab.
    pub mix: Dynamic,
}

/// The sections a lock can hold (the lock icons in Photoshop's Brush Settings list): a locked
/// section keeps its settings when another brush preset is picked.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SectionLocks {
    pub shape_dynamics: bool,
    pub scattering: bool,
    pub texture: bool,
    pub dual_brush: bool,
    pub color_dynamics: bool,
    pub transfer: bool,
    pub pose: bool,
    pub noise: bool,
    pub wet_edges: bool,
    pub build_up: bool,
    pub smoothing: bool,
    pub protect_texture: bool,
}

/// Brush Pose: stylus values used instead of (override) or for missing device data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Pose {
    pub enabled: bool,
    /// Degrees, -90..90 (W3C Pointer Events convention).
    pub tilt_x: f32,
    pub tilt_y: f32,
    /// Degrees 0..360.
    pub rotation: f32,
    /// 0..1.
    pub pressure: f32,
    pub override_tilt: bool,
    pub override_rotation: bool,
    pub override_pressure: bool,
}

impl Default for Pose {
    fn default() -> Self {
        Self {
            enabled: false,
            tilt_x: 0.0,
            tilt_y: 0.0,
            rotation: 0.0,
            pressure: 1.0,
            override_tilt: false,
            override_rotation: false,
            override_pressure: false,
        }
    }
}

/// Stroke smoothing (Photoshop's Smoothing % and its options).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Smoothing {
    /// 0..1.
    pub amount: f32,
    /// Lazy-mouse: paint only when the pointer pulls a string of `amount × 100 px` taut.
    pub pulled_string: bool,
    /// Keep catching up with the pointer while it pauses (uses point timestamps).
    pub catch_up: bool,
    /// Finish the stroke at the last pointer position. Off by default: finish at
    /// the filtered brush tip instead of extending through the pen's release tail.
    pub catch_up_on_end: bool,
    /// Divide the string length by the view zoom (the command's `zoom` param).
    pub adjust_for_zoom: bool,
}

impl Default for Smoothing {
    fn default() -> Self {
        Self { amount: 0.0, pulled_string: false, catch_up: true, catch_up_on_end: false, adjust_for_zoom: true }
    }
}

/// The highest CSP anti-aliasing level (strong).
pub const CSP_ANTI_ALIAS_MAX: u8 = 3;

/// `cspAntiAlias`: a level 0..=3, or the boolean older presets stored.
fn csp_anti_alias_level<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u8>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Level {
        Flag(bool),
        Level(f64),
    }
    match Option::<Level>::deserialize(d)? {
        None => Ok(None),
        Some(Level::Flag(on)) => Ok(Some(u8::from(on))),
        Some(Level::Level(n)) if n.fract() == 0.0 && (0.0..=f64::from(CSP_ANTI_ALIAS_MAX)).contains(&n) => Ok(Some(n as u8)),
        Some(Level::Level(_)) => Err(serde::de::Error::custom("cspAntiAlias must be 0..3 (none, weak, medium, strong) or a boolean")),
    }
}

/// A complete brush: tip shape, dynamics, and stroke-level options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BrushSettings {
    /// Diameter in pixels.
    pub size: f32,
    /// 0 (soft) ..1 (hard). Computed tips only.
    pub hardness: f32,
    /// Spacing between dabs as a fraction of the diameter.
    pub spacing: f32,
    /// Photoshop's Spacing checkbox. Off: the pointer's speed sets the spacing (one dab every
    /// [`crate::dynamics::SPEED_SPACING_MS`] of stroke time; without timestamps, by distance).
    pub spacing_enabled: bool,
    /// Maximum coverage for the whole stroke.
    pub opacity: f32,
    /// Per-dab coverage.
    pub flow: f32,
    /// Shortcut for Shape Dynamics › Size › Pen Pressure (the options-bar button).
    #[serde(alias = "pressure_size")]
    pub pressure_size: bool,
    /// Shortcut for Transfer › Opacity › Pen Pressure.
    #[serde(alias = "pressure_opacity")]
    pub pressure_opacity: bool,
    /// Straight RGBA colour (display RGB). Commands fill it from the foreground colour.
    pub color: [f32; 4],
    /// Background colour for Color Dynamics' foreground/background jitter.
    pub background: [f32; 4],
    /// Erase instead of paint.
    pub erase: bool,
    /// Brush blend mode (Normal, Multiply, Dissolve…).
    pub mode: BlendMode,
    // --- Brush Tip Shape ---
    pub tip: TipShape,
    /// Degrees, counter-clockwise.
    pub angle: f32,
    /// 0..1 (1 = circle).
    pub roundness: f32,
    pub flip_x: bool,
    pub flip_y: bool,
    /// Pencil: no anti-aliasing (coverage is 0 or 1).
    pub aliased: bool,
    /// CSP tip anti-aliasing level: 0 none, 1 weak, 2 medium, 3 strong (older presets stored
    /// a boolean: false = 0, true = 1). Kept separate from Pencil's binary coverage threshold.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "csp_anti_alias_level")]
    pub csp_anti_alias: Option<u8>,
    /// CSP local tip-axis scale, independent of Photoshop roundness. 2.4 = 240%.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csp_tip_scale: Option<[f32; 2]>,
    // --- dynamics sections ---
    pub shape_dynamics: ShapeDynamics,
    pub scattering: Scattering,
    /// Imported CSP spraying effect: independent particles distributed over the brush area.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csp_spray: Option<crate::csp_spray::CspSpray>,
    pub texture: Texture,
    pub dual_brush: DualBrush,
    pub color_dynamics: ColorDynamics,
    pub transfer: Transfer,
    pub pose: Pose,
    /// Add grain to the soft parts of the tip.
    pub noise: bool,
    /// Watercolour look: lighter interior, darker rims; overlapping dabs don't build up.
    pub wet_edges: bool,
    /// Imported CSP watercolor edge: stroke-level alpha added near the stroke buffer's edges.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csp_water_edge: Option<crate::csp_water_edge::CspWaterEdge>,
    /// Airbrush: keep depositing dabs over time while the pointer lingers.
    pub build_up: bool,
    /// Build-up rate in dabs per second.
    pub build_up_rate: f32,
    /// Photoshop tool smoothing is retained across preset switches, but bypassed for CSP.
    pub smoothing: Smoothing,
    /// Explicit CSP correction policy, including SUTs with stabilization disabled.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub csp_correction: bool,
    /// CSP's fixed sub-color contribution when no input effector is active (0..1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csp_sub_color: Option<f32>,
    /// Imported CSP color mixing (amount/density of paint, color stretch): picks up the colour
    /// under the brush as it moves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csp_mixing: Option<crate::csp_mixing::CspColorMixing>,
    /// Imported CSP Start and end: tapers target parameters over the start and end of the path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csp_in_out: Option<crate::csp_in_out::CspInOut>,
    /// Imported CSP Stabilization / Adjust by speed: a moving average of the raw input samples,
    /// Used with the CSP correction policy; legacy explicit stabilizers remain supported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csp_stabilizer: Option<crate::csp_stabilizer::CspStabilizer>,
    /// Imported CSP Ribbon: the sampled tip is laid continuously along the stroke instead of
    /// stamped as dabs (ignored with a round tip or a spray).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csp_ribbon: Option<crate::csp_ribbon::CspRibbon>,
    /// Imported CSP tip order: which tip of a multi-tip brush each dab uses (`None`: repeat).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csp_tip_order: Option<crate::csp_tip::CspTipOrder>,
    /// Imported CSP colour tips: the colours of tip `i` of `tip`, painted instead of the brush
    /// colour (`None`: that tip paints in the brush colour). Ignored with colour mixing or erasing.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub csp_tip_colors: Vec<Option<crate::csp_tip::CspTipColor>>,
    /// Imported CSP Erase blending mode: the brush erases with whichever tool runs it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub csp_erase: bool,
    /// Imported CSP stroke blending mode without a Prism blend mode (Background, Replace alpha,
    /// Compare density, Erase (compare)); `mode` is then Normal. Ignored with colour mixing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csp_stroke: Option<CspStrokeMode>,
    /// Imported CSP size synchronized with the view scale: the size is in screen pixels, so a
    /// stroke at zoom `z` paints `size / z` image pixels.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub csp_size_by_view: bool,
    /// Keep the current texture (pattern + scale) when switching to another textured preset.
    pub protect_texture: bool,
    /// Sections whose settings stay when another preset is picked (tool state, like Photoshop's locks).
    pub locks: SectionLocks,
    /// Mixer Brush options (Wet, Load, Mix, Flow, Sample All Layers) used by `paint.mixerBrush`.
    pub mixer: MixerSettings,
    /// Random seed for jitters (commands derive it from the stroke when not given), so strokes replay.
    pub seed: u64,
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self {
            size: 20.0,
            hardness: 1.0,
            spacing: 0.1,
            spacing_enabled: true,
            opacity: 1.0,
            flow: 1.0,
            pressure_size: true,
            pressure_opacity: false,
            color: [0.0, 0.0, 0.0, 1.0],
            background: [1.0, 1.0, 1.0, 1.0],
            erase: false,
            mode: BlendMode::Normal,
            tip: TipShape::Round,
            angle: 0.0,
            roundness: 1.0,
            flip_x: false,
            flip_y: false,
            aliased: false,
            csp_anti_alias: None,
            csp_tip_scale: None,
            shape_dynamics: ShapeDynamics::default(),
            scattering: Scattering::default(),
            csp_spray: None,
            texture: Texture::default(),
            dual_brush: DualBrush::default(),
            color_dynamics: ColorDynamics::default(),
            transfer: Transfer::default(),
            pose: Pose::default(),
            noise: false,
            wet_edges: false,
            csp_water_edge: None,
            build_up: false,
            build_up_rate: 20.0,
            smoothing: Smoothing::default(),
            csp_correction: false,
            csp_sub_color: None,
            csp_stabilizer: None,
            csp_in_out: None,
            csp_mixing: None,
            csp_ribbon: None,
            csp_tip_order: None,
            csp_tip_colors: Vec::new(),
            csp_erase: false,
            csp_stroke: None,
            csp_size_by_view: false,
            protect_texture: false,
            locks: SectionLocks::default(),
            mixer: MixerSettings::default(),
            seed: 0,
        }
    }
}

impl BrushSettings {
    /// Keep Photoshop's tool setting available for switching back without filtering CSP input.
    pub fn effective_smoothing(&self) -> Smoothing {
        if self.csp_correction { Smoothing::default() } else { self.smoothing.clone() }
    }

    /// Copy the settings with the primary and dual diameters and Scatter amounts constrained for
    /// rasterization.
    ///
    /// Engine commands reject out-of-range values; this is a final guard for direct users of the
    /// infallible paint API, which must not turn malformed brush dimensions into giant allocations,
    /// nor a huge Scatter amount into non-finite dab centres (#977).
    pub fn bounded_for_render(&self) -> Self {
        let safe_size = |size: f32| {
            if size.is_finite() { size.clamp(0.5, MAX_BRUSH_SIZE) } else { 0.5 }
        };
        // NaN scatters nothing (as before: it never passed the `> 0` check); infinity is the maximum.
        let safe_scatter = |amount: f32| if amount.is_nan() { 0.0 } else { amount.clamp(0.0, MAX_SCATTER) };
        let mut brush = self.clone();
        brush.flow = photoshop_level(brush.flow);
        brush.opacity = photoshop_level(brush.opacity);
        brush.size = safe_size(brush.size);
        brush.dual_brush.size = safe_size(brush.dual_brush.size);
        // A particle size synchronized with the brush size follows it.
        if let Some(spray) = &mut brush.csp_spray
            && let Some(k) = spray.size_ratio.filter(|k| k.is_finite() && *k > 0.0)
        {
            spray.size = safe_size(k * brush.size);
        }
        if let Some(level) = &mut brush.csp_anti_alias {
            *level = (*level).min(CSP_ANTI_ALIAS_MAX);
        }
        if let Some(scale) = &mut brush.csp_tip_scale {
            let diameter = brush.csp_spray.as_ref().map_or(brush.size, |s| safe_size(s.size));
            let limit = (MAX_BRUSH_SIZE / diameter).min(10.0);
            for axis in scale {
                *axis = if axis.is_finite() { axis.clamp(0.01, limit) } else { 1.0 };
            }
        }
        brush.scattering.scatter.jitter = safe_scatter(brush.scattering.scatter.jitter);
        brush.dual_brush.scatter = safe_scatter(brush.dual_brush.scatter);
        if let Some(spray) = &mut brush.csp_spray {
            spray.size = safe_size(spray.size);
            spray.count = spray.count.clamp(1, crate::csp_spray::MAX_PARTICLES);
            spray.bias = if spray.bias.is_finite() { spray.bias.clamp(-1.0, 1.0) } else { 0.0 };
            spray.angle = if spray.angle.is_finite() { spray.angle.rem_euclid(360.0) } else { 0.0 };
            spray.angle_jitter = if spray.angle_jitter.is_finite() { spray.angle_jitter.clamp(0.0, 2.0) } else { 0.0 };
        }
        if let Some(edge) = &mut brush.csp_water_edge {
            *edge = edge.bounded();
        }
        if let Some(mixing) = &mut brush.csp_mixing {
            *mixing = mixing.bounded();
        }
        if let Some(in_out) = &mut brush.csp_in_out {
            *in_out = in_out.bounded();
        }
        if let Some(stabilizer) = &mut brush.csp_stabilizer {
            *stabilizer = stabilizer.bounded();
        }
        brush
    }

    /// The brush a stroke at view zoom `zoom` paints with when its size follows the view scale
    /// ([`Self::csp_size_by_view`]); `None` when it doesn't. Hostile zooms are bounded.
    pub fn csp_size_by_view_at(&self, zoom: f32) -> Option<Self> {
        if !self.csp_size_by_view {
            return None;
        }
        let zoom = if zoom.is_finite() { zoom.clamp(0.01, 100.0) } else { 1.0 };
        let mut b = self.clone();
        b.size = (b.size / zoom).clamp(0.5, MAX_BRUSH_SIZE);
        Some(b)
    }

    /// This brush (a preset) picked while `current` is the tool's brush: Smoothing is a tool
    /// option, so it stays the tool's (Photoshop), a protected texture stays too, and so does every
    /// section `current` has locked (the locks themselves are tool state and carry over).
    pub fn picked_over(self, current: &BrushSettings) -> Self {
        let mut b = self.with_protected_texture(current);
        b.smoothing = current.smoothing.clone();
        let l = &current.locks;
        if l.shape_dynamics {
            b.shape_dynamics = current.shape_dynamics.clone();
        }
        if l.scattering {
            b.scattering = current.scattering.clone();
            b.csp_spray = current.csp_spray.clone();
        }
        if l.texture {
            b.texture = current.texture.clone();
        }
        if l.dual_brush {
            b.dual_brush = current.dual_brush.clone();
        }
        if l.color_dynamics {
            b.color_dynamics = current.color_dynamics.clone();
        }
        if l.transfer {
            b.transfer = current.transfer.clone();
        }
        if l.pose {
            b.pose = current.pose.clone();
        }
        if l.noise {
            b.noise = current.noise;
        }
        if l.wet_edges {
            b.wet_edges = current.wet_edges;
            b.csp_water_edge = current.csp_water_edge.clone();
        }
        if l.build_up {
            b.build_up = current.build_up;
            b.build_up_rate = current.build_up_rate;
        }
        if l.protect_texture {
            b.protect_texture = current.protect_texture;
        }
        b.locks = current.locks.clone();
        b
    }

    /// Apply Protect Texture: if `current` protects its texture, a newly chosen brush with a texture
    /// keeps `current`'s pattern and scale.
    pub fn with_protected_texture(mut self, current: &BrushSettings) -> Self {
        if current.protect_texture && current.texture.enabled && self.texture.enabled {
            self.texture.pattern = current.texture.pattern.clone();
            self.texture.scale = current.texture.scale;
            self.protect_texture = true;
        }
        self
    }
}

/// A brush's Flow or Opacity as Photoshop applies it: to the nearest 1/255 (halves up), whatever
/// the document depth. Measured with `cargo xtask brush-compare` (`flow16-*`, `opac16-*` and
/// `flow*-x10-*` cases): in a 16-bit document one dab at 1 % flow lays 3/255 (0.01178, not 0.01),
/// at 50 % 128/255, and ten stacked 1 % dabs build up as 3/255 dabs do.
pub fn photoshop_level(v: f32) -> f32 {
    if v.is_nan() { 0.0 } else { (v.clamp(0.0, 1.0) * 255.0 + 0.5).floor() / 255.0 }
}

/// A named brush preset (session tool state; serde round-trips).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushPreset {
    pub name: String,
    pub brush: BrushSettings,
    /// Shipped with Prism (built-ins can be deleted from a session but are regenerated on reset).
    #[serde(default)]
    pub builtin: bool,
    /// Preset group (folder) in the Brushes panel, e.g. "General" or an imported file's name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    /// Explicit SUT provenance and presentation; absent in legacy/non-SUT presets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sut: Option<crate::sut::SutBrushMetadata>,
}
