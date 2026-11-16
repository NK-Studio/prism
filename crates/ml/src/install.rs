//! Installing the Real-ESRGAN runtime for the current user.
//!
//! The upstream package for this platform (from the Real-ESRGAN v0.2.5.0 release) is downloaded
//! over HTTPS, accepted only if its size and SHA-256 match the pins below, and unpacked: just the
//! executable (plus its OpenMP DLLs on Windows) and the `realesrgan-x4plus` and
//! `realesrgan-x4plus-anime` models, into a fresh folder that replaces `<user dir>/realesrgan`
//! in one rename. A `SOURCE.txt` beside them records where they came from and their licences.

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::runtime::{self, DIR_NAME, Model, Runtime};
use crate::{Control, Error, Result, zip};

/// One pinned upstream package.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Package {
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    /// Files unpacked besides the models (the executable first).
    pub files: &'static [&'static str],
}

/// macOS (universal binary; MoltenVK built in).
pub const MACOS: Package = Package {
    url: "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesrgan-ncnn-vulkan-20220424-macos.zip",
    size: 51_817_124,
    sha256: "e0ad05580abfeb25f8d8fb55aaf7bedf552c375b5b4d9bd3c8d59764d2cc333a",
    files: &["realesrgan-ncnn-vulkan"],
};

/// Windows x64.
pub const WINDOWS: Package = Package {
    url: "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesrgan-ncnn-vulkan-20220424-windows.zip",
    size: 45_474_481,
    sha256: "abc02804e17982a3be33675e4d471e91ea374e65b70167abc09e31acb412802d",
    files: &["realesrgan-ncnn-vulkan.exe", "vcomp140.dll", "vcomp140d.dll"],
};

/// The package for this platform, if upstream publishes one.
pub fn package() -> Option<Package> {
    if cfg!(target_os = "macos") {
        Some(MACOS)
    } else if cfg!(windows) {
        Some(WINDOWS)
    } else {
        None
    }
}

const SOURCE_NOTE: &str = "\
Real-ESRGAN runtime installed by Prism.

realesrgan-ncnn-vulkan: Copyright (c) 2021 Xintao Wang, MIT License,
  https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan (built on Tencent/ncnn, BSD-3-Clause).
Models realesrgan-x4plus and realesrgan-x4plus-anime: Copyright (c) 2021 Xintao Wang,
  BSD-3-Clause, https://github.com/xinntao/Real-ESRGAN
";

/// Downloads and installs the runtime into `<user dir>/realesrgan` (see the module docs).
/// Needs [`runtime::configure`] to have been called with a user directory.
pub fn install(ctl: &dyn Control) -> Result<Runtime> {
    let target = runtime::user_runtime_dir()
        .ok_or_else(|| Error::Unsupported("installing Real-ESRGAN is only available in the Prism app (no per-user folder is configured)".into()))?;
    let pkg = package().ok_or_else(|| Error::Unsupported("Real-ESRGAN publishes no package for this platform".into()))?;
    // One install at a time; a second caller finds the first one's result.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(rt) = Runtime::at(&target).filter(|rt| Model::ALL.iter().all(|m| rt.has_model(*m))) {
        return Ok(rt);
    }
    let bytes = download(&pkg, &crate::Stage { outer: ctl, from: 0.0, to: 0.9 })?;
    ctl.progress(0.9, "Installing Real-ESRGAN");
    unpack(&bytes, &pkg, &target)?;
    ctl.progress(1.0, "");
    Runtime::at(&target).ok_or_else(|| Error::Io(format!("the runtime was unpacked but {} is missing", runtime::executable_name())))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent(concat!("Prism/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// The verified package bytes.
fn download(pkg: &Package, ctl: &dyn Control) -> Result<Vec<u8>> {
    ctl.progress(0.0, "Downloading Real-ESRGAN");
    let mut resp = agent().get(pkg.url).call().map_err(|e| Error::Network(e.to_string()))?;
    let mut body = resp.body_mut().with_config().limit(pkg.size + 1).reader();
    let mut bytes = Vec::with_capacity(pkg.size as usize);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if ctl.cancelled() {
            return Err(Error::Cancelled);
        }
        let n = body.read(&mut buf).map_err(|e| Error::Network(e.to_string()))?;
        let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else { break };
        hasher.update(chunk);
        bytes.extend_from_slice(chunk);
        if bytes.len() as u64 > pkg.size {
            break;
        }
        ctl.progress(bytes.len() as f32 / pkg.size.max(1) as f32, "Downloading Real-ESRGAN");
    }
    verify(&bytes, &format!("{:x}", hasher.finalize()), pkg)?;
    Ok(bytes)
}

fn verify(bytes: &[u8], sha256_hex: &str, pkg: &Package) -> Result<()> {
    if bytes.len() as u64 != pkg.size {
        return Err(Error::Integrity(format!("the Real-ESRGAN download is {} bytes, expected {}", bytes.len(), pkg.size)));
    }
    if !sha256_hex.eq_ignore_ascii_case(pkg.sha256) {
        return Err(Error::Integrity("the Real-ESRGAN download doesn't match its pinned SHA-256".into()));
    }
    Ok(())
}

/// Unpacks `pkg`'s files and both models from `zip_bytes` into `target`, replacing it.
fn unpack(zip_bytes: &[u8], pkg: &Package, target: &Path) -> Result<()> {
    let parent = target.parent().ok_or_else(|| Error::Io(format!("{} has no parent folder", target.display())))?;
    std::fs::create_dir_all(parent)?;
    let staging = parent.join(format!("{DIR_NAME}.part"));
    let _ = std::fs::remove_dir_all(&staging);
    let result = (|| {
        std::fs::create_dir_all(staging.join("models"))?;
        let entries = zip::entries(zip_bytes)?;
        let wanted: Vec<String> = pkg.files.iter().map(|f| f.to_string()).chain(Model::ALL.iter().flat_map(|m| m.files())).collect();
        for name in &wanted {
            // Upstream zips have no top-level folder; accept one anyway.
            let e = entries
                .iter()
                .find(|e| e.name == *name || e.name.ends_with(&format!("/{name}")))
                .ok_or_else(|| Error::Integrity(format!("the Real-ESRGAN package has no {name}")))?;
            let data = zip::read(zip_bytes, e)?;
            let dest = staging.join(name);
            let mut f = std::fs::File::create(&dest)?;
            f.write_all(&data)?;
            f.sync_all()?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(exe) = pkg.files.first() {
                std::fs::set_permissions(staging.join(exe), std::fs::Permissions::from_mode(0o755))?;
            }
        }
        std::fs::write(staging.join("SOURCE.txt"), format!("{SOURCE_NOTE}\nPackage: {}\nSHA-256: {}\n", pkg.url, pkg.sha256))?;
        if target.exists() {
            std::fs::remove_dir_all(target)?;
        }
        std::fs::rename(&staging, target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_package(files: &'static [&'static str]) -> Package {
        Package { url: "https://example.invalid/x.zip", size: 0, sha256: "", files }
    }

    #[test]
    fn pins_are_well_formed() {
        for p in [MACOS, WINDOWS] {
            assert!(p.url.starts_with("https://github.com/xinntao/Real-ESRGAN/releases/download/"));
            assert_eq!(p.sha256.len(), 64);
            assert!(p.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert_eq!(p.files[0], if p == WINDOWS { "realesrgan-ncnn-vulkan.exe" } else { "realesrgan-ncnn-vulkan" });
        }
    }

    #[test]
    fn verify_checks_size_and_hash() {
        let p = Package { size: 3, sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", ..fake_package(&[]) };
        let digest = format!("{:x}", Sha256::digest(b"abc"));
        assert!(verify(b"abc", &digest, &p).is_ok());
        assert!(verify(b"abd", &format!("{:x}", Sha256::digest(b"abd")), &p).is_err());
        assert!(verify(b"abcd", &digest, &p).is_err());
    }

    #[test]
    fn unpack_takes_only_the_runtime_and_replaces_the_old_copy() {
        let root = std::env::temp_dir().join(format!("prism-ml-unpack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let target = root.join(DIR_NAME);
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("stale.txt"), b"old").unwrap();
        let mut files: Vec<(String, Vec<u8>)> = vec![("tool".into(), b"#!".to_vec()), ("input.jpg".into(), b"demo".to_vec())];
        for m in Model::ALL {
            for f in m.files() {
                files.push((f.clone(), f.into_bytes()));
            }
        }
        let refs: Vec<(&str, &[u8], bool)> = files.iter().map(|(n, d)| (n.as_str(), d.as_slice(), true)).collect();
        let z = zip::tests::build(&refs);
        unpack(&z, &fake_package(&["tool"]), &target).unwrap();
        assert!(target.join("tool").is_file());
        assert!(target.join("models/realesrgan-x4plus-anime.bin").is_file());
        assert!(target.join("SOURCE.txt").is_file());
        assert!(!target.join("input.jpg").exists());
        assert!(!target.join("stale.txt").exists());
        // A package missing a model leaves the installed copy alone.
        let z = zip::tests::build(&[("tool", b"#!", false)]);
        assert!(unpack(&z, &fake_package(&["tool"]), &target).is_err());
        assert!(target.join("models/realesrgan-x4plus.bin").is_file());
        assert!(!root.join(format!("{DIR_NAME}.part")).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The real upstream package, when `PRISM_REALESRGAN_PACKAGE` names a downloaded copy of this
    /// platform's zip: it must match the pins and unpack into a working runtime layout.
    #[test]
    #[ignore = "needs the downloaded upstream package"]
    fn real_package_verifies_and_unpacks() {
        let path = std::env::var_os("PRISM_REALESRGAN_PACKAGE").expect("set PRISM_REALESRGAN_PACKAGE");
        let bytes = std::fs::read(path).unwrap();
        let pkg = package().unwrap();
        verify(&bytes, &format!("{:x}", Sha256::digest(&bytes)), &pkg).unwrap();
        let root = std::env::temp_dir().join(format!("prism-ml-real-{}", std::process::id()));
        let target = root.join(DIR_NAME);
        unpack(&bytes, &pkg, &target).unwrap();
        let rt = Runtime::at(&target).unwrap();
        assert!(Model::ALL.iter().all(|m| rt.has_model(*m)));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_without_a_configured_folder_is_refused() {
        // Tests never call `runtime::configure`, so nothing is downloaded here.
        assert!(matches!(install(&crate::NoControl), Err(Error::Unsupported(_))));
    }
}
