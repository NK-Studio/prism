//! Bounded, deterministic discovery of installed font files, including macOS font assets.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub(crate) fn files(roots: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for root in roots {
        visit(&root, 0, &mut seen, &mut out);
    }
    out
}

fn visit(path: &Path, depth: usize, seen: &mut HashSet<PathBuf>, out: &mut Vec<PathBuf>) {
    if depth > 8 || seen.len() >= 65536 {
        return;
    }
    let Ok(path) = path.canonicalize() else { return };
    if !seen.insert(path.clone()) {
        return;
    }
    if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else { return };
        let mut entries: Vec<_> = entries.flatten().take(16384).map(|e| e.path()).collect();
        entries.sort();
        for entry in entries {
            visit(&entry, depth + 1, seen, out);
        }
    } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc" | "otc")) {
        out.push(path);
    }
}

pub(crate) fn asset_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        for entry in entries.flatten().take(4096) {
            if entry.file_name().to_string_lossy().starts_with("com_apple_MobileAsset_Font") {
                out.push(entry.path());
            }
        }
    }
    out.sort();
    out
}

pub(crate) fn mac_asset_roots() -> Vec<PathBuf> {
    asset_roots(&["/System/Library/AssetsV2".into(), "/System/Library/AssetsV2/PreinstalledAssetsV2/InstallWithOs".into()])
}

#[cfg(test)]
mod tests {
    #[test]
    fn multi_face_collection_survives_bad_and_missing_neighbors() {
        use crate::fonts::{INTER_REGULAR, JETBRAINS_MONO_REGULAR};
        let root = std::env::temp_dir().join(format!("prism-font-collection-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut ttc = b"ttcf\0\x01\0\0\0\0\0\x02".to_vec();
        ttc.extend_from_slice(&[0; 8]);
        for (index, font) in [INTER_REGULAR, JETBRAINS_MONO_REGULAR].iter().enumerate() {
            while !ttc.len().is_multiple_of(4) {
                ttc.push(0);
            }
            let base = ttc.len();
            ttc[12 + index * 4..16 + index * 4].copy_from_slice(&(base as u32).to_be_bytes());
            ttc.extend_from_slice(font);
            let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
            for table in 0..tables {
                let offset = base + 12 + table * 16 + 8;
                let old = u32::from_be_bytes(ttc[offset..offset + 4].try_into().unwrap());
                ttc[offset..offset + 4].copy_from_slice(&(old + base as u32).to_be_bytes());
            }
        }
        std::fs::write(root.join("faces.ttc"), ttc).unwrap();
        std::fs::write(root.join("bad.otf"), b"corrupt").unwrap();
        let files = super::files([root.clone()]);
        let mut collection = parley::fontique::Collection::new(parley::fontique::CollectionOptions { shared: false, system_fonts: false });
        for file in files.into_iter().chain([root.join("unreadable.ttf")]) {
            collection.load_fonts_from_paths([file]);
        }
        assert!(collection.family_id("Inter").is_some());
        assert!(collection.family_id("JetBrains Mono").is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn assets_versions_collections_duplicates_and_cycles() {
        let root = std::env::temp_dir().join(format!("prism-font-discovery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for version in [8, 99] {
            let dir = root.join(format!("com_apple_MobileAsset_Font{version}/test.asset/AssetData"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("Font.TTC"), b"fixture").unwrap();
            std::fs::write(dir.join("ignore.txt"), b"fixture").unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, root.join("com_apple_MobileAsset_Font99/test.asset/AssetData/cycle")).unwrap();
        let roots = super::asset_roots(std::slice::from_ref(&root));
        assert_eq!(roots.len(), 2);
        let found = super::files(roots.iter().cloned().chain(roots.iter().cloned()).chain([root.join("missing")]));
        assert_eq!(found.len(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }
}
