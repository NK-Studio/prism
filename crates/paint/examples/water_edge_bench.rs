//! CSP watercolor edge cost on a sparse 6000 x 6000 (36 MP) RGBA8 canvas: a diagonal 500-point
//! stroke with a 40 px brush, live 16-point chunks plus the committed composite. Compares no
//! edge, a live 2 px edge and an after-drag 2 px edge with a 10 px blur. One warmup, three runs,
//! medians.
//!
//! cargo run --release -p prism-paint --example water_edge_bench

use std::time::Instant;

use prism_color::PixelFormat;
use prism_paint::{BrushSettings, CspWaterEdge, StrokePoint, StrokeRenderer};
use prism_raster::Surface;

fn run(brush: &BrushSettings, points: &[StrokePoint]) -> (f64, f64) {
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
    let live = started.elapsed().as_secs_f64() * 1000.0;
    let commit = Instant::now();
    renderer.composite(&original, &mut surface, None, false, true);
    (live, commit.elapsed().as_secs_f64() * 1000.0)
}

fn main() {
    let points: Vec<StrokePoint> = (0..500).map(|i| StrokePoint::new(200.0 + f64::from(i) * 11.0, 300.0 + f64::from(i) * 10.5, 1.0)).collect();
    let base = BrushSettings { size: 40.0, pressure_size: false, spacing: 0.1, ..Default::default() };
    let cases = [
        ("no edge", None),
        ("live edge r2", Some(CspWaterEdge { radius: 2.0, opacity: 0.15, ..Default::default() })),
        ("after-drag edge r2 blur10", Some(CspWaterEdge { radius: 2.0, opacity: 0.15, blur: 10.0, after_drag: true, ..Default::default() })),
    ];
    for (name, edge) in cases {
        let brush = BrushSettings { csp_water_edge: edge, ..base.clone() };
        run(&brush, &points);
        let mut live = Vec::new();
        let mut commit = Vec::new();
        for _ in 0..3 {
            let (l, c) = run(&brush, &points);
            live.push(l);
            commit.push(c);
        }
        live.sort_by(f64::total_cmp);
        commit.sort_by(f64::total_cmp);
        println!("{name:28} live {:8.2} ms   commit {:8.2} ms", live[1], commit[1]);
    }
}
