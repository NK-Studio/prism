//! Fetching the feed and downloading packages.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::manifest::{self, Asset, MAX_MANIFEST_BYTES, Update};
use crate::{Config, Error, Result, platform};

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(20)))
        .user_agent(concat!("Prism/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn get_small(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>> {
    let mut resp = agent.get(url).call().map_err(|e| Error::Network(e.to_string()))?;
    resp.body_mut().with_config().limit(MAX_MANIFEST_BYTES).read_to_vec().map_err(|e| Error::Network(e.to_string()))
}

/// Fetches and verifies the feed, then compares it with `current` (normally
/// `prism_engine::build_info::VERSION`). `Ok(None)`: up to date.
pub fn check(config: &Config, current: &str) -> Result<Option<Update>> {
    let agent = agent();
    let bytes = get_small(&agent, &config.feed_url)?;
    let sig = get_small(&agent, &config.signature_url())?;
    let sig = String::from_utf8(sig).map_err(|_| Error::BadSignature("signature is not text".into()))?;
    let manifest = manifest::verify_manifest(&bytes, &sig, &config.public_key)?;
    manifest::evaluate(&manifest, current, &platform::platform_keys())
}

/// Downloads `asset` into `dir`, reporting `(received, total)` and stopping early when `cancel`
/// is set. The file is accepted only if its size and SHA-256 match the signed feed; otherwise it
/// is deleted and an error returned.
pub fn download(asset: &Asset, dir: &Path, cancel: &AtomicBool, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf> {
    if !manifest::is_trusted_url(&asset.url) {
        return Err(Error::BadManifest("package URL must be https".into()));
    }
    std::fs::create_dir_all(dir)?;
    let name = file_name(&asset.url);
    let dest = dir.join(&name);
    let partial = dir.join(format!("{name}.part"));
    let result = fetch_to(asset, &partial, cancel, &mut progress);
    match result {
        Ok(()) => {
            std::fs::rename(&partial, &dest)?;
            Ok(dest)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            Err(e)
        }
    }
}

fn fetch_to(asset: &Asset, path: &Path, cancel: &AtomicBool, progress: &mut impl FnMut(u64, u64)) -> Result<()> {
    let agent = agent();
    let mut resp = agent.get(&asset.url).call().map_err(|e| Error::Network(e.to_string()))?;
    // One byte over the limit tells "too large" apart from "exactly right".
    let mut reader = resp.body_mut().with_config().limit(asset.size.saturating_add(1)).reader();
    let mut file = std::fs::File::create(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut received: u64 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(|e| Error::Network(e.to_string()))?;
        if n == 0 {
            break;
        }
        let chunk = buf.get(..n).ok_or(Error::Network("short read".into()))?;
        received = received.saturating_add(n as u64);
        if received > asset.size {
            return Err(Error::TooLarge(received));
        }
        hasher.update(chunk);
        file.write_all(chunk)?;
        progress(received, asset.size);
    }
    file.sync_all()?;
    if received != asset.size || !hex(&hasher.finalize()).eq_ignore_ascii_case(&asset.sha256) {
        return Err(Error::Checksum);
    }
    Ok(())
}

/// The last path segment of `url`, reduced to safe file-name characters.
fn file_name(url: &str) -> String {
    let last = url.split(['?', '#']).next().unwrap_or("").rsplit('/').next().unwrap_or("");
    let clean: String = last.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).collect();
    let clean = clean.trim_start_matches('.').to_owned();
    if clean.is_empty() { "prism-update".to_owned() } else { clean }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_sanitised() {
        assert_eq!(file_name("https://x/releases/1.2.0/prism-1.2.0-macos-universal.app.zip?t=1"), "prism-1.2.0-macos-universal.app.zip");
        assert_eq!(file_name("https://x/../../etc/"), "prism-update");
        assert_eq!(file_name("https://x/..%2F..%2Fpasswd"), "2F..2Fpasswd");
        assert_eq!(file_name(""), "prism-update");
    }

    #[test]
    fn untrusted_url_is_refused_before_any_io() {
        let asset = Asset { url: "http://evil.example/a".into(), sha256: "0".repeat(64), size: 1 };
        let dir = std::env::temp_dir().join("prism-updater-test-refuse");
        assert!(download(&asset, &dir, &AtomicBool::new(false), |_, _| {}).is_err());
    }

    #[test]
    fn sha256_hex() {
        assert_eq!(hex(&Sha256::digest(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
