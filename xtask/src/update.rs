//! `cargo xtask update`: signing keys and the in-app update feed (see `crates/updater`).
//!
//! ```text
//! cargo xtask update keygen --secret ~/prism-update.key [--public PATH]
//!     new minisign key pair; the public half goes to packaging/update/public.key (compiled into
//!     the app), the secret half to the given path outside the repository (store it as the
//!     PRISM_UPDATE_SECRET_KEY CI secret).
//! cargo xtask update publish --site <vercel site checkout> --base-url https://prism-updates.vercel.app
//!     [--dist dist/release] [--version X.Y.Z] [--min-supported X.Y.Z] [--notes-url URL]
//!     [--download-url URL] [--secret PATH] [--keep 3]
//!     copies this version's update packages to <site>/releases/<version>/, writes and signs
//!     <site>/stable/latest.json, and removes all but the newest --keep version folders.
//! ```
//!
//! The secret key comes from `--secret PATH` or the `PRISM_UPDATE_SECRET_KEY` environment
//! variable (the key file's text). It is never written inside the repository.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// Update packages produced by the packaging scripts: (file suffix after `prism-<v>-`, feed key).
const PACKAGES: &[(&str, &str)] = &[
    ("macos-universal.app.zip", "macos-universal"),
    ("macos-aarch64.app.zip", "macos-aarch64"),
    ("macos-x86_64.app.zip", "macos-x86_64"),
    ("windows-x64.msi", "windows-x64"),
    ("windows-x86.msi", "windows-x86"),
    ("windows-arm64.msi", "windows-arm64"),
];

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    match args.first().copied() {
        Some("keygen") => keygen(root, args.get(1..).unwrap_or_default()),
        Some("publish") => publish(root, args.get(1..).unwrap_or_default()),
        _ => Err("usage: cargo xtask update keygen --secret PATH | update publish --site DIR --base-url URL [...]".into()),
    }
}

fn flag<'a>(args: &[&'a str], name: &str) -> Option<&'a str> {
    args.iter().position(|a| *a == name).and_then(|i| args.get(i + 1)).copied()
}

fn keygen(root: &Path, args: &[&str]) -> Result<(), String> {
    let secret = PathBuf::from(flag(args, "--secret").ok_or("--secret PATH is required (outside the repository)")?);
    let secret_abs = absolute(&secret)?;
    if secret_abs.starts_with(root) {
        return Err(format!("refusing to write the secret key inside the repository: {}", secret_abs.display()));
    }
    if secret_abs.exists() {
        return Err(format!("{} already exists; move it away first", secret_abs.display()));
    }
    let kp = minisign::KeyPair::generate_unencrypted_keypair().map_err(|e| e.to_string())?;
    let sk = kp.sk.to_box(Some("Prism update signing key")).map_err(|e| e.to_string())?.into_string();
    let pk = kp.pk.to_box().map_err(|e| e.to_string())?.into_string();
    write_private(&secret_abs, &sk)?;
    let pub_path = match flag(args, "--public") {
        Some(p) => absolute(Path::new(p))?,
        None => root.join("packaging/update/public.key"),
    };
    std::fs::create_dir_all(pub_path.parent().ok_or("bad path")?).map_err(|e| e.to_string())?;
    std::fs::write(&pub_path, &pk).map_err(|e| e.to_string())?;
    eprintln!("secret key: {} (keep it safe; CI secret PRISM_UPDATE_SECRET_KEY = its contents)", secret_abs.display());
    eprintln!("public key: {} (commit it; it is compiled into Prism)", pub_path.display());
    Ok(())
}

fn absolute(p: &Path) -> Result<PathBuf, String> {
    let p = if let Some(rest) = p.to_str().and_then(|s| s.strip_prefix("~/")) {
        PathBuf::from(std::env::var("HOME").map_err(|e| e.to_string())?).join(rest)
    } else {
        p.to_path_buf()
    };
    if p.is_absolute() { Ok(p) } else { Ok(std::env::current_dir().map_err(|e| e.to_string())?.join(p)) }
}

#[cfg(unix)]
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(text.as_bytes()).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn secret_key(args: &[&str]) -> Result<minisign::SecretKey, String> {
    let text = match flag(args, "--secret") {
        Some(p) => std::fs::read_to_string(absolute(Path::new(p))?).map_err(|e| format!("{p}: {e}"))?,
        None => std::env::var("PRISM_UPDATE_SECRET_KEY").map_err(|_| "set PRISM_UPDATE_SECRET_KEY or pass --secret PATH")?,
    };
    let sk_box = minisign::SecretKeyBox::from_string(text.trim()).map_err(|e| format!("secret key: {e}"))?;
    sk_box.into_unencrypted_secret_key().map_err(|e| format!("secret key: {e}"))
}

fn publish(root: &Path, args: &[&str]) -> Result<(), String> {
    let site = absolute(Path::new(flag(args, "--site").ok_or("--site DIR is required (the Vercel site checkout)")?))?;
    let base = flag(args, "--base-url").ok_or("--base-url URL is required")?.trim_end_matches('/');
    if !base.starts_with("https://") && !base.starts_with("http://localhost") && !base.starts_with("http://127.0.0.1") {
        return Err("--base-url must be https".into());
    }
    let dist = match flag(args, "--dist") {
        Some(d) => absolute(Path::new(d))?,
        None => std::env::var("DIST").map(PathBuf::from).unwrap_or_else(|_| root.join("dist/release")),
    };
    let version = match flag(args, "--version") {
        Some(v) => v.to_owned(),
        None => crate::version::read(&std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?)?,
    };
    crate::version::validate(&version)?;
    let keep: usize = flag(args, "--keep").map(str::parse).transpose().map_err(|e| format!("--keep: {e}"))?.unwrap_or(3).max(1);
    let sk = secret_key(args)?;

    let release_dir = site.join("releases").join(&version);
    std::fs::create_dir_all(&release_dir).map_err(|e| e.to_string())?;
    let mut platforms = Map::new();
    for (suffix, key) in PACKAGES {
        let name = format!("prism-{version}-{suffix}");
        let src = dist.join(&name);
        if !src.is_file() {
            continue;
        }
        let bytes = std::fs::read(&src).map_err(|e| format!("{}: {e}", src.display()))?;
        std::fs::copy(&src, release_dir.join(&name)).map_err(|e| e.to_string())?;
        platforms.insert((*key).to_owned(), json!({"url": format!("{base}/releases/{version}/{name}"), "sha256": crate::sha256::hex(&bytes), "size": bytes.len()}));
        eprintln!("  {key:<16} {name} ({} bytes)", bytes.len());
    }
    if platforms.is_empty() {
        return Err(format!("no update packages for {version} in {} (expected prism-{version}-macos-*.app.zip or prism-{version}-windows-*.msi)", dist.display()));
    }

    let mut manifest = json!({
        "schema": 1,
        "version": version,
        "pub_date": today(),
        "platforms": Value::Object(platforms),
    });
    for (flag_name, field) in [("--min-supported", "min_supported"), ("--notes-url", "notes_url"), ("--download-url", "download_url")] {
        if let Some(v) = flag(args, flag_name) {
            manifest[field] = json!(v);
        }
    }
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())? + "\n";
    let sig = minisign::sign(None, &sk, std::io::Cursor::new(text.as_bytes()), Some(&format!("prism {version}")), Some("Prism update feed"))
        .map_err(|e| e.to_string())?
        .into_string();
    let stable = site.join("stable");
    std::fs::create_dir_all(&stable).map_err(|e| e.to_string())?;
    std::fs::write(stable.join("latest.json"), &text).map_err(|e| e.to_string())?;
    std::fs::write(stable.join("latest.json.minisig"), sig).map_err(|e| e.to_string())?;
    write_vercel_config(&site)?;
    prune(&site.join("releases"), keep)?;
    eprintln!("wrote {} (signed); deploy the site to publish", stable.join("latest.json").display());
    Ok(())
}

/// The feed must never be cached; the versioned packages never change.
fn write_vercel_config(site: &Path) -> Result<(), String> {
    let path = site.join("vercel.json");
    if path.exists() {
        return Ok(());
    }
    let cfg = json!({
        "headers": [
            {"source": "/stable/(.*)", "headers": [{"key": "Cache-Control", "value": "no-cache, max-age=0"}]},
            {"source": "/releases/(.*)", "headers": [{"key": "Cache-Control", "value": "public, max-age=31536000, immutable"}]}
        ]
    });
    std::fs::write(&path, serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())? + "\n").map_err(|e| e.to_string())
}

/// Keeps the newest `keep` version folders under `releases/`.
fn prune(releases: &Path, keep: usize) -> Result<(), String> {
    let mut versions: Vec<(Vec<u64>, bool, PathBuf)> = std::fs::read_dir(releases)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            version_key(&name).map(|(nums, release)| (nums, release, e.path()))
        })
        .collect();
    versions.sort_by(|a, b| (&b.0, b.1).cmp(&(&a.0, a.1)));
    for (_, _, path) in versions.into_iter().skip(keep) {
        eprintln!("  pruning {}", path.display());
        std::fs::remove_dir_all(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// Sort key for `X.Y.Z[-pre]`: numbers, then "is a final release" (1.2.0 > 1.2.0-rc.1).
fn version_key(v: &str) -> Option<(Vec<u64>, bool)> {
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let nums: Option<Vec<u64>> = core.split('.').map(|n| n.parse().ok()).collect();
    nums.filter(|n| n.len() == 3).map(|n| (n, pre.is_none()))
}

fn today() -> String {
    std::env::var("PRISM_BUILD_DATE").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| {
        std::process::Command::new("date")
            .args(["-u", "+%Y-%m-%d"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .filter(|d| !d.is_empty())
            .unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_sort_newest_first() {
        assert!(version_key("1.10.0") > version_key("1.9.3"));
        assert!(version_key("1.2.0") > version_key("1.2.0-rc.1"));
        assert_eq!(version_key("stable"), None);
        assert_eq!(version_key("1.2"), None);
    }

    #[test]
    fn prune_keeps_three() {
        let dir = std::env::temp_dir().join(format!("xtask-prune-{}", std::process::id()));
        for v in ["1.0.0", "1.1.0", "1.2.0-rc.1", "1.2.0", "1.10.0"] {
            std::fs::create_dir_all(dir.join(v)).unwrap();
        }
        prune(&dir, 3).unwrap();
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["1.10.0", "1.2.0", "1.2.0-rc.1"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
