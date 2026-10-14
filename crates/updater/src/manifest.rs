//! The release feed (`latest.json`) and its signature.
//!
//! ```json
//! { "schema": 1, "version": "1.2.0", "pub_date": "2026-10-20", "min_supported": "1.0.0",
//!   "notes_url": "https://…/releases/1.2.0/notes.html", "download_url": "https://…",
//!   "platforms": {
//!     "macos-universal": { "url": "https://…/prism-1.2.0-macos-universal.app.zip",
//!                          "sha256": "…64 hex…", "size": 61234567 },
//!     "windows-x64":     { "url": "https://…/prism-1.2.0-windows-x64.msi", "sha256": "…", "size": 1 } } }
//! ```
//!
//! The bytes are trusted only after their minisign signature verifies against the public key
//! compiled into the app; every package is then pinned by the SHA-256 and size inside them.

use std::collections::BTreeMap;

use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Largest feed we read. A real one is a few hundred bytes.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// Largest package we download.
pub const MAX_PACKAGE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Feed schema versions this build understands.
pub const SCHEMA: u32 = 1;

/// The parsed feed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    /// The newest version, semver (`1.2.0`, `1.3.0-rc.1`).
    pub version: String,
    #[serde(default)]
    pub pub_date: Option<String>,
    /// Builds older than this can't update in place (the server no longer keeps a path from them);
    /// they are sent to `download_url` instead.
    #[serde(default)]
    pub min_supported: Option<String>,
    #[serde(default)]
    pub notes_url: Option<String>,
    /// Page for a manual download (portable builds, unsupported platforms, too-old builds).
    #[serde(default)]
    pub download_url: Option<String>,
    pub platforms: BTreeMap<String, Asset>,
}

/// One downloadable package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    pub url: String,
    /// Lower-case hex SHA-256 of the file.
    pub sha256: String,
    /// Exact size in bytes.
    pub size: u64,
}

/// An update this build can take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    pub current: String,
    pub version: String,
    pub pub_date: Option<String>,
    pub notes_url: Option<String>,
    pub download_url: Option<String>,
    /// The feed key that matched (`macos-universal`, `windows-x64`, …).
    pub platform: String,
    /// The package, or `None` when only a manual download is possible.
    pub asset: Option<Asset>,
    /// True when this build is older than the feed's `min_supported`.
    pub manual_only: bool,
}

/// Verifies `signature` (a minisign `.minisig` file) over `bytes` and parses the feed.
pub fn verify_manifest(bytes: &[u8], signature: &str, public_key: &str) -> Result<Manifest> {
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(Error::BadManifest(format!("{} bytes is too large", bytes.len())));
    }
    let key = decode_key(public_key)?;
    let sig = Signature::decode(signature).map_err(|e| Error::BadSignature(e.to_string()))?;
    key.verify(bytes, &sig, false).map_err(|e| Error::BadSignature(e.to_string()))?;
    let manifest: Manifest = serde_json::from_slice(bytes).map_err(|e| Error::BadManifest(e.to_string()))?;
    validate(&manifest)?;
    Ok(manifest)
}

fn decode_key(public_key: &str) -> Result<PublicKey> {
    let trimmed = public_key.trim();
    let parsed = if trimmed.contains('\n') { PublicKey::decode(trimmed) } else { PublicKey::from_base64(trimmed) };
    parsed.map_err(|e| Error::BadSignature(format!("public key: {e}")))
}

fn validate(m: &Manifest) -> Result<()> {
    if m.schema != SCHEMA {
        return Err(Error::BadManifest(format!("unsupported schema {}", m.schema)));
    }
    parse_version(&m.version)?;
    if let Some(min) = &m.min_supported {
        parse_version(min)?;
    }
    for (key, asset) in &m.platforms {
        if !is_trusted_url(&asset.url) {
            return Err(Error::BadManifest(format!("{key}: package URL must be https")));
        }
        if asset.sha256.len() != 64 || !asset.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::BadManifest(format!("{key}: sha256 must be 64 hex digits")));
        }
        if asset.size == 0 || asset.size > MAX_PACKAGE_BYTES {
            return Err(Error::BadManifest(format!("{key}: size {} is out of range", asset.size)));
        }
    }
    Ok(())
}

/// Packages come over https; plain http is allowed only from this machine (a local test feed).
pub fn is_trusted_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("https://") || ["http://localhost:", "http://localhost/", "http://127.0.0.1:", "http://127.0.0.1/"].iter().any(|p| lower.starts_with(p))
}

fn parse_version(v: &str) -> Result<semver::Version> {
    semver::Version::parse(v.trim()).map_err(|e| Error::BadManifest(format!("version {v:?}: {e}")))
}

/// Compares a verified feed with the running version. `Ok(None)` means up to date (or the feed
/// is older, e.g. after a rollback: we never downgrade).
pub fn evaluate(manifest: &Manifest, current: &str, platform_keys: &[&str]) -> Result<Option<Update>> {
    let latest = parse_version(&manifest.version)?;
    let running = parse_version(current)?;
    if latest <= running {
        return Ok(None);
    }
    let manual_only = match &manifest.min_supported {
        Some(min) => running < parse_version(min)?,
        None => false,
    };
    let found = platform_keys.iter().find_map(|k| manifest.platforms.get(*k).map(|a| (*k, a)));
    let (platform, asset) = match found {
        Some((k, a)) => (k.to_owned(), Some(a.clone())),
        None => (platform_keys.first().copied().unwrap_or("unknown").to_owned(), None),
    };
    if asset.is_none() && manifest.download_url.is_none() {
        return Err(Error::NoAssetForPlatform(platform));
    }
    Ok(Some(Update {
        current: current.to_owned(),
        version: manifest.version.clone(),
        pub_date: manifest.pub_date.clone(),
        notes_url: manifest.notes_url.clone(),
        download_url: manifest.download_url.clone(),
        platform,
        asset: if manual_only { None } else { asset },
        manual_only,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn feed(version: &str) -> String {
        format!(
            r#"{{"schema":1,"version":"{version}","min_supported":"1.0.0","download_url":"https://example.com/dl",
               "platforms":{{"macos-universal":{{"url":"https://example.com/a.app.zip","sha256":"{HASH}","size":10}},
                             "windows-x64":{{"url":"https://example.com/a.msi","sha256":"{HASH}","size":10}}}}}}"#
        )
    }

    /// Signs `data` with a fresh key; returns (public key base64, .minisig text).
    fn sign(data: &[u8]) -> (String, String) {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let sig = minisign::sign(Some(&kp.pk), &kp.sk, std::io::Cursor::new(data), Some("prism test"), None).unwrap();
        (kp.pk.to_base64(), sig.to_string())
    }

    #[test]
    fn accepts_a_signed_feed() {
        let bytes = feed("1.2.0");
        let (pk, sig) = sign(bytes.as_bytes());
        let m = verify_manifest(bytes.as_bytes(), &sig, &pk).unwrap();
        assert_eq!(m.version, "1.2.0");
        let up = evaluate(&m, "1.1.0", &["macos-aarch64", "macos-universal"]).unwrap().unwrap();
        assert_eq!(up.platform, "macos-universal");
        assert!(up.asset.is_some() && !up.manual_only);
    }

    #[test]
    fn rejects_tampered_bytes_and_foreign_keys() {
        let bytes = feed("1.2.0");
        let (pk, sig) = sign(bytes.as_bytes());
        let tampered = bytes.replace("1.2.0", "9.9.9");
        assert!(matches!(verify_manifest(tampered.as_bytes(), &sig, &pk), Err(Error::BadSignature(_))));
        let (other_pk, _) = sign(b"x");
        assert!(matches!(verify_manifest(bytes.as_bytes(), &sig, &other_pk), Err(Error::BadSignature(_))));
        assert!(verify_manifest(bytes.as_bytes(), "garbage", &pk).is_err());
        assert!(verify_manifest(bytes.as_bytes(), &sig, "not a key").is_err());
    }

    #[test]
    fn up_to_date_and_rollbacks_are_not_updates() {
        let bytes = feed("1.1.0");
        let (pk, sig) = sign(bytes.as_bytes());
        let m = verify_manifest(bytes.as_bytes(), &sig, &pk).unwrap();
        assert_eq!(evaluate(&m, "1.1.0", &["windows-x64"]).unwrap(), None);
        assert_eq!(evaluate(&m, "1.2.0", &["windows-x64"]).unwrap(), None);
        assert!(evaluate(&m, "1.1.0-rc.1", &["windows-x64"]).unwrap().is_some());
    }

    #[test]
    fn too_old_builds_get_a_manual_download() {
        let bytes = feed("1.4.0").replace(r#""min_supported":"1.0.0""#, r#""min_supported":"1.2.0""#);
        let (pk, sig) = sign(bytes.as_bytes());
        let m = verify_manifest(bytes.as_bytes(), &sig, &pk).unwrap();
        let up = evaluate(&m, "1.1.0", &["windows-x64"]).unwrap().unwrap();
        assert!(up.manual_only && up.asset.is_none());
    }

    #[test]
    fn unknown_platform_falls_back_to_the_download_page() {
        let bytes = feed("1.2.0");
        let (pk, sig) = sign(bytes.as_bytes());
        let m = verify_manifest(bytes.as_bytes(), &sig, &pk).unwrap();
        let up = evaluate(&m, "1.1.0", &["freebsd-x86_64"]).unwrap().unwrap();
        assert!(up.asset.is_none());
        assert_eq!(up.download_url.as_deref(), Some("https://example.com/dl"));
    }

    #[test]
    fn hostile_feeds_are_errors() {
        let cases = [
            feed("1.2.0").replace("https://example.com/a.msi", "http://evil.example/a.msi"),
            feed("1.2.0").replace(HASH, "zz"),
            feed("1.2.0").replace(r#""size":10}}}"#, r#""size":0}}}"#),
            feed("1.2.0").replace(r#""size":10}}}"#, r#""size":99999999999999}}}"#),
            feed("not-semver"),
            feed("1.2.0").replace(r#""schema":1"#, r#""schema":7"#),
            r#"{"schema":1}"#.to_owned(),
            "[]".to_owned(),
        ];
        for bytes in cases {
            let (pk, sig) = sign(bytes.as_bytes());
            assert!(verify_manifest(bytes.as_bytes(), &sig, &pk).is_err(), "{bytes}");
        }
        let huge = vec![b' '; (MAX_MANIFEST_BYTES + 1) as usize];
        assert!(verify_manifest(&huge, "", "").is_err());
    }

    #[test]
    fn bad_running_version_is_an_error_not_a_panic() {
        let bytes = feed("1.2.0");
        let (pk, sig) = sign(bytes.as_bytes());
        let m = verify_manifest(bytes.as_bytes(), &sig, &pk).unwrap();
        assert!(evaluate(&m, "", &[]).is_err());
    }

    #[test]
    fn local_http_only() {
        assert!(is_trusted_url("https://x.vercel.app/a"));
        assert!(is_trusted_url("http://localhost:8000/a"));
        assert!(is_trusted_url("http://127.0.0.1/a"));
        assert!(!is_trusted_url("http://localhost.evil.com/a"));
        assert!(!is_trusted_url("file:///etc/passwd"));
    }
}
