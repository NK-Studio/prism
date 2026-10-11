//! Copy legacy settings into a new Prism directory without altering the original.
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Existing Prism settings win. Publish a complete copy atomically; partial copies are never read.
pub fn migrate(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.try_exists()? || !source.try_exists()? {
        return Ok(());
    }
    let parent = destination.parent().ok_or_else(|| io::Error::other("settings directory has no parent"))?;
    std::fs::create_dir_all(parent)?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let staging = parent.join(format!(".prism-migrate-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    // Create unfinished copies privately before any files are copied.
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(&staging)?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir(&staging)?;
    let result = (|| {
        let mut entries = 0;
        copy_tree(source, &staging, 0, &mut entries)?;
        // Preserve historical logs under their new names; the original directory is untouched.
        for (old, new) in [("photocraft.log", "prism.log"), ("photocraft.1.log", "prism.1.log"), ("photocraft.2.log", "prism.2.log")] {
            let old = staging.join("logs").join(old);
            let new = staging.join("logs").join(new);
            if old.is_file() && !new.try_exists()? {
                std::fs::rename(old, new)?;
            }
        }
        if destination.try_exists()? {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "Prism settings appeared during migration"));
        }
        std::fs::rename(&staging, destination)
    })();
    // This directory was created exclusively above; cleanup cannot remove an existing user folder.
    if result.is_err() {
        let _ = std::fs::remove_dir_all(staging);
    }
    result
}

fn copy_tree(source: &Path, destination: &Path, depth: usize, entries: &mut usize) -> io::Result<()> {
    if depth > 64 || *entries >= 100_000 {
        return Err(io::Error::other("settings migration exceeds the directory limit"));
    }
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        *entries = entries.saturating_add(1);
        if *entries > 100_000 {
            return Err(io::Error::other("settings migration exceeds the file limit"));
        }
        let kind = entry.file_type()?;
        let to = destination.join(entry.file_name());
        if kind.is_symlink() {
            return Err(io::Error::other("settings migration cannot copy symbolic links; original settings are retained"));
        }
        if kind.is_dir() {
            std::fs::create_dir(&to)?;
            copy_tree(&entry.path(), &to, depth.saturating_add(1), entries)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), to)?;
        } else {
            return Err(io::Error::other("settings migration encountered a special file"));
        }
    }
    std::fs::set_permissions(destination, std::fs::metadata(source)?.permissions())
}
