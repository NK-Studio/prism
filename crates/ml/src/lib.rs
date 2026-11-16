//! # prism-ml
//!
//! Machine-learning features (architecture L4, milestone M10).
//!
//! The first one is **Super Zoom**: Real-ESRGAN upscaling through the official
//! `realesrgan-ncnn-vulkan` executable (MIT, <https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan>)
//! and its `realesrgan-x4plus` / `realesrgan-x4plus-anime` models (BSD-3-Clause,
//! <https://github.com/xinntao/Real-ESRGAN>). The executable runs on the GPU through Vulkan
//! (MoltenVK on macOS) as a **child process**, so Prism links no native inference code and stays
//! free of `unsafe`:
//!
//! - [`runtime`] finds an installed runtime (an override directory, a copy bundled beside the
//!   app, or the per-user copy) and knows which models it holds.
//! - [`install`] downloads the pinned upstream package for this platform, checks its size and
//!   SHA-256, and unpacks only the executable and the models into the per-user directory.
//! - [`esrgan`] hands an 8-bit RGBA image to the executable as PNG, follows its progress, kills
//!   it on cancel and checks the result (the executable exits with status 0 even when it fails).
//!
//! Every failure is an [`Error`]; nothing here panics on any input.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod esrgan;
pub mod install;
pub mod runtime;
mod zip;

pub use esrgan::{NATIVE_SCALE, Rgba8, upscale};
pub use runtime::{Model, Runtime};

/// Everything that can go wrong with an ML feature.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// No runtime (or not the wanted model) is installed.
    #[error("{0}")]
    NotInstalled(String),
    /// Not available on this platform or in this process.
    #[error("{0}")]
    Unsupported(String),
    #[error("bad input: {0}")]
    BadInput(String),
    #[error("download failed: {0}")]
    Network(String),
    /// A downloaded or unpacked file isn't what was pinned.
    #[error("{0}")]
    Integrity(String),
    #[error("{0}")]
    Io(String),
    /// The executable failed or produced no usable result.
    #[error("{0}")]
    Process(String),
    #[error("cancelled")]
    Cancelled,
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// What a long operation reports to and asks of its caller: a progress fraction (`0..=1`) with a
/// short message, and whether to stop.
pub trait Control {
    fn progress(&self, fraction: f32, message: &str);
    fn cancelled(&self) -> bool;
}

/// A [`Control`] that reports nothing and never cancels.
pub struct NoControl;

impl Control for NoControl {
    fn progress(&self, _: f32, _: &str) {}
    fn cancelled(&self) -> bool {
        false
    }
}

/// Maps a sub-step's `0..=1` progress into `[from, to]` of an outer one.
pub(crate) struct Stage<'a> {
    pub outer: &'a dyn Control,
    pub from: f32,
    pub to: f32,
}

impl Control for Stage<'_> {
    fn progress(&self, fraction: f32, message: &str) {
        let f = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.0 };
        self.outer.progress(self.from + (self.to - self.from) * f, message);
    }
    fn cancelled(&self) -> bool {
        self.outer.cancelled()
    }
}
