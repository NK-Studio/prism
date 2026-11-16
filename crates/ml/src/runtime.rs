//! Where the Real-ESRGAN runtime lives and what it holds.
//!
//! A runtime directory holds the executable (`realesrgan-ncnn-vulkan`, `.exe` on Windows, with
//! its OpenMP DLLs) and a `models/` folder of ncnn `.param`/`.bin` pairs. It is looked for, in
//! order:
//!
//! 1. `PRISM_REALESRGAN_DIR` (tests, agents, custom setups);
//! 2. a copy bundled with the app: `<exe dir>/realesrgan`, or `<exe dir>/../Resources/realesrgan`
//!    inside a macOS bundle;
//! 3. the per-user copy [`install`](crate::install) writes: `<user dir>/realesrgan`, where the
//!    user directory is the one the app passed to [`configure`]. Processes that never call
//!    [`configure`] (tests, the engine on its own) don't look there and never download.

use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock};

use crate::{Error, Result};

/// Environment override for the runtime directory.
pub const ENV_DIR: &str = "PRISM_REALESRGAN_DIR";
/// Folder name of a runtime, bundled or per-user.
pub const DIR_NAME: &str = "realesrgan";

/// The executable's file name on this platform.
pub fn executable_name() -> &'static str {
    if cfg!(windows) { "realesrgan-ncnn-vulkan.exe" } else { "realesrgan-ncnn-vulkan" }
}

/// A Real-ESRGAN model the runtime can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Model {
    /// `realesrgan-x4plus`: photographs and general images (best quality, slowest; 4× only).
    X4Plus,
    /// `realesrgan-x4plus-anime`: illustrations, anime and line art (4× only).
    X4PlusAnime,
    /// `realesr-animevideov3`: the compact model, about 6× faster than x4plus at 4× and with its
    /// own 2× and 3× networks (so 2× costs a quarter of 4×). Tuned for anime and video, so photos
    /// keep less fine detail than with x4plus.
    AnimeVideoV3,
}

impl Model {
    pub const ALL: [Model; 3] = [Model::X4Plus, Model::X4PlusAnime, Model::AnimeVideoV3];

    /// The model's name for the executable (`-n`) and its file stem in `models/`; also the id
    /// commands use.
    pub fn id(self) -> &'static str {
        match self {
            Model::X4Plus => "realesrgan-x4plus",
            Model::X4PlusAnime => "realesrgan-x4plus-anime",
            Model::AnimeVideoV3 => "realesr-animevideov3",
        }
    }

    /// The factors the model has its own network for.
    pub fn scales(self) -> &'static [u32] {
        match self {
            Model::X4Plus | Model::X4PlusAnime => &[4],
            Model::AnimeVideoV3 => &[2, 3, 4],
        }
    }

    /// The factor to run the model at for a wanted `scale`: `scale` itself when the model has it,
    /// else 4 (the caller resamples the result).
    pub fn native_scale(self, scale: u32) -> u32 {
        if self.scales().contains(&scale) { scale } else { 4 }
    }

    /// English UI label.
    pub fn label(self) -> &'static str {
        match self {
            Model::X4Plus => "Photo (Real-ESRGAN x4plus)",
            Model::X4PlusAnime => "Illustration (Real-ESRGAN x4plus anime)",
            Model::AnimeVideoV3 => "Fast (Real-ESRGAN animevideov3)",
        }
    }

    pub fn parse(id: &str) -> Option<Model> {
        Model::ALL.into_iter().find(|m| m.id() == id)
    }

    /// The files the model needs, relative to the runtime directory (a `.param`/`.bin` pair per
    /// network; the multi-scale model names each pair after its factor).
    pub fn files(self) -> Vec<String> {
        let stems: Vec<String> = match self {
            Model::AnimeVideoV3 => self.scales().iter().map(|s| format!("{}-x{s}", self.id())).collect(),
            _ => vec![self.id().to_string()],
        };
        stems.iter().flat_map(|stem| [format!("models/{stem}.param"), format!("models/{stem}.bin")]).collect()
    }
}

/// The app's settings, set once at start-up by [`configure`].
#[derive(Clone, Debug, Default)]
struct Config {
    user_dir: Option<PathBuf>,
}

static CONFIG: RwLock<Config> = RwLock::new(Config { user_dir: None });

/// Called by the app at start-up: `user_dir` is its per-user data directory (the runtime is
/// installed in `<user_dir>/realesrgan`). Until this is called, nothing is installed or looked
/// for there.
pub fn configure(user_dir: Option<PathBuf>) {
    let mut c = CONFIG.write().unwrap_or_else(PoisonError::into_inner);
    c.user_dir = user_dir;
}

/// The per-user runtime directory, if the app configured one.
pub fn user_runtime_dir() -> Option<PathBuf> {
    CONFIG.read().unwrap_or_else(PoisonError::into_inner).user_dir.as_ref().map(|d| d.join(DIR_NAME))
}

/// An installed runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runtime {
    pub dir: PathBuf,
}

impl Runtime {
    /// A runtime at `dir`, if its executable is there.
    pub fn at(dir: &Path) -> Option<Runtime> {
        dir.join(executable_name()).is_file().then(|| Runtime { dir: dir.to_path_buf() })
    }

    pub fn executable(&self) -> PathBuf {
        self.dir.join(executable_name())
    }

    pub fn models_dir(&self) -> PathBuf {
        self.dir.join("models")
    }

    pub fn has_model(&self, m: Model) -> bool {
        m.files().iter().all(|f| self.dir.join(f).is_file())
    }

    /// The first runtime found (see the module docs) that holds `model`, if any.
    pub fn find(model: Model) -> Option<Runtime> {
        candidates().into_iter().filter_map(|d| Runtime::at(&d)).find(|r| r.has_model(model))
    }

    /// [`Runtime::find`], or an error that says where it was looked for.
    pub fn require(model: Model) -> Result<Runtime> {
        Runtime::find(model).ok_or_else(|| {
            let looked: Vec<String> = candidates().iter().map(|d| d.display().to_string()).collect();
            Error::NotInstalled(format!(
                "Real-ESRGAN ({}) is not installed (looked in: {})",
                model.id(),
                if looked.is_empty() { "nowhere".to_string() } else { looked.join(", ") }
            ))
        })
    }
}

/// The directories searched, in order.
pub fn candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = std::env::var_os(ENV_DIR).filter(|d| !d.is_empty()) {
        v.push(PathBuf::from(d));
    }
    if let Some(exe_dir) = std::env::current_exe().ok().as_deref().and_then(Path::parent) {
        v.push(exe_dir.join(DIR_NAME));
        if cfg!(target_os = "macos") {
            v.push(exe_dir.join("..").join("Resources").join(DIR_NAME));
        }
    }
    v.extend(user_runtime_dir());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_ids_round_trip() {
        for m in Model::ALL {
            assert_eq!(Model::parse(m.id()), Some(m));
            assert!(m.files()[0].ends_with(".param"));
            assert_eq!(m.files().len(), m.scales().len() * 2);
            assert_eq!(m.native_scale(4), 4);
        }
        assert_eq!(Model::AnimeVideoV3.native_scale(2), 2);
        assert_eq!(Model::X4Plus.native_scale(2), 4);
        assert!(Model::AnimeVideoV3.files().contains(&"models/realesr-animevideov3-x3.bin".to_string()));
        assert_eq!(Model::parse("realesr-animevideov3-x2"), None);
        assert_eq!(Model::parse(""), None);
    }

    #[test]
    fn runtime_needs_executable_and_model_files() {
        let dir = std::env::temp_dir().join(format!("prism-ml-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("models")).unwrap();
        assert!(Runtime::at(&dir).is_none());
        std::fs::write(dir.join(executable_name()), b"").unwrap();
        let rt = Runtime::at(&dir).unwrap();
        assert!(!rt.has_model(Model::X4Plus));
        for f in Model::X4Plus.files() {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        assert!(rt.has_model(Model::X4Plus));
        assert!(!rt.has_model(Model::X4PlusAnime));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
