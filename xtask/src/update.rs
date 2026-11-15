//! `cargo xtask update`: signing keys and the in-app update feed (see `crates/updater`).
//!
//! ```text
//! cargo xtask update keygen --secret ~/prism-update.key [--public PATH]
//!     new minisign key pair; the public half goes to packaging/update/public.key (compiled into
//!     the app), the secret half to the given path outside the repository (store it as the
//!     PRISM_UPDATE_SECRET_KEY CI secret).
//! cargo xtask update publish --site <update site checkout>
//!     [--packages-url URL] [--dist dist/release] [--version X.Y.Z] [--min-supported X.Y.Z]
//!     [--notes-url URL] [--download-url URL] [--secret PATH]
//!     writes and signs <site>/stable/latest.json for this version's update packages, which are
//!     served from --packages-url (default: this version's GitHub Release), and writes the site's
//!     Cloudflare config if missing.
//! ```
//!
//! The site serves only the feed: Cloudflare's static assets stop at 25 MiB per file, and the
//! packages are larger, so they live on the GitHub Release (`{version}` in --packages-url is
//! replaced by the version). The packages must be uploaded there before the feed goes live.
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
        _ => Err("usage: cargo xtask update keygen --secret PATH | update publish --site DIR [--packages-url URL] [...]".into()),
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
    let site = absolute(Path::new(flag(args, "--site").ok_or("--site DIR is required (the update site checkout)")?))?;
    let dist = match flag(args, "--dist") {
        Some(d) => absolute(Path::new(d))?,
        None => std::env::var("DIST").map(PathBuf::from).unwrap_or_else(|_| root.join("dist/release")),
    };
    let version = match flag(args, "--version") {
        Some(v) => v.to_owned(),
        None => crate::version::read(&std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?)?,
    };
    crate::version::validate(&version)?;
    let packages_url = packages_url(flag(args, "--packages-url").unwrap_or(DEFAULT_PACKAGES_URL), &version)?;
    let sk = secret_key(args)?;

    let mut platforms = Map::new();
    for (suffix, key) in PACKAGES {
        let name = format!("prism-{version}-{suffix}");
        let src = dist.join(&name);
        if !src.is_file() {
            continue;
        }
        let bytes = std::fs::read(&src).map_err(|e| format!("{}: {e}", src.display()))?;
        platforms.insert((*key).to_owned(), json!({"url": format!("{packages_url}/{name}"), "sha256": crate::sha256::hex(&bytes), "size": bytes.len()}));
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
    write_site_config(&site)?;
    // Packages used to be served by the site itself; they now live on the GitHub Release.
    let old_packages = site.join("releases");
    if old_packages.is_dir() {
        std::fs::remove_dir_all(&old_packages).map_err(|e| format!("{}: {e}", old_packages.display()))?;
        eprintln!("  removed {} (packages are served from {packages_url})", old_packages.display());
    }
    eprintln!("wrote {} (signed); upload the packages to {packages_url}, then deploy the site", stable.join("latest.json").display());
    Ok(())
}

/// Where the update packages are downloaded from: this version's GitHub Release.
const DEFAULT_PACKAGES_URL: &str = "https://github.com/NK-Studio/prism/releases/download/v{version}";

fn packages_url(template: &str, version: &str) -> Result<String, String> {
    let url = template.replace("{version}", version).trim_end_matches('/').to_owned();
    if !prism_updater_trusts(&url) {
        return Err(format!("--packages-url must be https: {url}"));
    }
    Ok(url)
}

/// Mirrors `prism_updater::manifest::is_trusted_url`: Prism refuses anything else.
fn prism_updater_trusts(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("https://") || ["http://localhost:", "http://localhost/", "http://127.0.0.1:", "http://127.0.0.1/"].iter().any(|p| lower.starts_with(p))
}

/// Site files written once and then left alone (edit them in the site repository).
/// Cloudflare Workers Builds deploys the site repository with `wrangler.jsonc`.
const SITE_FILES: &[(&str, &str)] = &[
    (
        "wrangler.jsonc",
        "// Cloudflare Workers Builds deploys this repository on push: a static site with the Prism\n\
         // update feed (written by `cargo xtask update publish` in the Prism repository).\n\
         {\n  \"name\": \"prism-updates\",\n  \"compatibility_date\": \"2026-11-01\",\n  \"assets\": {\n    \"directory\": \".\"\n  }\n}\n",
    ),
    (
        // Not uploaded as assets (Cloudflare reads `_headers` itself).
        ".assetsignore",
        ".git\n.github\n.assetsignore\nwrangler.jsonc\nvercel.json\nREADME.md\n",
    ),
    (
        // The feed must never be cached, so a new version reaches everyone at once.
        "_headers",
        "/stable/*\n  Cache-Control: no-cache, max-age=0\n",
    ),
];

fn write_site_config(site: &Path) -> Result<(), String> {
    for (name, text) in SITE_FILES {
        let path = site.join(name);
        if !path.exists() {
            std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
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
    fn packages_come_from_the_github_release() {
        assert_eq!(packages_url(DEFAULT_PACKAGES_URL, "1.3.0").unwrap(), "https://github.com/NK-Studio/prism/releases/download/v1.3.0");
        assert_eq!(packages_url("https://example.com/p/{version}/", "2.0.0").unwrap(), "https://example.com/p/2.0.0");
        assert!(packages_url("http://example.com/{version}", "1.0.0").is_err());
    }

    #[test]
    fn site_config_is_written_once() {
        let dir = std::env::temp_dir().join(format!("xtask-site-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("_headers"), "custom\n").unwrap();
        write_site_config(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("_headers")).unwrap(), "custom\n");
        let wrangler = std::fs::read_to_string(dir.join("wrangler.jsonc")).unwrap();
        assert!(wrangler.contains("\"name\": \"prism-updates\""));
        let ignore = std::fs::read_to_string(dir.join(".assetsignore")).unwrap();
        assert!(ignore.lines().any(|l| l == ".git"));
        assert!(!dir.join("vercel.json").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
