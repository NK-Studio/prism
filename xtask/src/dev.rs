//! Local iteration only: no release packaging or CI configuration changes.
use std::ffi::OsStr;
use std::io::{BufRead, BufReader};
#[cfg(target_os = "macos")]
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Instant;

#[derive(Debug, Default, PartialEq)]
struct Options {
    check: bool,
    launch: bool,
    no_cache: bool,
    features: Option<String>,
    app_args: Vec<String>,
}

impl Options {
    fn parse(args: &[&str]) -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = args.iter().copied();
        while let Some(arg) = args.next() {
            match arg {
                "--check" => options.check = true,
                "--run" => options.launch = true,
                "--no-cache" => options.no_cache = true,
                "--features" => {
                    options.features = Some(args.next().filter(|s| !s.is_empty() && !s.starts_with('-')).ok_or("--features needs a feature list")?.into())
                }
                "--" => {
                    options.app_args = args.map(String::from).collect();
                    break;
                }
                other => return Err(format!("unknown dev option: {other}")),
            }
        }
        if options.check && options.launch {
            return Err("--check and --run cannot be combined".into());
        }
        if !options.launch && !options.app_args.is_empty() {
            return Err("application arguments require --run".into());
        }
        Ok(options)
    }

    fn build_command(&self) -> Command {
        let mut cmd = crate::cargo();
        cmd.args([if self.check { "check" } else { "build" }, "--profile", "fast", "-p", "photocraft", "--locked", "--message-format=json-render-diagnostics"]);
        if let Some(features) = &self.features {
            cmd.args(["--features", features]);
        }
        cmd
    }
}

fn cache_policy(disabled: bool, wrapper: Option<&OsStr>, available: bool) -> &'static str {
    if disabled {
        "disabled"
    } else if wrapper.is_some() {
        "explicit"
    } else if available {
        "sccache"
    } else {
        "compiler"
    }
}

fn compiler_cache(cmd: &mut Command, options: &Options) {
    let wrapper = std::env::var_os("RUSTC_WRAPPER").or_else(|| std::env::var_os("RUSTC_WORKSPACE_WRAPPER"));
    let available = !options.no_cache && wrapper.is_none() && Command::new("sccache").arg("--show-stats").output().is_ok_and(|o| o.status.success());
    let policy = cache_policy(options.no_cache, wrapper.as_deref(), available);
    match policy {
        "disabled" => {
            cmd.env("RUSTC_WRAPPER", "").env("RUSTC_WORKSPACE_WRAPPER", "");
        }
        "sccache" => {
            // Keep workspace incremental compilation; sccache caches eligible dependencies.
            cmd.env("RUSTC_WRAPPER", "sccache").env("SCCACHE_IGNORE_SERVER_IO_ERROR", "1");
        }
        _ => {}
    }
    eprintln!("compiler cache: {policy} (workspace incremental compilation retained)");
}

fn artifact(message: &str) -> Option<PathBuf> {
    let value: serde_json::Value = serde_json::from_str(message).ok()?;
    if value.get("reason")?.as_str()? != "compiler-artifact" || value.get("target")?.get("name")?.as_str()? != "photocraft" {
        return None;
    }
    value.get("executable")?.as_str().map(PathBuf::from)
}

pub fn run(args: &[&str]) -> Result<(), String> {
    if args.iter().take_while(|arg| **arg != "--").any(|arg| matches!(*arg, "-h" | "--help")) {
        println!(
            "cargo dev [--check | --run] [--no-cache] [--features LIST] [-- <app args>]\nDefault: fast native build; macOS also assembles Prism.app. --run launches it explicitly."
        );
        return Ok(());
    }
    let options = Options::parse(args)?;
    let start = Instant::now();
    let mut cmd = options.build_command();
    compiler_cache(&mut cmd, &options);
    let mut child = cmd.stdout(Stdio::piped()).spawn().map_err(|e| format!("cargo dev: {e}"))?;
    let output = child.stdout.take().ok_or("cargo dev: no compiler output")?;
    let mut executable = None;
    let mut read_error = None;
    for line in BufReader::new(output).lines() {
        match line {
            Ok(line) => {
                if let Some(path) = artifact(&line) {
                    executable = Some(path);
                }
            }
            Err(e) => {
                read_error = Some(e);
                break;
            }
        }
    }
    if read_error.is_some() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|e| format!("waiting for compiler: {e}"))?;
    if let Some(e) = read_error {
        return Err(format!("reading compiler output: {e}"));
    }
    if !status.success() {
        return Err(format!("cargo dev compilation failed: {status}"));
    }
    if options.check {
        eprintln!("check complete in {:.2}s", start.elapsed().as_secs_f64());
        return Ok(());
    }
    let executable = executable.ok_or("Cargo did not report the Prism executable")?;
    #[cfg(target_os = "macos")]
    let launch_path = macos_bundle(&executable)?;
    #[cfg(not(target_os = "macos"))]
    let launch_path = executable;
    println!("ready: {} ({:.2}s)", launch_path.display(), start.elapsed().as_secs_f64());
    if options.launch {
        #[cfg(target_os = "macos")]
        let launch_path = launch_path.join("Contents/MacOS/Prism");
        let status = Command::new(launch_path).args(&options.app_args).status().map_err(|e| format!("launching Prism: {e}"))?;
        if !status.success() {
            return Err(format!("Prism exited with {status}"));
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_bundle(executable: &Path) -> Result<PathBuf, String> {
    let root = crate::root();
    let manifest: toml::Value = toml::from_str(&std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let version = manifest
        .get("workspace")
        .and_then(|v| v.get("package"))
        .and_then(|v| v.get("version"))
        .and_then(toml::Value::as_str)
        .ok_or("workspace version missing")?;
    let short = version.split('-').next().unwrap_or(version);
    let sha = Command::new("git")
        .current_dir(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    let template = std::fs::read_to_string(root.join("packaging/macos/Info.plist.in")).map_err(|e| e.to_string())?;
    let plist = template.replace("@VERSION@", version).replace("@SHORT_VERSION@", short).replace("@BUILD_SHA@", &sha);
    let mut files = vec![
        (PathBuf::from("Contents/MacOS/Prism"), executable.to_path_buf()),
        (PathBuf::from("Contents/Resources/Prism.icns"), root.join("assets/app-icon/photocraft.icns")),
    ];
    if let Some(fonts) = std::env::var_os("CRAFT_FONTS_DIR").filter(|p| !p.is_empty()) {
        let families = PathBuf::from(fonts).join("fonts");
        for family in std::fs::read_dir(&families).map_err(|e| format!("font licenses in {}: {e}", families.display()))? {
            let family = family.map_err(|e| e.to_string())?;
            let license = family.path().join("OFL.txt");
            if license.is_file() {
                files.push((PathBuf::from("Contents/Resources/Licenses").join(format!("OFL-{}.txt", family.file_name().to_string_lossy())), license));
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let app = executable.parent().ok_or("executable has no directory")?.join("Prism.app");
    let changed = assemble_bundle(&app, &plist, &files, |stage| {
        let status = Command::new("codesign").args(["--force", "--sign", "-", "--timestamp=none"]).arg(stage).status().map_err(|e| format!("codesign: {e}"))?;
        if status.success() { Ok(()) } else { Err(format!("codesign failed: {status}")) }
    })?;
    eprintln!("macOS bundle: {}", if changed { "updated and ad-hoc signed" } else { "unchanged; assembly/signing skipped" });
    Ok(app)
}

/// Build beside the destination; a failed signer never replaces the working app.
#[cfg(target_os = "macos")]
fn assemble_bundle(app: &Path, plist: &str, files: &[(PathBuf, PathBuf)], sign: impl FnOnce(&Path) -> Result<(), String>) -> Result<bool, String> {
    use std::fs;
    let mut fingerprint = plist.to_string();
    for (relative, source) in files {
        fingerprint.push_str(&format!("\n{}:{}", relative.display(), crate::sha256::hex(&fs::read(source).map_err(|e| e.to_string())?)));
    }
    let stamp = crate::sha256::hex(fingerprint.as_bytes());
    let stamp_path = app.join("Contents/Resources/dev-build.sha256");
    if fs::read_to_string(&stamp_path).ok().as_deref() == Some(&stamp)
        && app.join("Contents/Info.plist").is_file()
        && files.iter().all(|(path, _)| app.join(path).is_file())
    {
        return Ok(false);
    }
    let parent = app.parent().ok_or("bundle has no parent directory")?;
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
    let stage = parent.join(format!(".prism-dev-{}-{nonce}", std::process::id()));
    let backup = stage.with_extension("previous");
    let result = (|| {
        fs::create_dir_all(stage.join("Contents/Resources")).map_err(|e| e.to_string())?;
        for (relative, source) in files {
            let dest = stage.join(relative);
            fs::create_dir_all(dest.parent().ok_or("asset has no parent")?).map_err(|e| e.to_string())?;
            fs::copy(source, dest).map_err(|e| e.to_string())?;
        }
        fs::write(stage.join("Contents/Info.plist"), plist).map_err(|e| e.to_string())?;
        fs::write(stage.join("Contents/Resources/dev-build.sha256"), &stamp).map_err(|e| e.to_string())?;
        sign(&stage)?;
        let old = app.exists();
        if old {
            fs::rename(app, &backup).map_err(|e| e.to_string())?;
        }
        if let Err(e) = fs::rename(&stage, app) {
            if old && let Err(restore) = fs::rename(&backup, app) {
                return Err(format!("bundle install failed: {e}; restore failed: {restore}; previous app: {}", backup.display()));
            }
            return Err(format!("bundle install failed: {e}"));
        }
        if old {
            fs::remove_dir_all(&backup).map_err(|e| format!("bundle updated; removing previous app {}: {e}", backup.display()))?;
        }
        Ok(true)
    })();
    if stage.exists() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn options_are_unambiguous_and_keep_application_arguments_literal() {
        let options = Options::parse(&["--run", "--features", "heif", "--", "a b.psd", "--safe-gpu"]).unwrap();
        assert!(options.launch);
        assert_eq!(options.features.as_deref(), Some("heif"));
        assert_eq!(options.app_args, ["a b.psd", "--safe-gpu"]);
        for args in [vec!["--check", "--run"], vec!["--features"], vec!["--features", "--run"], vec!["--", "file"], vec!["--wat"]] {
            assert!(Options::parse(&args).is_err());
        }
        assert_eq!(Options::parse(&[]).unwrap(), Options::default());
    }
    #[test]
    fn cache_is_optional_and_explicit_wrappers_are_preserved() {
        assert_eq!(cache_policy(false, None, false), "compiler");
        assert_eq!(cache_policy(false, None, true), "sccache");
        assert_eq!(cache_policy(false, Some(OsStr::new("custom")), true), "explicit");
        assert_eq!(cache_policy(true, Some(OsStr::new("custom")), true), "disabled");
    }
    #[test]
    fn cargo_artifact_handles_custom_target_directory_and_nonartifact_messages() {
        assert_eq!(
            artifact(r#"{"reason":"compiler-artifact","target":{"name":"photocraft"},"executable":"/tmp/custom build/fast/photocraft"}"#),
            Some(PathBuf::from("/tmp/custom build/fast/photocraft"))
        );
        for line in ["not json", "{}", r#"{"reason":"compiler-artifact","target":{"name":"other"},"executable":"other"}"#] {
            assert!(artifact(line).is_none());
        }
        let cmd = Options { check: true, features: Some("heif".into()), ..Default::default() }.build_command();
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.starts_with(&["check".into(), "--profile".into(), "fast".into()]));
        assert!(args.ends_with(&["--features".into(), "heif".into()]));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn bundle_skips_unchanged_rebuilds_changes_and_preserves_previous_app_on_sign_failure() {
        let root = std::env::temp_dir().join(format!("prism-dev-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("binary");
        std::fs::write(&source, b"v1").unwrap();
        let app = root.join("Prism.app");
        let files = vec![(PathBuf::from("Contents/MacOS/Prism"), source.clone())];
        assert!(assemble_bundle(&app, "plist", &files, |_| Ok(())).unwrap());
        assert!(!assemble_bundle(&app, "plist", &files, |_| panic!("unchanged must skip signer")).unwrap());
        std::fs::write(&source, b"v2").unwrap();
        assert!(assemble_bundle(&app, "plist", &files, |_| Err("sign failed".into())).is_err());
        assert_eq!(std::fs::read(app.join("Contents/MacOS/Prism")).unwrap(), b"v1");
        assert!(assemble_bundle(&app, "plist", &files, |_| Ok(())).unwrap());
        assert_eq!(std::fs::read(app.join("Contents/MacOS/Prism")).unwrap(), b"v2");
        std::fs::remove_dir_all(root).unwrap();
    }
}
