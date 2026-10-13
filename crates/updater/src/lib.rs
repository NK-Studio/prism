//! # prism-updater
//!
//! In-app updates for the desktop app:
//!
//! 1. [`check`] fetches the release feed (`latest.json` plus its minisign signature), verifies the
//!    signature against the public key compiled into this build and compares versions.
//! 2. [`download`] streams the platform's package to disk and checks its size and SHA-256 against
//!    the signed manifest.
//! 3. [`install`] replaces the whole app after Prism exits: on macOS the signed `Prism.app` bundle
//!    is swapped atomically (never patched file by file, which would break its code signature); on
//!    Windows the MSI runs as a major upgrade.
//!
//! The feed URL and public key are build inputs (see `build.rs`); a build without them reports
//! [`Error::NotConfigured`] and the app simply offers no updates. Everything returns `Result`:
//! a hostile or broken feed, a network failure or a full disk is an error, never a panic.
//!
//! L6, no UI-toolkit dependencies.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod install;
pub mod manifest;
pub mod net;
pub mod platform;

pub use install::{InstallTarget, Prepared, detect_install_target, install, prepare};
pub use manifest::{Asset, Manifest, Update, evaluate, verify_manifest};
pub use net::{check, download};

/// Feed URL compiled into this build (`PRISM_UPDATE_URL` or `packaging/update/feed-url.txt`).
const BUILD_FEED_URL: Option<&str> = option_env!("PRISM_UPDATE_URL_BUILD");
/// Minisign public key compiled into this build (`PRISM_UPDATE_PUBKEY` or
/// `packaging/update/public.key`).
const BUILD_PUBLIC_KEY: Option<&str> = option_env!("PRISM_UPDATE_PUBKEY_BUILD");

/// Errors from checking, downloading or installing an update.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("updates are not configured in this build")]
    NotConfigured,
    #[error("couldn't reach the update server: {0}")]
    Network(String),
    #[error("the update feed's signature is invalid: {0}")]
    BadSignature(String),
    #[error("the update feed is malformed: {0}")]
    BadManifest(String),
    #[error("no update package for this platform ({0})")]
    NoAssetForPlatform(String),
    #[error("the download is larger than the feed says ({0} bytes)")]
    TooLarge(u64),
    #[error("the download is corrupt (SHA-256 mismatch)")]
    Checksum,
    #[error("the downloaded app failed verification: {0}")]
    Verify(String),
    #[error("cancelled")]
    Cancelled,
    #[error("this installation can't update itself: {0}")]
    Unsupported(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Where to look for updates and which key must have signed them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// URL of `latest.json`; the signature is fetched from the same URL plus `.minisig`.
    pub feed_url: String,
    /// Minisign public key: the base64 line, or the whole `.pub` file.
    pub public_key: String,
}

impl Config {
    /// The configuration compiled into this build. `PRISM_UPDATE_URL` in the environment overrides
    /// the feed URL at run time (handy for a local test feed); the key never comes from the
    /// environment of a release build, so redirecting the feed can't make Prism trust other files.
    pub fn from_build() -> Result<Config> {
        let feed_url = std::env::var("PRISM_UPDATE_URL").ok().filter(|s| !s.trim().is_empty()).or_else(|| BUILD_FEED_URL.map(str::to_owned));
        let public_key = runtime_key_override().or_else(|| BUILD_PUBLIC_KEY.map(str::to_owned));
        match (feed_url, public_key) {
            (Some(feed_url), Some(public_key)) if !feed_url.trim().is_empty() && !public_key.trim().is_empty() => {
                Ok(Config { feed_url: feed_url.trim().to_owned(), public_key: public_key.trim().to_owned() })
            }
            _ => Err(Error::NotConfigured),
        }
    }

    /// URL of the detached signature next to the feed.
    pub fn signature_url(&self) -> String {
        format!("{}.minisig", self.feed_url)
    }
}

/// Debug builds accept `PRISM_UPDATE_PUBKEY` at run time so a local test feed can be signed with a
/// throwaway key. Release builds ignore it.
fn runtime_key_override() -> Option<String> {
    if cfg!(debug_assertions) { std::env::var("PRISM_UPDATE_PUBKEY").ok().filter(|s| !s.trim().is_empty()) } else { None }
}
