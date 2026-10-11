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
    for file in ["com.nkstudio.prism.desktop", "com.nkstudio.prism.mime.xml", "com.nkstudio.prism.metainfo.xml.in"] {
        assert!(root.join("packaging/freebsd").join(file).is_file());
    }
    let package = include_str!("../../../packaging/freebsd/package.sh");
    assert!(package.contains("$ROOT/packaging/freebsd"));
    assert!(!package.contains("packaging/linux"));
}

#[test]
fn published_app_branding_is_prism_by_nk_studio() {
    for resource in [include_str!("../build.rs"), include_str!("../../prism-cli/build.rs")] {
        assert!(resource.contains(".set(\"ProductName\", \"Prism\")"));
        assert!(resource.contains(".set(\"CompanyName\", \"NK Studio\")"));
        assert!(resource.contains("Copyright (c) 2026 NK Studio."));
    }
    let dmg = include_str!("../../../packaging/macos/dmg/background.svg");
    assert!(dmg.contains("<title>Prism — NK Studio</title>"));
    assert!(!dmg.contains("prism-small.svg"));
    let installer = include_str!("../../../packaging/windows/prism.wxs");
    assert!(installer.contains("Name=\"Prism\""));
    assert!(installer.contains("Manufacturer=\"NK Studio\""));
    let metadata = include_str!("../../../packaging/freebsd/com.nkstudio.prism.metainfo.xml.in");
    assert!(metadata.contains("<name>Prism</name>") && metadata.contains("<name>NK Studio</name>"));
    for published in [installer, metadata] {
        assert!(!published.contains("discord.gg") && !published.contains("getartcraft.com"));
    }
}
