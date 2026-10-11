//! Compare committed radius edits with the live corner drag on a 24 MP CPU canvas.
//! cargo run --release -p prism-ui-egui --example shape_corners_bench
use std::{io, time::Instant};

use egui::Modifiers;
use prism_ui_egui::{
    PrismApp,
    canvas::{ToolEvent, ensure_texture, tool_event},
    state::Tool,
};
use serde_json::json;

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples.get(samples.len() / 2).copied().unwrap_or(0.0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut a = PrismApp::new(prism_engine::Session::new(), Default::default());
    a.run("file.new", json!({"width":6000,"height":4000,"background":"white"})).map_err(io::Error::other)?;
    a.run("shape.create", json!({"kind":"roundedRect","rect":[800,600,1600,1000],"radii":20,"fill":"#317acf"})).map_err(io::Error::other)?;
    a.ui.tool = Tool::Rectangle;
    a.ui.extras.snap = false;
    a.ui.view.show.smart_guides = false;
    a.sync_views();
    let ctx = egui::Context::default();
    let _ = ensure_texture(&mut a, &ctx, 0, None);
    let mut committed = Vec::new();
    for i in 1..=20 {
        let start = Instant::now();
        a.run("shape.edit", json!({"radii":20+i})).map_err(io::Error::other)?;
        let _ = ensure_texture(&mut a, &ctx, 0, None);
        committed.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    a.run("shape.edit", json!({"radii":20})).map_err(io::Error::other)?;
    let _ = ensure_texture(&mut a, &ctx, 0, None);
    let before = a.session.active().map_or(0, |s| s.history.past_len());
    tool_event(&mut a, ToolEvent::Down { x: 820.0, y: 620.0, pressure: 1.0 }, Modifiers::NONE);
    let mut preview = Vec::new();
    for i in 1..=20 {
        let start = Instant::now();
        let d = f64::from(i);
        tool_event(&mut a, ToolEvent::Move { x: 820.0 + d, y: 620.0 + d, pressure: 1.0 }, Modifiers::NONE);
        let _ = ensure_texture(&mut a, &ctx, 0, None);
        preview.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    tool_event(&mut a, ToolEvent::Up { x: 840.0, y: 640.0 }, Modifiers::NONE);
    let after = a.session.active().map_or(0, |s| s.history.past_len());
    if after != before + 1 {
        return Err(io::Error::other("corner drag did not commit exactly one history entry").into());
    }
    println!(
        "24 MP, 1600×1000 shape, 20 steps, CPU canvas: committed radius edits {:.2} ms/step; live corner preview {:.2} ms/step (median); drag history entries: {}",
        median(committed),
        median(preview),
        after.saturating_sub(before)
    );
    Ok(())
}
