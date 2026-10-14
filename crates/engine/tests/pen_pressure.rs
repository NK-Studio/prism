//! Synthetic pen samples exercise the engine/preview seam; these are controlled input oracles,
//! not recordings from Photoshop or a physical tablet.

use prism_doc::{Document, LayerId};
use prism_engine::{Session, brush_cmds::LiveStroke};
use prism_paint::StrokePoint;
use prism_raster::Surface;
use serde_json::{Value, json};

fn session(depth: u32, mask: bool) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 96, "height": 80, "depth": depth, "background": "transparent"})).unwrap();
    if mask {
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    }
    s
}

fn target(doc: &Document, layer: LayerId, mask: bool) -> &Surface {
    let layer = doc.layer(layer).unwrap();
    if mask { &layer.mask.as_ref().unwrap().surface } else { layer.surface().unwrap() }
}

fn same_pixels(a: &Surface, b: &Surface) {
    assert_eq!(a.format(), b.format());
    for y in 0..80 {
        for x in 0..96 {
            assert_eq!(a.pixel(x, y), b.pixel(x, y), "({x},{y})");
        }
    }
}

fn params(points: &[StrokePoint], mask: bool, smoothing: f32) -> Value {
    json!({
        "points": points,
        "target": if mask { "mask" } else { "pixels" },
        "color": "#000000",
        "seed": 17,
        "brush": {
            "size": 40, "hardness": 1.0, "flow": 0.3, "spacing": 0.1,
            "pressureSize": true, "pressureOpacity": false,
            "smoothing": {"amount": smoothing}
        }
    })
}

#[test]
fn stationary_pressure_grows_live_then_commits_as_one_undo_step() {
    for depth in [8, 16, 32] {
        for mask in [false, true] {
            let mut s = session(depth, mask);
            let state = s.active().unwrap();
            let id = state.active_layer.unwrap();
            let before = target(&state.doc, id, mask).clone();
            let history = state.history.past_len();
            let a = StrokePoint::new(40.0, 40.0, 0.1);
            let grow = StrokePoint { pressure: 0.8, time: 20.0, ..a };
            let hold = StrokePoint { time: 2000.0, ..grow };
            let release = StrokePoint { pressure: 0.1, time: 2010.0, ..a };
            let mut live = LiveStroke::begin(&s, &params(&[a], mask, 0.1)).unwrap();
            assert_eq!(target(&live.doc, id, mask).pixel(50, 40), before.pixel(50, 40));
            assert_ne!(target(&live.doc, id, mask).pixel(40, 40), before.pixel(40, 40), "the first weak contact is visible");
            live.push(&[grow]).unwrap();
            let grown = target(&live.doc, id, mask).clone();
            assert_ne!(grown.pixel(50, 40), before.pixel(50, 40), "stationary pressure must grow the preview: depth={depth}, mask={mask}");
            live.push(&[hold]).unwrap();
            same_pixels(target(&live.doc, id, mask), &grown);
            live.push(&[release]).unwrap();
            assert_eq!(target(&live.doc, id, mask).pixel(50, 40), grown.pixel(50, 40), "less pressure does not erase the outer mark");
            same_pixels(target(&s.active().unwrap().doc, id, mask), &before);
            assert_eq!(s.active().unwrap().history.past_len(), history, "preview does not mutate history");

            let mut p = params(&[a, grow, hold, release], mask, 0.1);
            p["seed"] = json!(live.seed);
            s.execute("paint.stroke", p).unwrap();
            let after = target(&s.active().unwrap().doc, id, mask).clone();
            same_pixels(target(&live.doc, id, mask), &after);
            assert_eq!(s.active().unwrap().history.past_len(), history + 1);
            s.execute("edit.undo", json!({})).unwrap();
            same_pixels(target(&s.active().unwrap().doc, id, mask), &before);
            assert_eq!(s.active().unwrap().history.past_len(), history);
            s.execute("edit.redo", json!({})).unwrap();
            same_pixels(target(&s.active().unwrap().doc, id, mask), &after);
        }
    }
}

#[test]
fn weak_start_and_releasing_tail_match_live_commit_for_each_frame_batching() {
    assert_releasing_tail_matches(None);
}

#[test]
fn sut_sequence_matches_live_commit_and_undo_for_each_frame_batching() {
    use prism_paint::{GrayTile, TipShape};
    let tip =
        TipShape::Sequence(vec![GrayTile::from_fn(8, 8, |x, _| if x < 4 { 1.0 } else { 0.0 }), GrayTile::from_fn(8, 8, |_, y| if y < 4 { 1.0 } else { 0.0 })]);
    assert_releasing_tail_matches(Some(tip));
}

fn assert_releasing_tail_matches(tip: Option<prism_paint::TipShape>) {
    let points: Vec<_> = [(20.0, 0.05), (20.0, 0.2), (32.0, 0.7), (44.0, 0.8), (56.0, 0.5), (68.0, 0.2), (80.0, 0.0)]
        .into_iter()
        .enumerate()
        .map(|(i, (x, p))| StrokePoint { time: i as f64 * 8.0, ..StrokePoint::new(x, 40.0, p) })
        .collect();
    for depth in [8, 16, 32] {
        for mask in [false, true] {
            for smoothing in [0.0, 0.5, 1.0] {
                for catch_up in [false, true] {
                    let mut s = session(depth, mask);
                    let id = s.active().unwrap().active_layer.unwrap();
                    let before = target(&s.active().unwrap().doc, id, mask).clone();
                    let history = s.active().unwrap().history.past_len();
                    let mut p = params(&points, mask, smoothing);
                    if let Some(tip) = &tip {
                        p["brush"]["tip"] = json!(tip);
                    }
                    p["brush"]["smoothing"]["catchUpOnEnd"] = json!(catch_up);
                    let mut previews = Vec::new();
                    for batch in [1, 2, 6] {
                        let mut begin = p.clone();
                        begin["points"] = json!(&points[..1]);
                        let mut live = LiveStroke::begin(&s, &begin).unwrap();
                        for chunk in points[1..].chunks(batch) {
                            live.push(chunk).unwrap();
                        }
                        previews.push(live.doc);
                    }
                    s.execute("paint.stroke", p).unwrap();
                    let committed = target(&s.active().unwrap().doc, id, mask).clone();
                    for preview in previews {
                        same_pixels(target(&preview, id, mask), &committed);
                    }
                    assert_eq!(s.active().unwrap().history.past_len(), history + 1);
                    s.execute("edit.undo", json!({})).unwrap();
                    same_pixels(target(&s.active().unwrap().doc, id, mask), &before);
                    s.execute("edit.redo", json!({})).unwrap();
                    same_pixels(target(&s.active().unwrap().doc, id, mask), &committed);
                }
            }
        }
    }
}

#[test]
#[ignore = "writes target/smoothing-end-comparison.png for visual inspection"]
fn write_smoothing_end_comparison() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 420, "background": "#888888"})).unwrap();
    for (column, catch_up) in [true, false].into_iter().enumerate() {
        let points: Vec<_> = (0..100)
            .map(|i| {
                let t = i as f64 / 99.0;
                let angle = 3.8 - 2.3 * t;
                let pressure = if i < 93 { 0.8 } else { 0.8 - (i - 92) as f64 * 0.11 };
                json!([320.0 + column as f64 * 400.0 + angle.cos() * 245.0, 225.0 - angle.sin() * 145.0, pressure, 0, 0, 0, i * 8])
            })
            .collect();
        s.execute(
            "paint.stroke",
            json!({"points": points, "color": "#ff0033", "seed": 17, "brush": {
                "size": 24, "hardness": 1.0, "spacing": 0.05, "pressureSize": true, "pressureOpacity": false,
                "smoothing": {"amount": 1.0, "catchUpOnEnd": catch_up}
            }}),
        )
        .unwrap();
    }
    let png = prism_io::export(&s.active().unwrap().doc, "png", &Default::default()).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/smoothing-end-comparison.png");
    std::fs::write(path, png.bytes).unwrap();
}

#[test]
#[ignore = "36 MP release benchmark; run with --release --ignored --nocapture"]
fn benchmark_smoothing_end_on_36mp() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 6000, "height": 6000, "background": "white"})).unwrap();
    let points: Vec<_> = (0..500)
        .map(|i| {
            let t = i as f64 / 499.0;
            StrokePoint {
                time: i as f64 * 8.0,
                ..StrokePoint::new(500.0 + t * 5000.0, 3000.0 + (t * 6.0).sin() * 2000.0, if i < 493 { 0.8 } else { 0.8 - (i - 492) as f32 * 0.11 })
            }
        })
        .collect();
    for catch_up in [true, false] {
        let mut p = params(&points[..1], false, 1.0);
        p["brush"]["smoothing"]["catchUpOnEnd"] = json!(catch_up);
        let mut times = Vec::new();
        for run in 0..4 {
            let started = std::time::Instant::now();
            let mut live = LiveStroke::begin(&s, &p).unwrap();
            for chunk in points[1..].chunks(16) {
                live.push(chunk).unwrap();
            }
            std::hint::black_box(&live.doc);
            if run > 0 {
                times.push(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
        times.sort_by(f64::total_cmp);
        println!("36 MP, smoothing 100, catchUpOnEnd={catch_up}: median {:.2} ms; samples {times:?}", times[1]);
    }
}
