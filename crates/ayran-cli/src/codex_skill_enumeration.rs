//! Read-only Codex discovery; overrides identify canonical SKILL.md files.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io;
use std::path::Path;

use ayran_core::diagnostic::Diagnostic;
use ayran_core::skills::{CodexSkill, SkillState};

#[derive(Clone, Copy)]
enum Scope {
    Personal,
    Project,
    Bundled,
    Admin,
}

pub fn read(
    state: &mut SkillState,
    real_home: Option<&Path>,
    harness_home: &crate::harness_home::HarnessHome,
) -> Result<(), Diagnostic> {
    let cwd = env::current_dir().map_err(|error| failure(Path::new("."), error))?;
    let home = harness_home.directory.clone();
    let mut personal = vec![home.join("skills")];
    if let Some(home) =
        real_home.filter(|_| harness_home.mode == ayran_core::launch::HomeMode::Shared)
    {
        personal.push(home.join(".agents/skills"));
    }
    for root in &personal {
        read_root(root, Scope::Personal, state)?;
    }
    read_root(&home.join("skills/.system"), Scope::Bundled, state)?;
    read_root(Path::new("/etc/codex/skills"), Scope::Admin, state)?;

    let directories = crate::codex_config::project_directories(&home, &cwd)?;
    for directory in &directories {
        let root = directory.join(".agents/skills");
        if !personal.iter().any(|user| same_root(&root, user))
            && real_home.is_none_or(|home| !same_root(&root, &home.join(".agents/skills")))
        {
            read_root(&root, Scope::Project, state)?;
        }
    }
    if let Some(project) = directories.last() {
        let root = project.join(".codex/skills");
        if !personal.iter().any(|user| same_root(&root, user))
            && real_home.is_none_or(|home| !same_root(&root, &home.join(".codex/skills")))
        {
            read_root(&root, Scope::Project, state)?;
        }
    }
    Ok(())
}

fn same_root(left: &Path, right: &Path) -> bool {
    left == right
        || matches!((left.canonicalize(), right.canonicalize()), (Ok(left), Ok(right)) if left == right)
}

fn read_root(root: &Path, scope: Scope, state: &mut SkillState) -> Result<(), Diagnostic> {
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut visited = BTreeSet::new();
    while let Some((directory, depth)) = pending.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound && depth == 0 => continue,
            Err(error) => return Err(failure(&directory, error)),
        };
        let canonical = directory
            .canonicalize()
            .map_err(|error| failure(&directory, error))?;
        if !visited.insert(canonical) {
            continue;
        }
        for entry in entries {
            let entry = entry.map_err(|error| failure(&directory, error))?;
            let path = entry.path();
            let kind = entry.file_type().map_err(|error| failure(&path, error))?;
            if kind.is_symlink() && matches!(scope, Scope::Bundled) {
                continue;
            }
            let metadata = fs::metadata(&path).map_err(|error| failure(&path, error))?;
            if kind.is_symlink() && !metadata.is_dir() {
                continue;
            }
            if metadata.is_dir() {
                if depth < 6 && !entry.file_name().to_string_lossy().starts_with('.') {
                    pending.push((path, depth + 1));
                }
                continue;
            }
            if entry.file_name() != "SKILL.md" {
                continue;
            }
            let contents = fs::read_to_string(&path).map_err(|error| failure(&path, error))?;
            let path = path.canonicalize().map_err(|error| failure(&path, error))?;
            let Some(mut name) = skill_name(&path, &contents) else {
                continue;
            };
            if let Some(namespace) = plugin_namespace(&path)? {
                name = format!("{namespace}:{name}");
            }
            if path.to_str().is_none() {
                return Err(failure(&path, "Skill path is not UTF-8"));
            }
            state
                .codex
                .entry(path)
                .and_modify(|skill| skill.personal |= matches!(scope, Scope::Personal))
                .or_insert(CodexSkill {
                    name,
                    personal: matches!(scope, Scope::Personal),
                });
        }
    }
    Ok(())
}

fn skill_name(path: &Path, contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut yaml = Vec::new();
    let mut closed = false;
    for line in lines {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        yaml.push(line.to_owned());
    }
    if !closed {
        return None;
    }
    // Codex repairs invalid plain YAML scalars used by third-party Skills.
    // Only retry after strict parsing fails; valid YAML retains its meaning.
    let fields: serde_yaml::Value = serde_yaml::from_str(&yaml.join("\n")).ok().or_else(|| {
        for line in &mut yaml {
            if let Some((key, value)) = line.split_once(':') {
                let value = value.trim();
                let comment = value.char_indices().find_map(|(index, character)| {
                    (character == '#'
                        && (index == 0 || value[..index].ends_with(char::is_whitespace)))
                    .then_some(index)
                });
                let (scalar, comment) =
                    comment.map_or((value, ""), |index| (&value[..index], &value[index..]));
                let scalar = scalar.trim_end();
                if !scalar.is_empty()
                    && !scalar.starts_with(['\'', '"', '|', '>'])
                    && (scalar
                        .split(':')
                        .skip(1)
                        .any(|part| part.starts_with(char::is_whitespace))
                        || serde_yaml::from_str::<serde_yaml::Value>(scalar).is_err())
                {
                    *line = format!("{key}: '{}' {comment}", scalar.replace('\'', "''"));
                }
            }
        }
        serde_yaml::from_str(&yaml.join("\n")).ok()
    })?;
    if fields.get("description")?.as_str()?.trim().is_empty() {
        return None;
    }
    if let Some(metadata) = fields.get("metadata") {
        let metadata = metadata.as_mapping()?;
        if let Some(value) = metadata.get("short-description")
            && !value.is_null()
            && !value.is_string()
        {
            return None;
        }
    }
    let name = match fields.get("name") {
        Some(serde_yaml::Value::Null) | None => None,
        Some(value) => Some(
            value
                .as_str()?
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        ),
    }
    .filter(|name| !name.is_empty())
    .or_else(|| {
        path.parent()?
            .file_name()?
            .to_str()
            .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
    })?;
    (!name.is_empty() && name.chars().count() <= 64).then_some(name)
}

fn plugin_namespace(skill: &Path) -> Result<Option<String>, Diagnostic> {
    for directory in skill.parent().into_iter().flat_map(Path::ancestors) {
        for manifest in [
            ".codex-plugin/plugin.json",
            ".claude-plugin/plugin.json",
            ".cursor-plugin/plugin.json",
        ] {
            let path = directory.join(manifest);
            let contents = match fs::read_to_string(&path) {
                Ok(contents) => contents,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(failure(&path, error)),
            };
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) {
                let name = value
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .or_else(|| directory.file_name()?.to_str());
                return Ok(name.map(str::to_owned));
            }
            // Codex stops at the first candidate manifest in this directory.
            break;
        }
    }
    Ok(None)
}

fn failure(path: &Path, message: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error(
        "enumeration-failed",
        format!(
            "cannot enumerate Codex Skills from {}: {message}",
            path.display()
        ),
        None,
    )
}
