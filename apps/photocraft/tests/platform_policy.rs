//! Keep unsupported desktop targets out of release artifacts and application CI.
use std::path::Path;

#[test]
fn linux_is_not_an_application_release_target() {
    let release = include_str!("../../../.github/workflows/release.yml");
    let jobs: Vec<_> = release.lines().filter(|line| line.starts_with("  ") && !line.starts_with("   ") && line.ends_with(':')).collect();
    assert!(!jobs.contains(&"  linux:") && !jobs.contains(&"  flatpak:"));
    assert!(jobs.contains(&"  macos:") && jobs.contains(&"  windows:") && jobs.contains(&"  freebsd:"));
    assert!(!release.contains("packaging/linux/") && !release.contains("release-linux-"));
    assert!(!Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/linux").exists());
}

#[test]
fn desktop_ci_tests_macos_and_windows() {
    let ci = include_str!("../../../.github/workflows/ci.yml");
    assert!(ci.contains("os: [macos-latest, windows-latest]"));
    assert!(!ci.contains("ubuntu-") && !ci.contains("runner.os == 'Linux'"));
}

#[test]
fn freebsd_metadata_remains_available_without_linux_packaging() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for file in ["ai.storyteller.photocraft.desktop", "ai.storyteller.photocraft.mime.xml", "ai.storyteller.photocraft.metainfo.xml.in"] {
        assert!(root.join("packaging/freebsd").join(file).is_file());
    }
    let package = include_str!("../../../packaging/freebsd/package.sh");
    assert!(package.contains("$ROOT/packaging/freebsd"));
    assert!(!package.contains("packaging/linux"));
}
