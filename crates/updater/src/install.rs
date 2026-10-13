//! Replacing the installed app with a verified download.
//!
//! The running app can't overwrite itself, so installation happens in two steps: [`prepare`]
//! (while Prism runs: unpack and verify) and [`install`] (start a small helper that waits for
//! Prism to exit, swaps the app and relaunches it). The app quits right after [`install`].
//!
//! - **macOS**: the package is a zip of the signed, notarized `Prism.app`. It is unpacked next to
//!   the installed bundle (same volume, so the swap is two renames), checked with
//!   `codesign --verify --deep --strict` and must carry the same Team ID as the running app.
//! - **Windows**: the package is the MSI, which is a major upgrade of the installed product;
//!   `msiexec` verifies its Authenticode signature and replaces the old version.
//! - Portable Windows builds and other platforms only get a download link.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::{Error, Result};

/// How this copy of Prism was installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallTarget {
    /// A macOS app bundle at this path (`…/Prism.app`).
    MacBundle(PathBuf),
    /// An MSI installation; the path is the running `prism.exe`.
    WindowsMsi(PathBuf),
    /// Anything we can't replace in place, with the reason shown to the user.
    Manual(String),
}

impl InstallTarget {
    pub fn can_install(&self) -> bool {
        !matches!(self, InstallTarget::Manual(_))
    }
}

/// Works out how the running app was installed.
pub fn detect_install_target() -> InstallTarget {
    let exe = match std::env::current_exe().and_then(|p| p.canonicalize()) {
        Ok(p) => p,
        Err(e) => return InstallTarget::Manual(format!("can't locate the running app: {e}")),
    };
    target_for(&exe, std::env::consts::OS)
}

fn target_for(exe: &Path, os: &str) -> InstallTarget {
    match os {
        "macos" => {
            let Some(bundle) = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")) else {
                return InstallTarget::Manual("Prism isn't running from an app bundle".into());
            };
            if bundle.to_string_lossy().contains("/AppTranslocation/") {
                return InstallTarget::Manual("move Prism to the Applications folder first".into());
            }
            if bundle.starts_with("/Volumes") && !is_writable_dir(bundle.parent()) {
                return InstallTarget::Manual("Prism is running from the disk image; copy it to Applications first".into());
            }
            InstallTarget::MacBundle(bundle.to_path_buf())
        }
        "windows" => {
            if exe.parent().is_some_and(|d| d.join("portable.txt").exists()) {
                return InstallTarget::Manual("portable copies are updated by downloading the new zip".into());
            }
            InstallTarget::WindowsMsi(exe.to_path_buf())
        }
        other => InstallTarget::Manual(format!("in-app updates aren't available on {other}")),
    }
}

fn is_writable_dir(dir: Option<&Path>) -> bool {
    let Some(dir) = dir else { return false };
    let probe = dir.join(format!(".prism-write-test-{}", std::process::id()));
    match std::fs::create_dir(&probe) {
        Ok(()) => {
            let _ = std::fs::remove_dir(&probe);
            true
        }
        Err(_) => false,
    }
}

/// A verified update, ready to be swapped in once Prism exits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prepared {
    /// macOS: `new_app` replaces `app`; `staging` is removed afterwards.
    MacBundle { app: PathBuf, new_app: PathBuf, staging: PathBuf },
    /// Windows: run `msi`, then relaunch `exe`.
    WindowsMsi { msi: PathBuf, exe: PathBuf },
}

/// Unpacks and verifies `package` for `target`. Nothing installed is touched yet.
pub fn prepare(target: &InstallTarget, package: &Path, version: &str) -> Result<Prepared> {
    match target {
        InstallTarget::MacBundle(app) => prepare_mac(app, package, version),
        InstallTarget::WindowsMsi(exe) => {
            if package.extension().is_none_or(|e| !e.eq_ignore_ascii_case("msi")) {
                return Err(Error::Verify("expected an .msi package".into()));
            }
            Ok(Prepared::WindowsMsi { msi: package.to_path_buf(), exe: exe.clone() })
        }
        InstallTarget::Manual(why) => Err(Error::Unsupported(why.clone())),
    }
}

fn prepare_mac(app: &Path, package: &Path, version: &str) -> Result<Prepared> {
    let parent = app.parent().ok_or_else(|| Error::Unsupported("the app has no parent folder".into()))?;
    if !is_writable_dir(Some(parent)) {
        return Err(Error::Unsupported(format!("{} isn't writable; install the update from the disk image", parent.display())));
    }
    let safe_version: String = version.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-')).collect();
    let staging = parent.join(format!(".prism-update-{safe_version}"));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    let result = (|| {
        // `ditto` keeps the bundle's extended attributes and signature intact (unlike most unzip tools).
        run("ditto", Command::new("/usr/bin/ditto").arg("-x").arg("-k").arg(package).arg(&staging))?;
        let new_app = find_app(&staging)?;
        run("codesign --verify", Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(&new_app))?;
        let (old_team, new_team) = (team_id(app)?, team_id(&new_app)?);
        if old_team.is_some() && old_team != new_team {
            return Err(Error::Verify(format!("signed by {} instead of {}", new_team.as_deref().unwrap_or("nobody"), old_team.as_deref().unwrap_or("?"))));
        }
        Ok(new_app)
    })();
    match result {
        Ok(new_app) => Ok(Prepared::MacBundle { app: app.to_path_buf(), new_app, staging }),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

fn find_app(dir: &Path) -> Result<PathBuf> {
    let mut apps = std::fs::read_dir(dir)?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|e| e == "app") && p.is_dir());
    match (apps.next(), apps.next()) {
        (Some(app), None) => Ok(app),
        _ => Err(Error::Verify("the package must contain exactly one .app".into())),
    }
}

/// The signing Team ID, or `None` for ad-hoc / unsigned builds.
fn team_id(app: &Path) -> Result<Option<String>> {
    let out = Command::new("/usr/bin/codesign").args(["-d", "--verbose=2"]).arg(app).output()?;
    // codesign prints the details on stderr.
    let text = String::from_utf8_lossy(&out.stderr);
    Ok(parse_team_id(&text))
}

fn parse_team_id(codesign_output: &str) -> Option<String> {
    codesign_output.lines().find_map(|l| l.strip_prefix("TeamIdentifier=")).map(str::trim).filter(|t| !t.is_empty() && *t != "not set").map(str::to_owned)
}

fn run(what: &str, cmd: &mut Command) -> Result<()> {
    let out = cmd.stdin(Stdio::null()).output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::Verify(format!("{what} failed: {}", String::from_utf8_lossy(&out.stderr).trim())))
    }
}

/// Swaps the app in when Prism exits (sh helper on macOS); the bundle is moved aside, the new
/// one moved in, and the old one restored if that fails. Paths are passed as arguments, never
/// spliced into the script.
const MAC_HELPER: &str = r#"
pid="$1"; app="$2"; new="$3"; staging="$4"; n=0
while kill -0 "$pid" 2>/dev/null; do
  n=$((n+1)); [ "$n" -gt 3000 ] && exit 1
  sleep 0.2
done
old="$app.prism-old"
rm -rf "$old"
if mv "$app" "$old"; then
  if mv "$new" "$app"; then rm -rf "$old"; else mv "$old" "$app"; fi
fi
rm -rf "$staging"
open "$app"
"#;

/// Runs the MSI once Prism exits, then relaunches it.
const WINDOWS_HELPER: &str = r#"
param([int]$ProcessId, [string]$Msi, [string]$Exe, [string]$Log)
try { Wait-Process -Id $ProcessId -Timeout 600 -ErrorAction SilentlyContinue } catch {}
$p = Start-Process -FilePath 'msiexec.exe' -ArgumentList @('/i', "`"$Msi`"", '/passive', '/norestart', '/l*v', "`"$Log`"") -Wait -PassThru
if (Test-Path -LiteralPath $Exe) { Start-Process -FilePath $Exe }
exit $p.ExitCode
"#;

/// Starts the helper that finishes the update after this process exits. The caller must quit
/// the app promptly afterwards (after the user has saved their work).
pub fn install(prepared: &Prepared) -> Result<()> {
    let pid = std::process::id().to_string();
    match prepared {
        Prepared::MacBundle { app, new_app, staging } => {
            Command::new("/bin/sh")
                .arg("-c")
                .arg(MAC_HELPER)
                .arg("prism-update")
                .arg(&pid)
                .arg(app)
                .arg(new_app)
                .arg(staging)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            Ok(())
        }
        Prepared::WindowsMsi { msi, exe } => {
            let dir = msi.parent().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir);
            let script = dir.join("prism-update.ps1");
            std::fs::write(&script, WINDOWS_HELPER)?;
            let log = dir.join("prism-update-msi.log");
            let mut cmd = Command::new("powershell.exe");
            cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-File"])
                .arg(&script)
                .arg("-ProcessId")
                .arg(&pid)
                .arg("-Msi")
                .arg(msi)
                .arg("-Exe")
                .arg(exe)
                .arg("-Log")
                .arg(&log)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                const DETACHED_PROCESS: u32 = 0x0000_0008;
                cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
            }
            cmd.spawn()?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_bundle_around_the_binary() {
        let t = target_for(Path::new("/Applications/Prism.app/Contents/MacOS/prism"), "macos");
        assert_eq!(t, InstallTarget::MacBundle(PathBuf::from("/Applications/Prism.app")));
        assert!(!target_for(Path::new("/Users/me/prism/target/release/prism"), "macos").can_install());
        assert!(!target_for(Path::new("/private/var/folders/x/AppTranslocation/ABC/d/Prism.app/Contents/MacOS/prism"), "macos").can_install());
        assert!(!target_for(Path::new("/usr/local/bin/prism"), "freebsd").can_install());
    }

    #[test]
    fn portable_windows_copies_are_manual() {
        let dir = std::env::temp_dir().join(format!("prism-updater-portable-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("portable.txt"), "").unwrap();
        assert!(!target_for(&dir.join("prism.exe"), "windows").can_install());
        std::fs::remove_file(dir.join("portable.txt")).unwrap();
        assert!(target_for(&dir.join("prism.exe"), "windows").can_install());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn team_ids() {
        assert_eq!(parse_team_id("Identifier=com.nkstudio.prism\nTeamIdentifier=AB12CD34EF\n"), Some("AB12CD34EF".into()));
        assert_eq!(parse_team_id("TeamIdentifier=not set\n"), None);
        assert_eq!(parse_team_id(""), None);
    }

    #[test]
    fn manual_targets_refuse_to_prepare() {
        let t = InstallTarget::Manual("no".into());
        assert!(matches!(prepare(&t, Path::new("x.zip"), "1.0.0"), Err(Error::Unsupported(_))));
        let w = InstallTarget::WindowsMsi(PathBuf::from("C:/Prism/prism.exe"));
        assert!(prepare(&w, Path::new("x.zip"), "1.0.0").is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_prepare_rejects_a_zip_without_an_app() {
        let root = std::env::temp_dir().join(format!("prism-updater-mac-{}", std::process::id()));
        let app = root.join("Prism.app");
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        let payload = root.join("payload");
        std::fs::create_dir_all(&payload).unwrap();
        std::fs::write(payload.join("readme.txt"), "hi").unwrap();
        let zip = root.join("pkg.zip");
        let ok = Command::new("/usr/bin/ditto").args(["-c", "-k"]).arg(&payload).arg(&zip).status().unwrap().success();
        assert!(ok);
        assert!(matches!(prepare_mac(&app, &zip, "9.9.9"), Err(Error::Verify(_))));
        assert!(!root.join(".prism-update-9.9.9").exists(), "staging is cleaned up");
        let _ = std::fs::remove_dir_all(&root);
    }
}
