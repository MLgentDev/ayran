//! Read-only Codex project config discovery shared by Capability enumerators.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ayran_core::diagnostic::Diagnostic;

pub fn project_directories<'a>(home: &Path, cwd: &'a Path) -> Result<Vec<&'a Path>, Diagnostic> {
    let markers = project_root_markers(home)?;
    let mut directories = Vec::new();
    for directory in cwd.ancestors() {
        directories.push(directory);
        for marker in &markers {
            let path = directory.join(marker);
            let metadata = match fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(failure(&path, error)),
            };
            if marker != ".git"
                || !metadata.is_dir()
                || crate::skill_enumeration::exists(&path.join("HEAD"))?
            {
                return Ok(directories);
            }
        }
    }
    directories.truncate(1);
    Ok(directories)
}

fn project_root_markers(home: &Path) -> Result<Vec<String>, Diagnostic> {
    let mut markers = vec![".git".to_owned()];
    for path in [
        PathBuf::from("/etc/codex/config.toml"),
        home.join("config.toml"),
    ] {
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(failure(&path, error)),
        };
        let config: toml::Table = contents.parse().map_err(|error| failure(&path, error))?;
        if let Some(value) = config.get("project_root_markers") {
            markers = value
                .as_array()
                .and_then(|values| {
                    values
                        .iter()
                        .map(|value| value.as_str().map(str::to_owned))
                        .collect()
                })
                .ok_or_else(|| failure(&path, "project_root_markers must be a string array"))?;
        }
    }
    Ok(markers)
}

fn failure(path: &Path, message: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error(
        "enumeration-failed",
        format!(
            "cannot read Codex config from {}: {message}",
            path.display()
        ),
        None,
    )
}
