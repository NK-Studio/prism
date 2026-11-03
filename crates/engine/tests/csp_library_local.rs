//! Every material in the local CLIP STUDIO library, end to end: import each one on its own,
//! paint a pressure stroke with every imported brush, and write contact sheets plus a JSON
//! report. Reads the user's CLIP STUDIO folder, so it is opt-in:
//!
//! ```sh
//! cargo test -p prism-engine --test csp_library_local -- --ignored --nocapture
//! ```
//!
//! `PRISM_TEST_CSP_ROOT` picks the CLIP STUDIO user data folder (default: the platform's), and
//! `PRISM_CSP_CHECK_OUT` the output folder (default `target/csp-check`).
use prism_engine::Session;
use serde_json::{Value, json};
use std::time::Instant;

const CELL_W: f64 = 400.0;
const CELL_H: f64 = 110.0;
const COLS: usize = 4;
const ROWS: usize = 10;
/// Brushes larger than this paint at this size, so a stroke fits its cell.
const MAX_SIZE: f64 = 40.0;

fn stroke(x0: f64, y0: f64) -> Value {
    let n = 48;
    let pts: Vec<Value> = (0..=n)
        .map(|i| {
            let t = f64::from(i) / f64::from(n);
            let pressure = if t < 0.6 { 0.1 + 0.9 * t / 0.6 } else { 1.0 - 0.7 * (t - 0.6) / 0.4 };
            json!([x0 + 30.0 + t * (CELL_W - 60.0), y0 + CELL_H / 2.0 + (t * std::f64::consts::TAU).sin() * 22.0, pressure, 0.0, 0.0, 0.0, t * 600.0])
        })
        .collect();
    Value::Array(pts)
}

/// Alpha coverage painted in a cell of the active layer.
fn painted(s: &Session, x0: i32, y0: i32) -> f64 {
    let d = s.active().unwrap();
    let surf = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap();
    let mut sum = 0.0;
    for y in (y0..y0 + CELL_H as i32).step_by(2) {
        for x in (x0..x0 + CELL_W as i32).step_by(2) {
            sum += f64::from(surf.pixel(x, y).get(3).copied().unwrap_or(0.0));
        }
    }
    sum
}

#[test]
#[ignore = "reads the local CLIP STUDIO user data folder"]
fn every_local_csp_material_imports_and_paints() {
    let out = std::path::PathBuf::from(std::env::var_os("PRISM_CSP_CHECK_OUT").unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/csp-check").into()));
    std::fs::create_dir_all(&out).unwrap();
    let mut root = json!({});
    if let Some(r) = std::env::var_os("PRISM_TEST_CSP_ROOT") {
        root["root"] = json!(r.to_string_lossy());
    }
    let mut s = Session::new();
    let listed = s.execute("brush.cspLibrary.list", root.clone()).unwrap();
    let materials = listed["brushes"].as_array().unwrap().clone();
    println!("{} materials, {} ready", materials.len(), listed["ready"]);
    let mut report = Vec::new();
    // Paintable presets: (material index, preset name).
    let mut brushes: Vec<(usize, String)> = Vec::new();
    for (i, m) in materials.iter().enumerate() {
        let mut row = json!({"index": i, "uuid": m["uuid"], "name": m["name"], "kind": m["kind"], "catalog": m["catalog"], "status": m["status"]});
        if m["status"] != "ready" {
            row["error"] = json!(format!("not ready: {}", m["guidance"]));
            report.push(row);
            continue;
        }
        let before: Vec<String> = s.tools.presets.iter().map(|p| p.name.clone()).collect();
        let mut p = root.clone();
        p["uuids"] = json!([m["uuid"]]);
        p["group"] = json!(format!("csp-check-{i}"));
        let t = Instant::now();
        match s.execute("brush.cspLibrary.import", p) {
            Ok(r) => {
                row["importMs"] = json!(t.elapsed().as_millis());
                row["warnings"] = r["warnings"].clone();
                row["notes"] = r["notes"].clone();
                row["failed"] = r["failed"].clone();
                let group = format!("csp-check-{i}");
                let mut presets = Vec::new();
                for preset in s.tools.presets.iter().filter(|p| p.group == group) {
                    let sut = preset.sut.as_ref();
                    presets.push(json!({
                        "name": preset.name,
                        "size": preset.brush.size,
                        "kind": sut.map(|m| m.kind().label()),
                        "ops": sut.map(|m| [m.input_op, m.output_op, m.node_icon]),
                        "brush": sut.is_none_or(|m| m.kind().is_brush()),
                    }));
                    if sut.is_none_or(|m| m.kind().is_brush()) {
                        brushes.push((i, preset.name.clone()));
                    }
                }
                row["presets"] = json!(presets);
                let _ = before;
            }
            Err(e) => row["error"] = json!(e.to_string()),
        }
        report.push(row);
    }

    // Contact sheets: one stroke per brush, in order.
    let per_sheet = COLS * ROWS;
    for (sheet, chunk) in brushes.chunks(per_sheet).enumerate() {
        s.execute("file.new", json!({"width": CELL_W as usize * COLS, "height": CELL_H as usize * ROWS, "depth": 8, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        for (k, (i, name)) in chunk.iter().enumerate() {
            let (x0, y0) = ((k % COLS) as f64 * CELL_W, (k / COLS) as f64 * CELL_H);
            let size = s.tools.presets.iter().find(|p| &p.name == name).map_or(10.0, |p| f64::from(p.brush.size));
            let mut p = json!({"preset": name, "points": stroke(x0, y0), "color": "#202060", "seed": 7});
            if size > MAX_SIZE {
                p["size"] = json!(MAX_SIZE);
            }
            let t = Instant::now();
            let result = s.execute("paint.stroke", p);
            let ms = t.elapsed().as_millis();
            let coverage = painted(&s, x0 as i32, y0 as i32);
            let row = report.iter_mut().find(|r| r["index"] == json!(i)).unwrap();
            let entry = json!({"preset": name, "sheet": sheet, "cell": k, "strokeMs": ms, "coverage": coverage, "error": result.err().map(|e| e.to_string())});
            match row.get_mut("strokes").and_then(Value::as_array_mut) {
                Some(a) => a.push(entry),
                None => row["strokes"] = json!([entry]),
            }
        }
        let path = out.join(format!("sheet-{sheet}.png"));
        s.execute("file.saveACopy", json!({"path": path.to_string_lossy(), "layers": false})).unwrap();
        s.execute("file.close", json!({"save": false})).ok();
    }
    let path = out.join("report.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    // Summary.
    let errors = report.iter().filter(|r| r.get("error").is_some()).count();
    let warned = report.iter().filter(|r| r["warnings"].as_array().is_some_and(|w| !w.is_empty())).count();
    let empty = report.iter().flat_map(|r| r["strokes"].as_array().cloned().unwrap_or_default()).filter(|s| s["coverage"].as_f64().unwrap_or(0.0) < 1.0 || !s["error"].is_null()).count();
    println!("report: {} ({} import errors, {} with warnings, {} empty or failed strokes, {} brushes)", path.display(), errors, warned, empty, brushes.len());
}

/// One material by name (`PRISM_CSP_ONE`): its brush settings, and the coverage of a stroke
/// with each mapped feature turned off in turn.
#[test]
#[ignore = "reads the local CLIP STUDIO user data folder"]
fn one_local_csp_material() {
    let Some(name) = std::env::var_os("PRISM_CSP_ONE") else { return };
    let name = name.to_string_lossy().to_string();
    let mut s = Session::new();
    let listed = s.execute("brush.cspLibrary.list", json!({})).unwrap();
    let m = listed["brushes"].as_array().unwrap().iter().find(|b| b["name"] == json!(name)).unwrap().clone();
    let r = s.execute("brush.cspLibrary.import", json!({"uuids": [m["uuid"]], "group": "one"})).unwrap();
    println!("warnings: {}", r["warnings"]);
    let preset = s.tools.presets.iter().find(|p| p.group == "one").unwrap().clone();
    let mut b = serde_json::to_value(&preset.brush).unwrap();
    if let Some(tip) = b.get_mut("tip") {
        *tip = json!("…");
    }
    println!("brush: {}", serde_json::to_string(&b).unwrap());
    s.execute("file.new", json!({"width": 400, "height": 110, "depth": 8, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let variants: Vec<(&str, Value)> = vec![
        ("as imported", json!({})),
        ("size 40", json!({"size": 40})),
        ("no in/out", json!({"brush": {"cspInOut": null}})),
        ("no size dynamics", json!({"brush": {"shapeDynamics": {"enabled": false}}})),
        ("no transfer", json!({"brush": {"transfer": {"enabled": false}}})),
        ("no stabilizer", json!({"brush": {"cspStabilizer": null}})),
        ("round tip", json!({"brush": {"tip": "round"}})),
    ];
    for (label, extra) in variants {
        s.execute("edit.selectAll", json!({})).ok();
        s.execute("select.all", json!({})).ok();
        s.execute("edit.clear", json!({})).ok();
        s.execute("select.deselect", json!({})).ok();
        let mut p = json!({"preset": preset.name, "points": stroke(0.0, 0.0), "seed": 7});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        let res = s.execute("paint.stroke", p);
        println!("{label}: coverage {:.1} {:?}", painted(&s, 0, 0), res.err());
    }
}

/// Experiment candidates for the open SUT behaviours (docs/sut-experiment-materials.md):
/// start/end fade (`BrushInOutType` 2), non-pixel lengths (`*Unit` ≠ 0) and colour mixing type 2
/// (`BrushWaterColor` 2). Prints a table and writes `target/csp-check/experiment-materials.json`.
#[test]
#[ignore = "reads the local CLIP STUDIO user data folder"]
fn local_csp_experiment_materials() {
    let mut s = Session::new();
    let mut root = json!({});
    if let Some(r) = std::env::var_os("PRISM_TEST_CSP_ROOT") {
        root["root"] = json!(r.to_string_lossy());
    }
    let listed = s.execute("brush.cspLibrary.list", root.clone()).unwrap();
    let keys = [
        "BrushInOutType",
        "BrushUseIn",
        "BrushUseOut",
        "BrushInLength",
        "BrushOutLength",
        "BrushInRatio",
        "BrushOutRatio",
        "BrushInLengthUnit",
        "BrushOutLengthUnit",
        "BrushSizeUnit",
        "BrushSpraySizeUnit",
        "BrushWaterEdgeRadiusUnit",
        "DualSizeUnit",
        "DualSpraySizeUnit",
        "BrushWaterColor",
        "BrushWaterColor2",
        "BrushUseWaterColor",
        "BrushRibbon",
    ];
    let mut found = Vec::new();
    // Colour mixing on with type 0 or 1: the controls for type 2.
    let mut mixing_controls = Vec::new();
    for (i, m) in listed["brushes"].as_array().unwrap().iter().enumerate() {
        if m["status"] != "ready" {
            continue;
        }
        let mut p = root.clone();
        p["uuids"] = json!([m["uuid"]]);
        p["group"] = json!(format!("csp-exp-{i}"));
        let Ok(r) = s.execute("brush.cspLibrary.import", p) else { continue };
        for preset in s.tools.presets.iter().filter(|p| p.group == format!("csp-exp-{i}")) {
            let Some(meta) = &preset.sut else { continue };
            let v = |k: &str| meta.parameters.get(k).copied().unwrap_or(0.0);
            let fade = v("BrushInOutType") == 2.0 && (v("BrushUseIn") != 0.0 || v("BrushUseOut") != 0.0);
            let units: Vec<String> = keys.iter().filter(|k| k.ends_with("Unit") && v(k) != 0.0).map(|k| format!("{k}={}", v(k))).collect();
            // CSP writes the mixing flag under two names; either enables it.
            let mixing_on = v("BrushUseWaterColor") != 0.0 || v("BrushUseWaterColor2") != 0.0;
            let mix2 = mixing_on && v("BrushWaterColor") == 2.0;
            if mixing_on && v("BrushWaterColor") != 2.0 {
                mixing_controls.push(json!({"name": preset.name, "catalog": m["catalog"], "type": v("BrushWaterColor")}));
            }
            if fade || !units.is_empty() || mix2 {
                let values: serde_json::Map<String, Value> = keys.iter().filter(|k| meta.parameters.contains_key(**k)).map(|k| (k.to_string(), json!(v(k)))).collect();
                println!("{:<28} {:<24} fade2={fade:<5} mix2={mix2:<5} units={units:?}", preset.name, m["catalog"].as_str().unwrap_or(""));
                found.push(json!({
                    "name": preset.name, "uuid": m["uuid"], "catalog": m["catalog"], "fadeType2": fade, "mixingType2": mix2,
                    "units": units, "values": values, "warnings": r["warnings"],
                }));
            }
        }
    }
    let out = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/csp-check"));
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("experiment-materials.json"), serde_json::to_vec_pretty(&json!({"candidates": found, "mixingControls": mixing_controls})).unwrap()).unwrap();
    println!("{} candidates, {} colour-mixing controls (type 0/1, mixing on)", found.len(), mixing_controls.len());
}
