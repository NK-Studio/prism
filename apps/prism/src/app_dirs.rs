//! Where the desktop app keeps its per-user data: preferences, brush presets, crash-recovery
//! autosaves and the GPU startup marker all live under the one directory [`config_dir`] returns.
//!
//! In order:
//! 1. `PRISM_CONFIG_DIR`, when set (tests, agents, custom setups).
//! 2. **Portable mode** (#228): a marker file ([`PORTABLE_MARKERS`]) beside the executable puts
//!    everything in `<exe dir>/PrismData`, so a portable copy on a USB stick leaves nothing in
//!    `%APPDATA%`. The Windows portable zip ships with `portable.txt`; the MSI doesn't. If that
//!    folder can't be written (read-only medium, Program Files), the app warns and falls back.
//! 3. The platform convention: macOS `~/Library/Application Support/Prism`, Windows
//!    `%APPDATA%\Prism`, FreeBSD `$XDG_CONFIG_HOME/prism` or `~/.config/prism`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Files beside the executable that switch on portable mode (either one; contents are ignored).
pub const PORTABLE_MARKERS: [&str; 3] = ["portable.txt", "Prism.portable", "PhotoCraft.portable"];
/// The data folder created beside the executable in portable mode.
pub const PORTABLE_DATA_DIR: &str = "PrismData";

/// How [`DataDir::dir`] was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `PRISM_CONFIG_DIR`.
    Override,
    /// `<exe dir>/PrismData`.
    Portable,
    /// The platform's per-user config directory.
    Platform,
}

/// The resolved data directory.
#[derive(Clone, Debug)]
pub struct DataDir {
    /// `None` only when no platform directory is known either (no `HOME`/`APPDATA`).
    pub dir: Option<PathBuf>,
    pub mode: Mode,
    /// Set when a portable marker was found but its data folder isn't writable.
    pub warning: Option<String>,
}

/// The data directory for this process, resolved once (the portable check probes the disk).
pub fn current() -> &'static DataDir {
    static DIR: OnceLock<DataDir> = OnceLock::new();
    DIR.get_or_init(|| {
        let exe = std::env::current_exe().ok();
        let d = resolve(|k| std::env::var_os(k), exe.as_deref().and_then(Path::parent));
        if let Some(w) = &d.warning {
            log::warn!("{w}");
            eprintln!("prism: {w}");
        }
        log::info!("settings directory ({:?}): {:?}", d.mode, d.dir);
        d
    })
}

/// The per-user settings directory (see the module docs).
pub fn config_dir() -> Option<PathBuf> {
    current().dir.clone()
}

/// Resolve the data directory from the environment (`env`) and the executable's folder.
pub fn resolve(env: impl Fn(&str) -> Option<OsString>, exe_dir: Option<&Path>) -> DataDir {
    if let Some(d) = env("PRISM_CONFIG_DIR").or_else(|| env("PHOTOCRAFT_CONFIG_DIR")).filter(|d| !d.is_empty()) {
        return DataDir { dir: Some(PathBuf::from(d)), mode: Mode::Override, warning: None };
    }
    let platform = platform_dir(&env);
    let Some((exe_dir, marker)) = exe_dir.and_then(|d| portable_marker(d).map(|m| (d, m))) else {
        return migrated(platform, legacy_platform_dir(&env), Mode::Platform);
    };
    let data = exe_dir.join(PORTABLE_DATA_DIR);
    let migration = migrated(Some(data.clone()), Some(exe_dir.join("PhotoCraftData")), Mode::Portable);
    let chosen = migration.dir.as_deref().unwrap_or(&data);
    match ensure_writable(chosen) {
        Ok(()) => migration,
        Err(e) => {
            let mut resolved = migrated(platform, legacy_platform_dir(&env), Mode::Platform);
            let fallback = resolved.dir.as_ref().map_or_else(|| "nowhere (no user folder found)".to_string(), |p| p.display().to_string());
            let warning =
                format!("portable mode ({} found) but {} isn't writable ({e}); settings are kept in {fallback} instead", marker.display(), data.display());
            resolved.warning = Some(resolved.warning.map_or_else(|| warning.clone(), |migration_warning| format!("{warning}; {migration_warning}")));
            resolved
        }
    }
}

/// The platform convention, from `env` (so tests can fake `HOME`/`APPDATA`/`XDG_CONFIG_HOME`).
pub fn platform_dir(env: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let home = env("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|h| h.join("Library/Application Support/Prism"));
    }
    if cfg!(windows) {
        return env("APPDATA").filter(|a| !a.is_empty()).map(|a| PathBuf::from(a).join("Prism"));
    }
    env("XDG_CONFIG_HOME").filter(|x| !x.is_empty()).map(PathBuf::from).or_else(|| home.map(|h| h.join(".config"))).map(|c| c.join("prism"))
}

/// Historical locations are read only for migration/fallback, never used for a fresh installation.
fn legacy_platform_dir(env: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    platform_dir(env)
        .and_then(|new| new.parent().map(|parent| parent.join(if cfg!(target_os = "macos") || cfg!(windows) { "Photocraft" } else { "photocraft" })))
}

fn migrated(destination: Option<PathBuf>, source: Option<PathBuf>, mode: Mode) -> DataDir {
    if let (Some(new), Some(old)) = (&destination, &source)
        && let Err(error) = crate::settings_migration::migrate(old, new)
    {
        let warning = format!("could not migrate settings to {} ({error}); original settings remain in use at {}", new.display(), old.display());
        return DataDir { dir: source, mode, warning: Some(warning) };
    }
    DataDir { dir: destination, mode, warning: None }
}

/// The first portable marker file in `exe_dir`, if any.
pub fn portable_marker(exe_dir: &Path) -> Option<PathBuf> {
    PORTABLE_MARKERS.iter().map(|m| exe_dir.join(m)).find(|p| p.is_file())
}

/// Create `dir` and prove a file can be written in it (a folder can exist yet be read-only).
fn ensure_writable(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let probe = dir.join(format!(".write-test-{}", std::process::id()));
    std::fs::write(&probe, b"")?;
    // A leftover probe is harmless; don't fail portable mode over it.
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("prism-app-dirs-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn env_of(pairs: &[(&str, &Path)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let m: HashMap<String, OsString> = pairs.iter().map(|(k, v)| (k.to_string(), v.as_os_str().to_owned())).collect();
        move |k| m.get(k).cloned()
    }

    /// A fake user folder that every platform's convention resolves under.
    fn fake_home(root: &Path) -> impl Fn(&str) -> Option<OsString> + use<> {
        let home = root.join("home");
        let appdata = root.join("appdata");
        env_of(&[("HOME", &home), ("APPDATA", &appdata)])
    }

    #[test]
    fn no_marker_uses_the_platform_dir() {
        let root = temp("platform");
        let exe = root.join("bin");
        std::fs::create_dir_all(&exe).unwrap();
        let env = fake_home(&root);
        let d = resolve(&env, Some(&exe));
        assert_eq!(d.mode, Mode::Platform);
        assert!(d.warning.is_none());
        let dir = d.dir.unwrap();
        assert_eq!(Some(dir.clone()), platform_dir(&env));
        assert!(!dir.starts_with(&exe));
        assert!(!exe.join(PORTABLE_DATA_DIR).exists(), "no portable folder without a marker");
        // The convention per platform.
        if cfg!(target_os = "macos") {
            assert_eq!(dir, root.join("home/Library/Application Support/Prism"));
        } else if cfg!(windows) {
            assert_eq!(dir, root.join("appdata").join("Prism"));
        } else {
            assert_eq!(dir, root.join("home/.config/prism"));
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn either_marker_keeps_everything_beside_the_exe() {
        for marker in PORTABLE_MARKERS {
            let root = temp("portable");
            let exe = root.join("Prism");
            std::fs::create_dir_all(&exe).unwrap();
            std::fs::write(exe.join(marker), b"").unwrap();
            let d = resolve(fake_home(&root), Some(&exe));
            assert_eq!(d.mode, Mode::Portable, "{marker}");
            assert!(d.warning.is_none());
            let dir = d.dir.unwrap();
            assert_eq!(dir, exe.join(PORTABLE_DATA_DIR));
            assert!(dir.is_dir(), "the data folder is created");
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "the write probe is removed");
            // Every derived path sits under the exe dir.
            for sub in ["preferences.json", "Presets", "Recovery", crate::gpu_startup::MARKER_FILE] {
                assert!(dir.join(sub).starts_with(&exe));
            }
            assert!(!root.join("home").exists() && !root.join("appdata").exists(), "nothing written to the user folder");
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn a_marker_folder_is_not_a_marker() {
        let root = temp("dir-marker");
        let exe = root.join("bin");
        std::fs::create_dir_all(exe.join("portable.txt")).unwrap();
        assert_eq!(resolve(fake_home(&root), Some(&exe)).mode, Mode::Platform);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unwritable_portable_folder_falls_back_with_a_warning() {
        let root = temp("unwritable");
        let exe = root.join("bin");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::write(exe.join("portable.txt"), b"").unwrap();
        // A file where the data folder should go: it can't be created, on every platform.
        std::fs::write(exe.join(PORTABLE_DATA_DIR), b"in the way").unwrap();
        let env = fake_home(&root);
        let d = resolve(&env, Some(&exe));
        assert_eq!(d.mode, Mode::Platform);
        assert_eq!(d.dir, platform_dir(&env));
        let w = d.warning.unwrap();
        assert!(w.contains("portable") && w.contains("isn't writable"), "{w}");
        // No user folder at all: still no panic, just no directory.
        let d = resolve(|_| None, Some(&exe));
        assert!(d.dir.is_none() && d.warning.is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn override_wins_over_portable() {
        let root = temp("override");
        let exe = root.join("bin");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::write(exe.join("portable.txt"), b"").unwrap();
        let custom = root.join("custom");
        let d = resolve(env_of(&[("PRISM_CONFIG_DIR", &custom)]), Some(&exe));
        assert_eq!((d.mode, d.dir), (Mode::Override, Some(custom)));
        assert!(!exe.join(PORTABLE_DATA_DIR).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_exe_dir_or_env_never_panics() {
        let d = resolve(|_| None, None);
        assert_eq!((d.mode, d.dir), (Mode::Platform, None));
        let empty = PathBuf::new();
        let d = resolve(env_of(&[("PRISM_CONFIG_DIR", &empty), ("HOME", &empty), ("APPDATA", &empty)]), None);
        assert_eq!(d.dir, None, "empty variables are ignored");
    }
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    fn root() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("prism-migration-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        path
    }
    #[test]
    fn settings_recovery_and_cursor_files_migrate_without_modifying_the_original() {
        let root = root();
        let env = |name: &str| match name {
            "HOME" | "APPDATA" | "XDG_CONFIG_HOME" => Some(root.as_os_str().to_owned()),
            _ => None,
        };
        let old = legacy_platform_dir(&env).unwrap();
        for (path, bytes) in [
            ("preferences.json", b"preferences".as_slice()),
            ("Recovery/a.pcraft/manifest.json", b"recovery"),
            ("cursor-pack/custom.png", b"cursor"),
            ("logs/photocraft.log", b"log"),
        ] {
            let file = old.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
        let choice = resolve(env, None);
        assert!(choice.warning.is_none());
        let new = choice.dir.unwrap();
        assert_ne!(old, new);
        assert_eq!(std::fs::read(new.join("preferences.json")).unwrap(), b"preferences");
        assert_eq!(std::fs::read(new.join("Recovery/a.pcraft/manifest.json")).unwrap(), b"recovery");
        assert_eq!(std::fs::read(new.join("cursor-pack/custom.png")).unwrap(), b"cursor");
        assert_eq!(std::fs::read(new.join("logs/prism.log")).unwrap(), b"log");
        assert!(old.join("logs/photocraft.log").is_file());
        // New preferences win on subsequent launches, even when the legacy directory changes.
        std::fs::write(new.join("preferences.json"), b"new settings").unwrap();
        resolve(env, None);
        assert_eq!(std::fs::read(new.join("preferences.json")).unwrap(), b"new settings");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn legacy_portable_marker_migrates_data_beside_the_executable() {
        let root = root();
        std::fs::write(root.join("PhotoCraft.portable"), b"").unwrap();
        let old = root.join("PhotoCraftData");
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("ui.ron"), b"window state").unwrap();
        let choice = resolve(|_| None, Some(&root));
        assert_eq!(choice.mode, Mode::Portable);
        assert_eq!(choice.dir, Some(root.join("PrismData")));
        assert_eq!(std::fs::read(root.join("PrismData/ui.ron")).unwrap(), b"window state");
        assert!(old.join("ui.ron").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn new_override_wins_and_legacy_override_is_still_readable() {
        let new = PathBuf::from("new");
        let old = PathBuf::from("old");
        let env = |name: &str| match name {
            "PRISM_CONFIG_DIR" => Some(new.as_os_str().to_owned()),
            "PHOTOCRAFT_CONFIG_DIR" => Some(old.as_os_str().to_owned()),
            _ => None,
        };
        assert_eq!(resolve(env, None).dir, Some(new));
        assert_eq!(resolve(|name| (name == "PHOTOCRAFT_CONFIG_DIR").then(|| old.as_os_str().to_owned()), None).dir, Some(old));
    }
    #[cfg(unix)]
    #[test]
    fn migration_preserves_private_directory_and_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let root = root();
        let old = root.join("old");
        let new = root.join("new");
        std::fs::create_dir_all(old.join("Recovery")).unwrap();
        std::fs::write(old.join("Recovery/private.json"), b"private document").unwrap();
        for (path, mode) in [(&old, 0o700), (&old.join("Recovery"), 0o710), (&old.join("Recovery/private.json"), 0o600)] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        crate::settings_migration::migrate(&old, &new).unwrap();
        for relative in ["", "Recovery", "Recovery/private.json"] {
            assert_eq!(
                std::fs::metadata(new.join(relative)).unwrap().permissions().mode(),
                std::fs::metadata(old.join(relative)).unwrap().permissions().mode()
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_links_leave_the_old_settings_and_no_partial_destination() {
        let root = root();
        let old = root.join("old");
        let new = root.join("new");
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("preferences.json"), b"original").unwrap();
        std::os::unix::fs::symlink("/", old.join("outside")).unwrap();
        let choice = migrated(Some(new.clone()), Some(old.clone()), Mode::Platform);
        assert_eq!(choice.dir, Some(old.clone()));
        assert!(choice.warning.is_some());
        assert!(!new.exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1, "staging directory is cleaned up");
        assert_eq!(std::fs::read(old.join("preferences.json")).unwrap(), b"original");
        std::fs::remove_dir_all(root).unwrap();
    }
}
