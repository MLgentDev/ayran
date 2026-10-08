//! Embedded Skill trees and their immutable generated-cache directories.
use ayran_core::diagnostic::Diagnostic;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

include!(concat!(env!("OUT_DIR"), "/builtin_skills.rs"));

pub(crate) fn files() -> &'static [(&'static str, &'static [u8])] {
    FILES
}

pub(crate) fn directories() -> &'static [&'static str] {
    DIRECTORIES
}

pub fn directory(name: &str) -> Result<PathBuf, Diagnostic> {
    let root = crate::generated_cache::root().ok_or_else(|| {
        Diagnostic::error(
            "config-invalid",
            crate::generated_cache::missing_root_message("built-in Skill"),
            None,
        )
    })?;
    // Names and byte lengths keep file boundaries unambiguous. Binary versions are
    // deliberately excluded: identical embedded content reuses the same tree.
    let mut digest = Sha256::new();
    for path in DIRECTORIES {
        digest.update((path.len() as u64).to_be_bytes());
        digest.update(path.as_bytes());
    }
    for (path, bytes) in FILES {
        digest.update((path.len() as u64).to_be_bytes());
        digest.update(path.as_bytes());
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    }
    Ok(root
        .join("builtin-skills")
        .join(format!("{name}-{:x}", digest.finalize())))
}

pub fn materialize(directory: &Path) -> Result<(), Diagnostic> {
    crate::generated_cache::materialize(directory, |temporary| {
        for relative in DIRECTORIES {
            fs::create_dir_all(temporary.join(relative))?;
        }
        for (relative, bytes) in FILES {
            let path = temporary.join(relative);
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(&path, bytes)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o644))?;
            }
        }
        Ok(())
    })
    .map_err(|error| {
        Diagnostic::error(
            "config-invalid",
            format!(
                "cannot prepare built-in Skill cache {}: {error}",
                directory.display()
            ),
            None,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn embedded_tree_matches_source_files() {
        fn walk(root: &Path, relative: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(root.join(relative)).unwrap() {
                let entry = entry.unwrap();
                let path = relative.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    walk(root, &path, files);
                } else {
                    files.insert(
                        path.to_str().unwrap().replace('\\', "/"),
                        fs::read(entry.path()).unwrap(),
                    );
                }
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.agents/skills/ayran");
        let mut files = BTreeMap::new();
        walk(&root, Path::new(""), &mut files);
        let embedded: BTreeMap<_, _> = FILES
            .iter()
            .map(|(path, bytes)| (path.to_string(), bytes.to_vec()))
            .collect();
        assert_eq!(embedded, files);
    }
}
