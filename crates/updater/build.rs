//! Bakes the update feed URL and the minisign public key into the build.
//!
//! Each comes from an environment variable when set (CI, packaging scripts), otherwise from a file
//! in `packaging/update/` at the workspace root. When neither exists the updater reports
//! "not configured" at run time; the build itself never fails over it.

use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default()).join("../..");
    let dir = root.join("packaging/update");
    emit("PRISM_UPDATE_URL", &dir.join("feed-url.txt"), "PRISM_UPDATE_URL_BUILD");
    emit("PRISM_UPDATE_PUBKEY", &dir.join("public.key"), "PRISM_UPDATE_PUBKEY_BUILD");
}

fn emit(env: &str, file: &std::path::Path, out: &str) {
    println!("cargo:rerun-if-env-changed={env}");
    println!("cargo:rerun-if-changed={}", file.display());
    let value = std::env::var(env).ok().or_else(|| std::fs::read_to_string(file).ok());
    // Keep only the meaningful line: minisign `.pub` files start with an `untrusted comment:` line.
    let value = value.as_deref().map(|v| v.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("untrusted comment:")).collect::<Vec<_>>().join(""));
    if let Some(v) = value.filter(|v| !v.is_empty()) {
        println!("cargo:rustc-env={out}={v}");
    }
}
