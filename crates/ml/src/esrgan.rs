//! Super Zoom: one Real-ESRGAN pass through the `realesrgan-ncnn-vulkan` child process.
//!
//! The image goes to the executable as an 8-bit RGBA PNG in a private temporary folder and comes
//! back `scale`× larger. The x4plus models only have a 4× network: asking the executable for
//! `-s 2` or `-s 3` with them produces scrambled tiles, so callers wanting another factor run them
//! at [`Model::native_scale`] and resample; `realesr-animevideov3` has 2×, 3× and 4× networks.
//! The executable reports progress as `NN.NN%` lines on stderr, exits with status 0 even when it
//! could not read or write an image, and prints its errors to stderr, so the result is always
//! checked (present, decodable, exactly 4× the input).

use std::io::{BufRead, BufReader, BufWriter, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use crate::runtime::{Model, Runtime};
use crate::{Control, Error, Result};

/// The largest factor any model runs at.
pub const NATIVE_SCALE: u32 = 4;
/// Largest result accepted, in pixels (16384² RGBA8 is 1 GiB).
pub const MAX_OUTPUT_PIXELS: u64 = 1 << 28;
/// Largest result side (the PNG hand-off and Prism's documents stay well inside it).
pub const MAX_OUTPUT_SIDE: u32 = 65_536;
/// Lines of the executable's stderr kept for error messages.
const TAIL_LINES: usize = 12;

/// An 8-bit image with straight (not premultiplied) alpha, row-major RGBA.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba8 {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Rgba8 {
    /// Checks the buffer length against the size.
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Result<Rgba8> {
        let want = (width as u64).checked_mul(height as u64).and_then(|n| n.checked_mul(4));
        if width == 0 || height == 0 || want != Some(data.len() as u64) {
            return Err(Error::BadInput(format!("an RGBA image of {width}×{height} needs {want:?} bytes, got {}", data.len())));
        }
        Ok(Rgba8 { width, height, data })
    }
}

/// The size of the `scale`× result of a `width`×`height` image, if it stays inside the limits.
pub fn output_size(width: u32, height: u32, scale: u32) -> Result<(u32, u32)> {
    let (w, h) = (width.checked_mul(scale), height.checked_mul(scale));
    match (w, h) {
        (Some(w), Some(h)) if w > 0 && h > 0 && w <= MAX_OUTPUT_SIDE && h <= MAX_OUTPUT_SIDE && (w as u64) * (h as u64) <= MAX_OUTPUT_PIXELS => Ok((w, h)),
        _ => Err(Error::BadInput(format!(
            "Super Zoom of {width}×{height} px would be larger than {} megapixels; select a smaller area",
            MAX_OUTPUT_PIXELS / 1_000_000
        ))),
    }
}

/// Where an [`upscale`] spent its time, in milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Timings {
    /// Writing the input PNG.
    pub encode_ms: f64,
    /// The executable, start to exit (model load, GPU set-up, inference, writing its PNG).
    pub run_ms: f64,
    /// Reading the result PNG.
    pub decode_ms: f64,
}

/// Upscales `img` `scale`× with `model` on `rt` (`scale` must be one of [`Model::scales`]).
/// Progress goes to `ctl` from 0 to 1; cancelling kills the executable.
pub fn upscale(rt: &Runtime, model: Model, scale: u32, img: &Rgba8, ctl: &dyn Control) -> Result<(Rgba8, Timings)> {
    if !model.scales().contains(&scale) {
        return Err(Error::BadInput(format!("{} has no {scale}× network", model.id())));
    }
    if (img.data.len() as u64) != (img.width as u64) * (img.height as u64) * 4 || img.width == 0 || img.height == 0 {
        return Err(Error::BadInput(format!("an RGBA image of {}×{} needs {} bytes, got {}", img.width, img.height, (img.width as u64) * (img.height as u64) * 4, img.data.len())));
    }
    let (ow, oh) = output_size(img.width, img.height, scale)?;
    if !rt.has_model(model) {
        return Err(Error::NotInstalled(format!("the Real-ESRGAN model {} is not in {}", model.id(), rt.models_dir().display())));
    }
    let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;
    let mut timings = Timings::default();
    let work = TempDir::new()?;
    let input = work.path.join("in.png");
    let output = work.path.join("out.png");
    ctl.progress(0.0, "Preparing");
    let t = std::time::Instant::now();
    write_png(&input, img)?;
    timings.encode_ms = ms(t);
    if ctl.cancelled() {
        return Err(Error::Cancelled);
    }
    let t = std::time::Instant::now();
    let tail = run(rt, model, scale, &input, &output, ctl)?;
    timings.run_ms = ms(t);
    if !output.is_file() {
        return Err(Error::Process(format!("Real-ESRGAN produced no image{}", describe(&tail))));
    }
    let t = std::time::Instant::now();
    let out = read_png(&output, ow, oh).map_err(|e| Error::Process(format!("Real-ESRGAN's result is unusable ({e}){}", describe(&tail))))?;
    timings.decode_ms = ms(t);
    ctl.progress(1.0, "");
    Ok((out, timings))
}

/// Runs the executable to completion (or cancellation) and returns the tail of its stderr.
fn run(rt: &Runtime, model: Model, scale: u32, input: &Path, output: &Path, ctl: &dyn Control) -> Result<Vec<String>> {
    let mut cmd = Command::new(rt.executable());
    cmd.arg("-i")
        .arg(input)
        .arg("-o")
        .arg(output)
        .arg("-n")
        .arg(model.id())
        .arg("-s")
        .arg(scale.to_string())
        .arg("-m")
        .arg(rt.models_dir())
        .arg("-f")
        .arg("png")
        .current_dir(&rt.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console window flashes up over the app.
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| Error::Process(format!("couldn't start {}: {e}", rt.executable().display())))?;
    let (tx, rx) = mpsc::channel::<f32>();
    let reader = child.stderr.take().map(|err| std::thread::Builder::new().name("realesrgan-stderr".into()).spawn(move || read_stderr(err, &tx)));
    let status = loop {
        for f in rx.try_iter() {
            ctl.progress(f, "Super Zoom");
        }
        if ctl.cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Process(format!("lost track of Real-ESRGAN: {e}")));
            }
        }
    };
    let tail = match reader {
        Some(Ok(handle)) => handle.join().unwrap_or_default(),
        _ => Vec::new(),
    };
    for f in rx.try_iter() {
        ctl.progress(f, "Super Zoom");
    }
    if !status.success() {
        return Err(Error::Process(format!("Real-ESRGAN failed ({status}){}", describe(&tail))));
    }
    Ok(tail)
}

/// Forwards progress lines and keeps the last other lines.
fn read_stderr(err: impl Read, tx: &mpsc::Sender<f32>) -> Vec<String> {
    let mut tail: Vec<String> = Vec::new();
    let mut r = BufReader::new(err);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match r.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        // Some builds separate progress with carriage returns.
        for part in String::from_utf8_lossy(&buf).split(['\r', '\n']) {
            let line = part.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(f) = parse_percent(line) {
                let _ = tx.send(f);
            } else {
                if tail.len() == TAIL_LINES {
                    tail.remove(0);
                }
                tail.push(line.chars().take(300).collect());
            }
        }
    }
    tail
}

/// `"37.50%"` → `0.375`.
pub(crate) fn parse_percent(line: &str) -> Option<f32> {
    let v: f32 = line.trim().strip_suffix('%')?.trim().parse().ok()?;
    v.is_finite().then(|| (v / 100.0).clamp(0.0, 1.0))
}

/// The useful part of the executable's stderr (its GPU banner lines start with `[`).
fn describe(tail: &[String]) -> String {
    let lines: Vec<&str> = tail.iter().map(String::as_str).filter(|l| !l.starts_with('[')).collect();
    if lines.is_empty() { String::new() } else { format!(": {}", lines.join("; ")) }
}

/// Writes `img` for the executable: as RGB when every pixel is opaque, since the executable runs
/// an RGBA image's alpha through a second pass (more than doubling its time on a photo).
fn write_png(path: &Path, img: &Rgba8) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(BufWriter::new(file), img.width, img.height);
    let opaque = img.data.as_chunks::<4>().0.iter().all(|p| p[3] == 255);
    let rgb: Vec<u8>;
    let data: &[u8] = if opaque {
        rgb = img.data.as_chunks::<4>().0.iter().flat_map(|&[r, g, b, _]| [r, g, b]).collect();
        &rgb
    } else {
        &img.data
    };
    enc.set_color(if opaque { png::ColorType::Rgb } else { png::ColorType::Rgba });
    enc.set_depth(png::BitDepth::Eight);
    // Speed over size: the file lives for one run.
    enc.set_compression(png::Compression::Fastest);
    let mut w = enc.write_header().map_err(|e| Error::Io(format!("writing the Super Zoom input: {e}")))?;
    w.write_image_data(data).map_err(|e| Error::Io(format!("writing the Super Zoom input: {e}")))?;
    w.finish().map_err(|e| Error::Io(format!("writing the Super Zoom input: {e}")))
}

/// Reads an 8-bit PNG that must be exactly `w`×`h`, as RGBA.
fn read_png(path: &Path, w: u32, h: u32) -> std::result::Result<Rgba8, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let limit = (w as usize).saturating_mul(h as usize).saturating_mul(8).saturating_add(1 << 20);
    let mut dec = png::Decoder::new_with_limits(BufReader::new(file), png::Limits { bytes: limit });
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    dec.set_ignore_text_chunk(true);
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let (rw, rh) = reader.info().size();
    if (rw, rh) != (w, h) {
        return Err(format!("expected {w}×{h} px, got {rw}×{rh}"));
    }
    let (color, depth) = reader.output_color_type();
    if depth != png::BitDepth::Eight {
        return Err(format!("unexpected bit depth {depth:?}"));
    }
    let size = reader.output_buffer_size().ok_or("image too large")?;
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(info.buffer_size());
    let rgba: Vec<u8> = match color {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.as_chunks::<2>().0.iter().flat_map(|&[g, a]| [g, g, g, a]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("palette was not expanded".into()),
    };
    Rgba8::new(w, h, rgba).map_err(|e| e.to_string())
}

/// A private folder under the system temp directory, removed when dropped.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Result<TempDir> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        let path = std::env::temp_dir().join(format!("prism-superzoom-{}-{nanos}-{n}", std::process::id()));
        std::fs::create_dir_all(&path)?;
        Ok(TempDir { path })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoControl;

    #[test]
    fn percent_lines_parse() {
        assert_eq!(parse_percent("0.00%"), Some(0.0));
        assert_eq!(parse_percent(" 37.50% "), Some(0.375));
        assert_eq!(parse_percent("150%"), Some(1.0));
        assert_eq!(parse_percent("NaN%"), None);
        assert_eq!(parse_percent("[0 Apple M2 Pro]  queueC=0[1]"), None);
        assert_eq!(parse_percent("%"), None);
    }

    #[test]
    fn sizes_are_checked() {
        assert!(Rgba8::new(2, 2, vec![0; 16]).is_ok());
        assert!(Rgba8::new(2, 2, vec![0; 15]).is_err());
        assert!(Rgba8::new(0, 2, vec![]).is_err());
        assert!(Rgba8::new(u32::MAX, u32::MAX, vec![]).is_err());
        assert_eq!(output_size(100, 50, 4).unwrap(), (400, 200));
        assert_eq!(output_size(100, 50, 2).unwrap(), (200, 100));
        assert!(output_size(4097, 4097, 4).is_err());
        assert!(output_size(u32::MAX, 1, 2).is_err());
        assert!(output_size(0, 1, 4).is_err());
    }

    #[test]
    fn png_round_trip_and_size_check() {
        let dir = TempDir::new().unwrap();
        let p = dir.path.join("t.png");
        let img = Rgba8::new(3, 2, (0..24).collect()).unwrap();
        write_png(&p, &img).unwrap();
        assert_eq!(read_png(&p, 3, 2).unwrap(), img);
        assert!(read_png(&p, 6, 4).is_err());
        // Opaque images travel as RGB and come back opaque.
        let opaque = Rgba8::new(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]).unwrap();
        write_png(&p, &opaque).unwrap();
        assert_eq!(std::fs::read(&p).unwrap()[25], 2, "colour type RGB");
        assert_eq!(read_png(&p, 2, 1).unwrap(), opaque);
        std::fs::write(&p, b"not a png").unwrap();
        assert!(read_png(&p, 3, 2).is_err());
    }

    #[test]
    fn missing_model_is_an_error_not_a_crash() {
        let dir = TempDir::new().unwrap();
        let rt = Runtime { dir: dir.path.clone() };
        let img = Rgba8::new(1, 1, vec![1, 2, 3, 4]).unwrap();
        assert!(matches!(upscale(&rt, Model::X4Plus, 4, &img, &NoControl).map(|r| r.0), Err(Error::NotInstalled(_))));
        let bad = Rgba8 { width: 5, height: 5, data: vec![] };
        assert!(matches!(upscale(&rt, Model::X4Plus, 4, &bad, &NoControl).map(|r| r.0), Err(Error::BadInput(_))));
    }

    /// A stand-in executable (a shell script) that copies a prepared result, fails, or hangs, so
    /// the hand-off, the result checks and cancellation run without a GPU.
    #[cfg(unix)]
    fn fake_runtime(script_body: &str) -> (TempDir, Runtime) {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path.join("models")).unwrap();
        for f in Model::X4Plus.files() {
            std::fs::write(dir.path.join(f), b"x").unwrap();
        }
        let exe = dir.path.join(crate::runtime::executable_name());
        std::fs::write(&exe, format!("#!/bin/sh\n{script_body}\n")).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let rt = Runtime { dir: dir.path.clone() };
        (dir, rt)
    }

    #[cfg(unix)]
    #[test]
    fn fake_runtime_result_is_checked() {
        let (dir, rt) = fake_runtime(
            r#"out=""; while [ $# -gt 0 ]; do [ "$1" = "-o" ] && out="$2"; shift; done; echo "50.00%" >&2; cp "$(dirname "$0")/result.png" "$out""#,
        );
        let img = Rgba8::new(2, 1, vec![9; 8]).unwrap();
        // Right size: accepted.
        let want = Rgba8::new(8, 4, (0..128).map(|i| i as u8).collect()).unwrap();
        write_png(&dir.path.join("result.png"), &want).unwrap();
        assert_eq!(upscale(&rt, Model::X4Plus, 4, &img, &NoControl).map(|r| r.0).unwrap(), want);
        // Wrong size: rejected.
        write_png(&dir.path.join("result.png"), &img).unwrap();
        assert!(matches!(upscale(&rt, Model::X4Plus, 4, &img, &NoControl).map(|r| r.0), Err(Error::Process(_))));
    }

    #[cfg(unix)]
    #[test]
    fn fake_runtime_that_writes_nothing_reports_its_stderr() {
        let (_dir, rt) = fake_runtime(r#"echo "[0 GPU]  banner" >&2; echo "decode image in.png failed" >&2; exit 0"#);
        let img = Rgba8::new(1, 1, vec![0; 4]).unwrap();
        let e = upscale(&rt, Model::X4Plus, 4, &img, &NoControl).map(|r| r.0).unwrap_err().to_string();
        assert!(e.contains("decode image in.png failed"), "{e}");
        assert!(!e.contains("banner"), "{e}");
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_kills_the_process() {
        struct CancelSoon(std::time::Instant);
        impl Control for CancelSoon {
            fn progress(&self, _: f32, _: &str) {}
            fn cancelled(&self) -> bool {
                self.0.elapsed() > Duration::from_millis(150)
            }
        }
        let (_dir, rt) = fake_runtime("sleep 30");
        let img = Rgba8::new(1, 1, vec![0; 4]).unwrap();
        let t = std::time::Instant::now();
        assert_eq!(upscale(&rt, Model::X4Plus, 4, &img, &CancelSoon(t)).map(|r| r.0), Err(Error::Cancelled));
        assert!(t.elapsed() < Duration::from_secs(10));
    }

    /// The real executable, when `PRISM_REALESRGAN_DIR` points at an unpacked upstream package.
    #[test]
    #[ignore = "needs the realesrgan-ncnn-vulkan package and a Vulkan GPU"]
    fn real_runtime_upscales_4x() {
        let rt = Runtime::require(Model::X4Plus).unwrap();
        let (w, h) = (40u32, 24u32);
        let data: Vec<u8> = (0..w * h).flat_map(|i| [(i % w * 6) as u8, (i / w * 10) as u8, 128, if i % 7 == 0 { 0 } else { 255 }]).collect();
        let img = Rgba8::new(w, h, data).unwrap();
        for m in Model::ALL {
            for &s in m.scales() {
                let (out, t) = upscale(&rt, m, s, &img, &NoControl).unwrap();
                assert_eq!((out.width, out.height), (w * s, h * s), "{m:?} x{s}");
                eprintln!("{m:?} x{s}: {t:?}");
            }
        }
        assert!(upscale(&rt, Model::X4Plus, 2, &img, &NoControl).is_err());
    }
}
