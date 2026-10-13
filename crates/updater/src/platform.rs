//! Which feed entries apply to the running build.

/// Feed keys for this OS and CPU, most specific first. macOS also accepts the universal package.
pub fn platform_keys() -> Vec<&'static str> {
    keys_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// [`platform_keys`] for an explicit OS / architecture (`std::env::consts` spelling).
pub fn keys_for(os: &str, arch: &str) -> Vec<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => vec!["macos-aarch64", "macos-universal"],
        ("macos", "x86_64") => vec!["macos-x86_64", "macos-universal"],
        ("windows", "x86_64") => vec!["windows-x64"],
        ("windows", "x86") => vec!["windows-x86"],
        ("windows", "aarch64") => vec!["windows-arm64"],
        ("freebsd", "x86_64") => vec!["freebsd-x86_64"],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_falls_back_to_universal() {
        assert_eq!(keys_for("macos", "aarch64"), ["macos-aarch64", "macos-universal"]);
        assert_eq!(keys_for("windows", "x86_64"), ["windows-x64"]);
        assert!(keys_for("linux", "x86_64").is_empty());
    }
}
