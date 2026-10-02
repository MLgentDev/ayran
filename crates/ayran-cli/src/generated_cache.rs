//! Filesystem lifecycle shared by generated Session directories.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use ayran_core::harness::Harness;

pub fn root() -> Option<PathBuf> {
    env::var_os("XDG_CACHE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .map(|root| root.join("ayran"))
}

/// Best-effort cleanup after the current launch's generated directory is touched.
pub fn prune(current: &[&Path]) {
    let Some(root) = root() else {
        return;
    };
    let now = SystemTime::now();
    let max_age = Duration::from_secs(30 * 24 * 60 * 60);
    for harness in [Harness::Claude, Harness::Codex, Harness::Copilot] {
        let directory = root.join(harness.binary());
        // Never traverse a symlink in place of an ayran-owned Harness directory.
        if !fs::symlink_metadata(&directory).is_ok_and(|metadata| metadata.is_dir()) {
            continue;
        }
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if current.contains(&path.as_path()) {
                continue;
            }
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.is_dir()
                && let Ok(modified) = metadata.modified()
                && let Ok(age) = now.duration_since(modified)
                && age > max_age
            {
                let _ = fs::remove_dir_all(path);
            }
        }
    }
}

/// Build a complete directory before publishing it, then refresh its last use.
pub fn materialize(
    directory: &Path,
    build: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    if !directory.is_dir() {
        let parent = directory
            .parent()
            .expect("generated directory has a parent");
        fs::create_dir_all(parent)?;
        let temporary = tempfile::Builder::new()
            .prefix(".generated-")
            .tempdir_in(parent)?;
        build(temporary.path())?;
        if let Err(error) = fs::rename(temporary.path(), directory)
            && !directory.is_dir()
        {
            return Err(error);
        }
    }
    filetime::set_file_mtime(directory, filetime::FileTime::now())?;
    Ok(())
}
