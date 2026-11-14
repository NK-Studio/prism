//! Inject a synthetic pen stroke (pressure, tilt, timing) into a window through Windows Ink.
//!
//! The stroke takes the same native path as a physical pen, so it tests Prism's real input
//! pipeline and can drive another painting application with identical input to make a
//! comparison oracle. The cursor really moves: don't touch the mouse while it runs.
//!
//! ```text
//! cargo run -p prism-tablet --example pen_inject -- --window Prism                # pressure ramp
//! cargo run -p prism-tablet --example pen_inject -- --window "CLIP STUDIO" --ramp 200,300,600
//! cargo run -p prism-tablet --example pen_inject -- --window Prism --file trace.json --origin 120,80 --scale 0.5
//! ```
//!
//! Options:
//! - `--window TITLE`: part of the target window's title (required unless `--list`).
//! - `--ramp X,Y,LEN`: client-pixel start and length of the 0→1→0 pressure ramp (default:
//!   centred, 60 % of the client width).
//! - `--file PATH`: a stroke file instead of the ramp: `ui.penTrace` output (`{"events":[…]}`,
//!   optionally wrapped in `result`), or an array of `[x, y, pressure]` / `{x, y, pressure,
//!   time?, tiltX?, tiltY?, rotation?}` contact samples. Points are mapped to the client area as
//!   `origin + point * scale` (`--origin X,Y`, default 0,0; `--scale S`, default 1).
//! - `--hover N`: 16 ms hover frames before the ramp touches down (default 8; 1 models a pen
//!   that touches right after entering range, as in quick successive strokes).
//! - `--wait MS`: pause after focusing the window (default 800).
//! - `--dry-run`: print the frames, inject nothing.
//! - `--list`: print visible window titles matching `--window` (or all) and exit.

use std::process::ExitCode;

use prism_tablet::synthetic_pen::{HOVER_FRAMES, PenFrame, Phase, Stroke, pressure_ramp_with_hover};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pen_inject: {error}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Default)]
struct Args {
    window: Option<String>,
    ramp: Option<(f64, f64, f64)>,
    file: Option<String>,
    origin: (f64, f64),
    scale: f64,
    wait_ms: u64,
    hover: usize,
    dry_run: bool,
    list: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args { scale: 1.0, wait_ms: 800, hover: HOVER_FRAMES, ..Default::default() };
    let mut it = std::env::args().skip(1);
    let nums = |s: String, n: usize| -> Result<Vec<f64>, String> {
        let v: Vec<f64> = s.split(',').map(|p| p.trim().parse::<f64>().map_err(|_| format!("not a number: {p}"))).collect::<Result<_, _>>()?;
        if v.len() != n || v.iter().any(|x| !x.is_finite()) {
            return Err(format!("expected {n} comma-separated numbers, got `{s}`"));
        }
        Ok(v)
    };
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--window" => args.window = Some(value()?),
            "--ramp" => {
                let v = nums(value()?, 3)?;
                args.ramp = Some((v[0], v[1], v[2]));
            }
            "--file" => args.file = Some(value()?),
            "--origin" => {
                let v = nums(value()?, 2)?;
                args.origin = (v[0], v[1]);
            }
            "--scale" => args.scale = nums(value()?, 1)?[0],
            "--wait" => args.wait_ms = value()?.parse().map_err(|_| "--wait needs milliseconds".to_string())?,
            "--hover" => args.hover = value()?.parse().map_err(|_| "--hover needs a frame count".to_string())?,
            "--dry-run" => args.dry_run = true,
            "--list" => args.list = true,
            other => return Err(format!("unknown option {other} (see the example's doc comment)")),
        }
    }
    if !(args.scale > 0.0 && args.scale <= 100.0) {
        return Err("--scale must be within (0, 100]".into());
    }
    Ok(args)
}

/// Contact samples `(x, y, pressure, delay_ms, tilt_x, tilt_y, rotation)` from a stroke file.
fn load_file(path: &str, origin: (f64, f64), scale: f64) -> Result<Stroke, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    let root = json.get("result").unwrap_or(&json);
    let items = root.get("events").unwrap_or(root).as_array().ok_or("expected an array of samples or {\"events\": [...]}")?;
    let num = |v: Option<&serde_json::Value>| v.and_then(serde_json::Value::as_f64);
    let mut frames = Vec::new();
    let mut last_time: Option<f64> = None;
    let mut contact = false;
    for (i, item) in items.iter().enumerate() {
        let (x, y, pressure, time, kind, tilt) = if let Some(a) = item.as_array() {
            let get = |k: usize| a.get(k).and_then(serde_json::Value::as_f64);
            (get(0), get(1), get(2).unwrap_or(1.0), None, None, (0.0, 0.0, 0.0))
        } else {
            // Trace barrel gestures belong to a different UI path; skip them like a replay does.
            if item.get("barrel").and_then(serde_json::Value::as_bool) == Some(true) {
                continue;
            }
            let tilt = (num(item.get("tiltX")).unwrap_or(0.0), num(item.get("tiltY")).unwrap_or(0.0), num(item.get("rotation")).unwrap_or(0.0));
            (
                num(item.get("x")),
                num(item.get("y")),
                num(item.get("pressure")).unwrap_or(1.0),
                num(item.get("time")),
                item.get("kind").and_then(serde_json::Value::as_str),
                tilt,
            )
        };
        let (Some(x), Some(y)) = (x, y) else { return Err(format!("sample {i} needs x and y")) };
        let delay = match (time, last_time) {
            (Some(t), Some(prev)) => (t - prev).clamp(0.0, 10_000.0).round() as u32,
            _ => 8,
        };
        if time.is_some() {
            last_time = time;
        }
        let phase = match kind {
            Some("hover") => continue,
            Some("cancel") => break,
            Some("down") => Phase::Down,
            Some("up") => Phase::Up,
            _ if !contact => Phase::Down,
            _ => Phase::Move,
        };
        // A trace can start mid-stroke or repeat a down; keep the contact sequence valid.
        let phase = match (phase, contact) {
            (Phase::Down, true) => Phase::Move,
            (Phase::Up, false) => continue,
            (p, _) => p,
        };
        let mut frame = PenFrame::new(phase, origin.0 + x * scale, origin.1 + y * scale, pressure as f32, delay);
        frame.tilt_x = tilt.0 as f32;
        frame.tilt_y = tilt.1 as f32;
        frame.rotation = tilt.2 as f32;
        if phase == Phase::Down {
            for h in 0..HOVER_FRAMES {
                frames.push(PenFrame::new(Phase::Hover, frame.x, frame.y, 0.0, if h == 0 { 0 } else { 16 }));
            }
        }
        contact = matches!(phase, Phase::Down | Phase::Move);
        frames.push(frame);
        if phase == Phase::Up {
            frames.push(PenFrame::new(Phase::Leave, frame.x, frame.y, 0.0, 16));
        }
    }
    if let Some(last) = frames.last().copied().filter(|_| contact) {
        frames.push(PenFrame::new(Phase::Up, last.x, last.y, 0.0, 8));
        frames.push(PenFrame::new(Phase::Leave, last.x, last.y, 0.0, 16));
    }
    Stroke::new(frames).map_err(|e| e.to_string())
}

#[cfg(target_os = "windows")]
fn run() -> Result<(), String> {
    use prism_tablet::windows::{SyntheticPen, bring_to_front, find_window, make_dpi_aware};
    let args = parse_args()?;
    // Window geometry and injected frames must both be physical pixels.
    make_dpi_aware();
    let mut pen = if args.dry_run { None } else { Some(SyntheticPen::new().map_err(|e| e.to_string())?) };
    if args.list {
        let needle = args.window.unwrap_or_default();
        match find_window(&needle).map_err(|e| e.to_string())? {
            Some(t) => println!("{:?}  client {}x{} at ({}, {})", t.title, t.client_width, t.client_height, t.client_x, t.client_y),
            None => println!("no visible window matches {needle:?}"),
        }
        return Ok(());
    }
    let needle = args.window.ok_or("--window TITLE is required")?;
    let target = find_window(&needle).map_err(|e| e.to_string())?.ok_or(format!("no visible window title contains {needle:?}"))?;
    println!("target {:?}: client {}x{} at ({}, {})", target.title, target.client_width, target.client_height, target.client_x, target.client_y);
    let stroke = match &args.file {
        Some(path) => load_file(path, args.origin, args.scale)?,
        None => {
            let (w, h) = (f64::from(target.client_width), f64::from(target.client_height));
            let (x, y, len) = args.ramp.unwrap_or((w * 0.2, h * 0.5, w * 0.6));
            pressure_ramp_with_hover(x, y, len, args.hover).map_err(|e| e.to_string())?
        }
    };
    let stroke = stroke.offset(f64::from(target.client_x), f64::from(target.client_y)).map_err(|e| e.to_string())?;
    let Some(pen) = pen.as_mut() else {
        for f in stroke.frames() {
            println!("{:?}\t{:.1}\t{:.1}\tp={:.3}\t+{} ms", f.phase, f.x, f.y, f.pressure, f.delay_ms);
        }
        return Ok(());
    };
    if !bring_to_front(&target) {
        eprintln!("warning: Windows refused to focus the window; click it once if nothing is drawn");
    }
    std::thread::sleep(std::time::Duration::from_millis(args.wait_ms));
    pen.play(&stroke).map_err(|e| e.to_string())?;
    println!("injected {} frames", stroke.frames().len());
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn run() -> Result<(), String> {
    let args = parse_args()?;
    if let Some(path) = &args.file {
        let stroke = load_file(path, args.origin, args.scale)?;
        println!("parsed {} frames; injection needs Windows", stroke.frames().len());
        return Ok(());
    }
    Err("synthetic pen injection needs Windows (InjectSyntheticPointerInput)".into())
}
