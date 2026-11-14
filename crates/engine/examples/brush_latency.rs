//! Local, read-only CSP brush latency probe. Run with --release or --profile fast.
//! Set PRISM_CSP_LATENCY_UUID to a downloaded brush UUID. Writes no presets or documents.
use prism_engine::{
    Session,
    paint::{StrokePoint, StrokeRenderer},
};
use serde_json::json;
use std::time::Instant;
fn report(name: &str, mut values: Vec<f64>) {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    println!(
        "{name}: p50={:.3}ms p95={:.3}ms max={:.3}ms",
        values.get(n / 2).unwrap_or(&0.0),
        values.get(n * 95 / 100).unwrap_or(&0.0),
        values.last().unwrap_or(&0.0)
    );
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut s = Session::new();
    let uuid = std::env::var("PRISM_CSP_LATENCY_UUID").map_err(|_| "set PRISM_CSP_LATENCY_UUID to a local brush UUID")?;
    let mut params = json!({"uuids":[uuid],"group":"latency"});
    if let Ok(root) = std::env::var("PRISM_TEST_CSP_ROOT") {
        params["root"] = json!(root);
    }
    s.execute("brush.cspLibrary.import", params)?;
    let preset = s.tools.presets.iter().find(|p| p.group == "latency").ok_or("missing preset")?;
    let name = preset.name.clone();
    let b = preset.brush.clone();
    println!("size={} stabilizer={:?} ribbon={:?}", b.size, b.csp_stabilizer, b.csp_ribbon);
    println!("color_bytes={}", b.csp_tip_colors.iter().flatten().map(|c| c.rgb.len()).sum::<usize>());
    s.execute("file.new", json!({"width":6000,"height":4000,"background":"transparent"}))?;
    let fmt = s.active().and_then(|d| d.doc.layers.first()).and_then(|l| l.surface()).ok_or("missing surface")?.format();
    let pts: Vec<_> = (0..=120)
        .map(|i| StrokePoint {
            x: 100.0 + f64::from(i) * 4.0,
            y: 200.0 + (f64::from(i) * 0.06).sin() * 50.0,
            time: f64::from(i) * 1000.0 / 120.0,
            ..Default::default()
        })
        .collect();
    let start = Instant::now();
    let mut r = StrokeRenderer::new(&b, Some(fmt), 1.0);
    println!("prepare_ms={:.3}", start.elapsed().as_secs_f64() * 1000.0);
    let mut preparations = Vec::new();
    for _ in 0..12 {
        let t = Instant::now();
        std::hint::black_box(StrokeRenderer::new(&b, Some(fmt), 1.0));
        preparations.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    report("warm_prepare", preparations);
    let (mut push, mut tail) = (Vec::new(), Vec::new());
    for p in &pts {
        let t = Instant::now();
        r.push(&[*p]);
        push.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        std::hint::black_box(r.tail_preview());
        tail.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    report("raster_push", push);
    report("tail_preview", tail);
    let start = Instant::now();
    let mut live = prism_engine::brush_cmds::LiveStroke::begin(&s, &json!({"preset":name,"points":[[100,200,1]],"seed":7}))?;
    println!("warm_pen_down_ms={:.3}", start.elapsed().as_secs_f64() * 1000.0);
    let mut times = Vec::new();
    for p in &pts {
        let t = Instant::now();
        live.push(&[*p])?;
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    report("engine_push", times);
    Ok(())
}
