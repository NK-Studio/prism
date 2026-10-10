//! Compare repeated committed fill edits with the live picker on a 24 MP CPU canvas.
use photocraft_ui_egui::{
    PhotocraftApp,
    canvas::ensure_texture,
    control::{ControlRequest, Outcome, handle},
};
use serde_json::{Value, json};
use std::{io, time::Instant};

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples.get(samples.len() / 2).copied().unwrap_or(0.0)
}

fn call(a: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> io::Result<Value> {
    let (req, _rx) = ControlRequest::new(method, params);
    match handle(a, ctx, &req) {
        Outcome::Done(v) if v["ok"] == true => Ok(v["result"].clone()),
        _ => Err(io::Error::other(format!("{method} failed"))),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    a.run("file.new", json!({"width":6000,"height":4000,"background":"white"})).map_err(io::Error::other)?;
    a.run("shape.create", json!({"kind":"roundedRect","rect":[800,600,1600,1000],"radii":40,"fill":"#0000ff"})).map_err(io::Error::other)?;
    a.sync_views();
    let ctx = egui::Context::default();
    let _ = ensure_texture(&mut a, &ctx, 0, None);
    let mut committed = Vec::new();
    for i in 1..=20 {
        let start = Instant::now();
        a.run("shape.edit", json!({"fill":format!("#{i:02x}8000")})).map_err(io::Error::other)?;
        let _ = ensure_texture(&mut a, &ctx, 0, None);
        committed.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    a.run("shape.edit", json!({"fill":"#0000ff"})).map_err(io::Error::other)?;
    let _ = ensure_texture(&mut a, &ctx, 0, None);
    let before = a.session.active().map_or(0, |s| s.history.past_len());
    let id = call(&mut a, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","target":"shapeFill"}))?["dialog"]
        .as_u64()
        .ok_or_else(|| io::Error::other("no picker"))?;
    let mut preview = Vec::new();
    for i in 1..=20 {
        let start = Instant::now();
        call(&mut a, &ctx, "ui.dialog.set", json!({"dialog":id,"field":"color","value":format!("#{i:02x}8000")}))?;
        let _ = ensure_texture(&mut a, &ctx, 0, None);
        preview.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    call(&mut a, &ctx, "ui.dialog.confirm", json!({"dialog":id}))?;
    let after = a.session.active().map_or(0, |s| s.history.past_len());
    if after != before + 1 {
        return Err(io::Error::other("picker did not commit exactly one history entry").into());
    }
    println!(
        "24 MP, 1600×1000 shape, 20 colours, CPU canvas: committed edits {:.2} ms/step; live picker {:.2} ms/step (median); history entries: {}",
        median(committed),
        median(preview),
        after.saturating_sub(before)
    );
    Ok(())
}
