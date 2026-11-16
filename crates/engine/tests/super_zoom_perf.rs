//! Super Zoom timings on a 1000×750 RGB document with the real Real-ESRGAN executable.
//!
//! `PRISM_REALESRGAN_DIR=<unpacked runtime> cargo test --release -p prism-engine --test super_zoom_perf -- --ignored --nocapture`

use prism_engine::Session;
use prism_engine::neural_cmds::{COMMAND, Model, installed};
use serde_json::json;

#[test]
#[ignore = "needs an installed Real-ESRGAN runtime (PRISM_REALESRGAN_DIR) and a Vulkan GPU"]
fn super_zoom_timings() {
    assert!(installed(Model::X4Plus), "set PRISM_REALESRGAN_DIR");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1000, "height": 750, "background": "white"})).unwrap();
    // Some detail for the model to work on.
    s.execute("filter.noise.addNoise", json!({"amount": 25})).ok();
    let models: Vec<&str> = std::env::var("SUPER_ZOOM_MODELS")
        .ok()
        .map(|m| m.split(',').map(|x| Box::leak(x.to_string().into_boxed_str()) as &str).collect())
        .unwrap_or_else(|| Model::ALL.iter().map(|m| m.id()).collect());
    for model in models {
        for scale in [4u32, 2] {
            for output in ["document", "newDocument"] {
                let t = std::time::Instant::now();
                let v = s.execute(COMMAND, json!({"model": model, "scale": scale, "output": output, "install": false}));
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                match v {
                    Ok(v) => eprintln!("{model} x{scale} {output}: {ms:.0} ms total; stages {}", v.get("ms").cloned().unwrap_or_default()),
                    Err(e) => panic!("{model} x{scale} {output}: {e}"),
                }
                if output == "document" {
                    s.execute("edit.undo", json!({})).unwrap();
                } else {
                    s.execute("file.close", json!({"force": true})).ok();
                    s.set_active(0);
                }
            }
        }
    }
}
