//! Equivalent pressure-response benchmark on a sparse 6000 x 6000 (36 MP) RGBA8 canvas.
//! Uses 500 input points, a 40 px brush, 16-point live-render chunks, one warmup and three runs.
//! Output equality and dab counts are checked outside the timed region for every run.
//!
//! cargo run --release -p prism-paint --example pressure_curve_bench

use std::hint::black_box;
use std::io::Write;
use std::time::Instant;

use prism_color::PixelFormat;
use prism_geom::Rect;
use prism_paint::{BrushSettings, Control, Dynamic, PressureCurve, ShapeDynamics, StrokePoint, StrokeRenderer};
use prism_raster::Surface;
use serde_json::json;

const REPEATS: usize = 3;
const SAMPLE_CALLS: u32 = 1_000_000;

struct StrokeRun {
    elapsed_ms: f64,
    dabs: usize,
    surface: Surface,
}

fn stroke(brush: &BrushSettings, points: &[StrokePoint]) -> Result<StrokeRun, String> {
    let original = Surface::new(PixelFormat::RGBA8);
    let mut surface = original.clone();
    let started = Instant::now();
    let mut renderer = StrokeRenderer::new(brush, Some(PixelFormat::RGBA8), 1.0);
    for chunk in points.chunks(16) {
        renderer.push(chunk);
        renderer.composite(&original, &mut surface, None, false, false);
    }
    renderer.finish();
    renderer.composite(&original, &mut surface, None, false, false);
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    if !Rect::new(0, 0, 6000, 6000).contains_rect(&renderer.bounds()) {
        return Err("benchmark stroke escaped its 6000 x 6000 canvas".into());
    }
    Ok(StrokeRun { elapsed_ms, dabs: renderer.dab_count(), surface })
}

fn median(mut samples: Vec<f64>) -> Result<f64, String> {
    if samples.len() != REPEATS {
        return Err("benchmark requires three timing samples".into());
    }
    samples.sort_by(f64::total_cmp);
    samples.get(REPEATS / 2).copied().ok_or_else(|| "missing benchmark median".into())
}

fn sample_cost(curve: Option<&PressureCurve>) -> f64 {
    let started = Instant::now();
    let mut sum = 0.0f64;
    for i in 0..SAMPLE_CALLS {
        let input = black_box((i % 1001) as f32 / 1000.0);
        let value = match curve {
            Some(curve) => black_box(curve).sample(input),
            None => input.clamp(0.0, 1.0),
        };
        sum += f64::from(black_box(value));
    }
    black_box(sum);
    started.elapsed().as_secs_f64() * 1000.0
}

fn main() -> Result<(), String> {
    let two = PressureCurve::new(vec![[0.0, 0.0], [1.0, 1.0]])?;
    let sixteen = PressureCurve::new(
        (0..16)
            .map(|i| {
                let x = f64::from(i) / 15.0;
                [x, x]
            })
            .collect(),
    )?;
    let cases = [("linear_control", None), ("identity_2_points", Some(two)), ("identity_16_points", Some(sixteen))];
    let points: Vec<_> = (0..500)
        .map(|i| {
            let t = f64::from(i) / 499.0;
            StrokePoint {
                time: f64::from(i) * 8.0,
                ..StrokePoint::new(300.0 + t * 5400.0, 3000.0 + (t * 12.0).sin() * 2000.0, 0.1 + 0.9 * (t * 30.0).sin().abs() as f32)
            }
        })
        .collect();
    let mut reference: Option<StrokeRun> = None;
    let mut rows = Vec::new();
    for (label, curve) in cases {
        let brush = BrushSettings {
            size: 40.0,
            hardness: 0.8,
            spacing: 0.25,
            pressure_size: false,
            seed: 1,
            shape_dynamics: ShapeDynamics {
                enabled: true,
                size: Dynamic { pressure_curve: curve.clone(), ..Dynamic::controlled(Control::PenPressure) },
                ..Default::default()
            },
            ..Default::default()
        };
        let warmup = stroke(&brush, &points)?;
        if reference.is_none() {
            reference = Some(warmup);
        } else if reference.as_ref().is_some_and(|base| base.dabs != warmup.dabs || base.surface != warmup.surface) {
            return Err(format!("{label}: warmup differs from the baseline"));
        }
        let mut stroke_ms = Vec::with_capacity(REPEATS);
        let mut dabs = 0;
        let mut tiles = 0;
        for _ in 0..REPEATS {
            let run = stroke(&brush, &points)?;
            if reference.as_ref().is_some_and(|base| base.dabs != run.dabs || base.surface != run.surface) {
                return Err(format!("{label}: pixels or dab count differ from the baseline"));
            }
            dabs = run.dabs;
            tiles = run.surface.tile_count();
            stroke_ms.push(run.elapsed_ms);
        }
        black_box(sample_cost(curve.as_ref()));
        let sample_ms: Vec<_> = (0..REPEATS).map(|_| sample_cost(curve.as_ref())).collect();
        rows.push(json!({
            "case": label,
            "stroke_median_ms": median(stroke_ms.clone())?,
            "stroke_runs_ms": stroke_ms,
            "sample_million_median_ms": median(sample_ms.clone())?,
            "sample_million_runs_ms": sample_ms,
            "dabs": dabs,
            "allocated_tiles": tiles,
            "matches_baseline_pixels_and_dabs": true
        }));
    }
    let report = json!({
        "canvas": {"width": 6000, "height": 6000, "megapixels": 36, "storage": "sparse", "format": "RGBA8"},
        "input_points": points.len(), "brush_size": 40, "chunk_points": 16,
        "warmups": 1, "repeats": REPEATS, "sample_calls_per_run": SAMPLE_CALLS,
        "cases": rows
    });
    let output = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    writeln!(std::io::stdout().lock(), "{output}").map_err(|e| e.to_string())
}
