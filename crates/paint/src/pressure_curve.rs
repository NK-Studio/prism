//! Validated per-parameter pen-pressure responses. No application-specific file encoding lives here.

use std::fmt;
use std::sync::Arc;

use serde::de::{Error, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Maximum number of editable control points in one pressure response.
pub const MAX_PRESSURE_CURVE_POINTS: usize = 16;

/// A normalized pressure response with ordered input coordinates and unrestricted output order.
/// Two points define a line. Additional interior points are quadratic Bezier controls, joined
/// at adjacent controls' midpoints; only the first and last points are interpolated endpoints.
/// Clones share immutable points; construction and deserialization enforce the same bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct PressureCurve {
    points: Arc<[[f64; 2]]>,
}

impl PressureCurve {
    pub fn new(points: Vec<[f64; 2]>) -> Result<Self, String> {
        if !(2..=MAX_PRESSURE_CURVE_POINTS).contains(&points.len()) {
            return Err(format!("a pressure curve needs 2..={MAX_PRESSURE_CURVE_POINTS} points"));
        }
        let mut previous_x = None;
        for &[x, y] in &points {
            if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
                return Err("pressure curve coordinates must be finite and within 0..=1".into());
            }
            if previous_x.is_some_and(|previous| x <= previous) {
                return Err("pressure curve input coordinates must be strictly increasing".into());
            }
            previous_x = Some(x);
        }
        Ok(Self { points: points.into() })
    }

    pub fn points(&self) -> &[[f64; 2]] {
        &self.points
    }

    /// Evaluate normalized pressure. Outside the endpoints, extrapolate the boundary segment's
    /// endpoint chord; clamp the resulting response to the usable 0..=1 range.
    pub fn sample(&self, input: f32) -> f32 {
        let input = if input.is_nan() { 0.0 } else { f64::from(input).clamp(0.0, 1.0) };
        if let [left, right] = self.points.as_ref() {
            return linear_at_x(*left, *right, input);
        }
        let midpoint = |[ax, ay]: [f64; 2], [bx, by]: [f64; 2]| [(ax + bx) * 0.5, (ay + by) * 0.5];
        for (i, controls) in self.points.windows(3).enumerate() {
            let [previous, control, next] = controls else { continue };
            let start = if i == 0 { *previous } else { midpoint(*previous, *control) };
            let is_last = i == self.points.len().saturating_sub(3);
            let end = if is_last { *next } else { midpoint(*control, *next) };
            let [start_x, _] = start;
            let [end_x, _] = end;
            if input <= end_x || is_last {
                if input < start_x || input > end_x {
                    return linear_at_x(start, end, input);
                }
                return quadratic_at_x(start, *control, end, input);
            }
        }
        self.points.last().map_or(0.0, |&[_, y]| y as f32)
    }
}

fn linear_at_x([x0, y0]: [f64; 2], [x1, y1]: [f64; 2], input: f64) -> f32 {
    // A constant response remains constant even if extrapolating across subnormal x spacing
    // makes the ratio infinite. The equality guard also handles rounded midpoint coordinates.
    if y0 == y1 || x0 == x1 {
        return y0 as f32;
    }
    (y0 + (y1 - y0) * ((input - x0) / (x1 - x0))).clamp(0.0, 1.0) as f32
}

/// Solve the strictly increasing quadratic x(t), then evaluate y(t). Normalizing the
/// segment's x range avoids underflow for very close control coordinates. Rationalizing the
/// positive quadratic root avoids cancellation near a straight line or the segment start.
fn quadratic_at_x([x0, y0]: [f64; 2], [cx, cy]: [f64; 2], [x1, y1]: [f64; 2], input: f64) -> f32 {
    let width = x1 - x0;
    let x = ((input - x0) / width).clamp(0.0, 1.0);
    if x <= 0.0 {
        return y0 as f32;
    }
    let b = 2.0 * ((cx - x0) / width).clamp(0.0, 1.0);
    let discriminant = (b * b + 4.0 * (1.0 - b) * x).max(0.0);
    let t = (2.0 * x / (b + discriminant.sqrt())).clamp(0.0, 1.0);
    let one_minus_t = 1.0 - t;
    (one_minus_t * one_minus_t * y0 + 2.0 * one_minus_t * t * cy + t * t * y1).clamp(0.0, 1.0) as f32
}

impl Serialize for PressureCurve {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.points.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PressureCurve {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct CurveVisitor;
        impl<'de> Visitor<'de> for CurveVisitor {
            type Value = PressureCurve;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an array of 2 to 16 normalized pressure curve points")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
                if sequence.size_hint().is_some_and(|size| size > MAX_PRESSURE_CURVE_POINTS) {
                    return Err(A::Error::custom("too many pressure curve points"));
                }
                let mut points = Vec::with_capacity(MAX_PRESSURE_CURVE_POINTS);
                while let Some(point) = sequence.next_element::<[f64; 2]>()? {
                    if points.len() == MAX_PRESSURE_CURVE_POINTS {
                        return Err(A::Error::custom("too many pressure curve points"));
                    }
                    points.push(point);
                }
                PressureCurve::new(points).map_err(A::Error::custom)
            }
        }
        deserializer.deserialize_seq(CurveVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_coordinates_point_count_and_input_order() {
        for points in [
            vec![],
            vec![[0.0, 0.0]],
            vec![[0.0, 0.0]; 17],
            vec![[0.0, 0.0], [0.0, 1.0]],
            vec![[0.5, 0.0], [0.25, 1.0]],
            vec![[-0.1, 0.0], [1.0, 1.0]],
            vec![[0.0, 0.0], [1.1, 1.0]],
            vec![[0.0, -0.1], [1.0, 1.0]],
            vec![[0.0, 0.0], [1.0, 1.1]],
            vec![[0.0, f64::NAN], [1.0, 1.0]],
            vec![[f64::NEG_INFINITY, 0.0], [1.0, 1.0]],
            vec![[0.0, 0.0], [1.0, f64::INFINITY]],
        ] {
            assert!(PressureCurve::new(points).is_err());
        }
        let points: Vec<_> = (0..16).map(|i| [f64::from(i) / 15.0, f64::from(i % 2)]).collect();
        assert_eq!(PressureCurve::new(points.clone()).unwrap().points(), points);
    }

    #[test]
    fn serde_uses_a_bounded_validated_point_array() {
        let json = "[[0.0,1.0],[0.5,0.25],[1.0,0.75]]";
        let curve: PressureCurve = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::to_string(&curve).unwrap(), json);
        for json in ["null", "{}", "[]", "[[0,0]]", "[[0,0],[0,1]]", "[[0,0],[1,2]]", "[[0,0,0],[1,1]]", "[[0,0],[1,null]]"] {
            assert!(serde_json::from_str::<PressureCurve>(json).is_err(), "{json}");
        }
        let oversized: Vec<_> = (0..17).map(|i| [f64::from(i) / 16.0, 0.5]).collect();
        assert!(serde_json::from_value::<PressureCurve>(serde_json::to_value(oversized).unwrap()).is_err());
    }

    #[test]
    fn json_round_trip_preserves_exact_double_coordinates_and_adjacent_inputs() {
        // Publicly constructed fractions exercise the accurate JSON float parser. Without
        // float_roundtrip, the first coordinate below changes by one ULP on reload.
        for i in 1..10007 {
            let x = f64::from(i) / 10007.0;
            let next = f64::from_bits(x.to_bits() + 1);
            let curve = PressureCurve::new(vec![[x, 1.0 - x], [next, x]]).unwrap();
            let serialized = serde_json::to_string(&curve).unwrap();
            let restored: PressureCurve = serde_json::from_str(&serialized).unwrap();
            assert_eq!(restored, curve);
        }
    }

    #[test]
    fn extrapolates_boundary_chords_and_clamps_the_response() {
        let curve = PressureCurve::new(vec![[0.25, 0.75], [0.5, 0.125], [0.75, 0.5]]).unwrap();
        assert_eq!(curve.sample(0.25), 0.75);
        assert_eq!(curve.sample(0.75), 0.5);
        for input in [-1.0, 0.0, f32::NEG_INFINITY, f32::NAN] {
            assert_eq!(curve.sample(input), 0.875);
        }
        for input in [1.0, 2.0, f32::INFINITY] {
            assert_eq!(curve.sample(input), 0.375);
        }
        for points in
            [vec![[0.25, 0.25], [0.75, 0.75]], vec![[0.25, 0.25], [0.5, 1.0], [0.75, 0.75]], vec![[0.25, 0.25], [0.375, 1.0], [0.625, 0.0], [0.75, 0.75]]]
        {
            let curve = PressureCurve::new(points).unwrap();
            assert_eq!(curve.sample(0.0), 0.0);
            assert_eq!(curve.sample(1.0), 1.0);
        }
        let steep = PressureCurve::new(vec![[0.25, 0.0], [0.75, 1.0]]).unwrap();
        assert_eq!(steep.sample(0.0), 0.0);
        assert_eq!(steep.sample(1.0), 1.0);
        let tiny_constant = PressureCurve::new(vec![[0.0, 0.5], [f64::from_bits(1), 0.5]]).unwrap();
        assert_eq!(tiny_constant.sample(1.0), 0.5);
    }

    #[test]
    fn evaluates_quadratic_controls_and_midpoint_joins() {
        let quadratic = PressureCurve::new(vec![[0.0, 0.0], [0.5, 1.0], [1.0, 1.0]]).unwrap();
        for i in 0..=100 {
            let x = i as f32 / 100.0;
            assert!((quadratic.sample(x) - (2.0 * x - x * x)).abs() < 1e-6);
        }
        let chain = PressureCurve::new(vec![[0.0, 0.0], [0.25, 1.0], [0.75, 0.0], [1.0, 1.0]]).unwrap();
        assert_eq!(chain.sample(0.25), 0.625);
        assert_eq!(chain.sample(0.5), 0.5);
        assert_eq!(chain.sample(0.75), 0.375);
        assert!((chain.sample(0.5 - 1e-6) - chain.sample(0.5 + 1e-6)).abs() < 1e-5);
    }

    #[test]
    fn quadratic_inversion_handles_nonuniform_x_and_nearly_linear_segments() {
        for control_x in [1e-9, 0.1, 0.499999999, 0.5, 0.500000001, 0.9, 1.0 - 1e-9] {
            let curve = PressureCurve::new(vec![[0.0, 0.0], [control_x, 0.75], [1.0, 0.25]]).unwrap();
            for i in 0..=100 {
                let t = f64::from(i) / 100.0;
                let x = 2.0 * (1.0 - t) * t * control_x + t * t;
                let y = 2.0 * (1.0 - t) * t * 0.75 + t * t * 0.25;
                assert!((f64::from(curve.sample(x as f32)) - y).abs() < 2e-5, "control {control_x}, t {t}");
            }
        }
        let line = PressureCurve::new(vec![[0.25, 0.75], [0.75, 0.25]]).unwrap();
        assert_eq!(line.sample(0.5), 0.5);
    }
}
