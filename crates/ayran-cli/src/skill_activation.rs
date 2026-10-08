//! Filesystem boundary for path Skill names and generated-directory lifecycle.

use std::fs;
use std::path::{Component, Path};

use ayran_core::cache::GeneratedSkills;
use ayran_core::config::{ConfigLayers, SkillBinding};
use ayran_core::diagnostic::Diagnostic;
use ayran_core::harness::Harness;
use ayran_core::skills::SkillState;

pub fn read(
    selected: &[String],
    layers: &ConfigLayers,
    harness: Harness,
) -> Result<SkillState, Diagnostic> {
    let mut state = SkillState::default();
    if selected.is_empty() || harness == Harness::Codex {
        return Ok(state);
    }
    for logical in selected {
        let Some(skill) = layers.skills.get(logical) else {
            continue;
        };
        if let Some(SkillBinding::Builtin(name)) = skill.value.binding(harness) {
            state
                .builtin
                .insert(name.clone(), crate::builtin_skills::directory(name)?);
            continue;
        }
        let Some(SkillBinding::Path(path)) = skill.value.binding(harness) else {
            continue;
        };
        if state.names.contains_key(path) {
            continue;
        }
        let contents = fs::read_to_string(path.join("SKILL.md")).map_err(|error| {
            Diagnostic::error(
                "path-not-found",
                format!(
                    "Skill path {} must contain a readable SKILL.md: {error}",
                    path.display()
                ),
                None,
            )
        })?;
        let name = skill_name(path, &contents)?;
        state.names.insert(path.clone(), name);
    }
    if !state.names.is_empty() || !state.builtin.is_empty() {
        state.cache_root = crate::generated_cache::root().ok_or_else(|| {
            Diagnostic::error(
                "config-invalid",
                crate::generated_cache::missing_root_message("Skill"),
                None,
            )
        })?;
    }
    Ok(state)
}

pub(crate) fn skill_name(path: &Path, contents: &str) -> Result<String, Diagnostic> {
    let failure = |message: String| {
        Diagnostic::error(
            "config-invalid",
            format!("{}: {message}", path.join("SKILL.md").display()),
            None,
        )
    };
    let mut lines = contents.trim_start_matches('\u{feff}').lines();
    let name = if lines.next() == Some("---") {
        let mut frontmatter = String::new();
        let mut closed = false;
        for line in lines {
            if line == "---" || line == "..." {
                closed = true;
                break;
            }
            frontmatter.push_str(line);
            frontmatter.push('\n');
        }
        if !closed {
            return Err(failure("unterminated frontmatter".into()));
        }
        let fields: serde_yaml::Value =
            serde_yaml::from_str(&frontmatter).map_err(|error| failure(error.to_string()))?;
        match fields.get("name") {
            Some(value) => Some(
                value
                    .as_str()
                    .ok_or_else(|| failure("frontmatter name must be a string".into()))?
                    .to_owned(),
            ),
            None => None,
        }
    } else {
        None
    };
    let name = name
        .or_else(|| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .ok_or_else(|| failure("Skill directory has no name".into()))?;
    // Names become a single symlink entry; never let frontmatter escape the cache.
    let mut components = Path::new(&name).components();
    if name.is_empty()
        || name.contains(['/', '\\'])
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err(failure(format!("invalid Skill name {name:?}")));
    }
    Ok(name)
}

pub fn materialize(cache: &GeneratedSkills) -> Result<(), Diagnostic> {
    for target in cache.builtin.iter() {
        crate::builtin_skills::materialize(target)?;
    }
    crate::generated_cache::materialize(&cache.directory, |temporary| {
        let skills = if cache.harness == Harness::Copilot {
            let plugin = temporary.join("ayran");
            fs::create_dir_all(&plugin)?;
            fs::write(
                plugin.join("plugin.json"),
                r#"{"name":"ayran","version":"1.0.0","skills":"./skills"}"#,
            )?;
            plugin.join("skills")
        } else {
            temporary.join(".claude/skills")
        };
        fs::create_dir_all(&skills)?;
        for (name, target) in &cache.targets {
            symlink_dir(target, &skills.join(name))?;
        }
        Ok(())
    })
    .map_err(|error| {
        Diagnostic::error(
            "config-invalid",
            format!(
                "cannot prepare Skill cache {}: {error}",
                cache.directory.display()
            ),
            None,
        )
    })
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}
#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}
