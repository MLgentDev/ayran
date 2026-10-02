//! Read-only discovery of Copilot standalone Skills.

use std::collections::BTreeMap;
use std::env;
use std::path::Path;

use crate::skill_enumeration::{exists, read_root, same_root};
use ayran_core::diagnostic::Diagnostic;
use ayran_core::harness::Harness;
use ayran_core::skills::SkillState;

pub fn read(
    state: &mut SkillState,
    real_home: Option<&Path>,
    harness_home: &crate::harness_home::HarnessHome,
) -> Result<(), Diagnostic> {
    let home = harness_home.directory.clone();
    let cwd = env::current_dir()
        .map_err(|error| Diagnostic::error("enumeration-failed", error.to_string(), None))?;
    let settings = home.join("settings.json");
    let failure = |message: String| {
        Diagnostic::error(
            "enumeration-failed",
            format!(
                "cannot read Copilot settings {}: {message}",
                settings.display()
            ),
            None,
        )
    };
    match std::fs::read_to_string(&settings) {
        Ok(contents) => {
            let value: serde_json::Value = serde_json_lenient::from_str(&contents)
                .map_err(|error| failure(error.to_string()))?;
            if !value.is_object() {
                return Err(failure("settings must be an object".into()));
            }
            if let Some(directories) = value.get("skillDirectories") {
                let directories: Vec<String> = serde_json::from_value(directories.clone())
                    .map_err(|error| failure(error.to_string()))?;
                for directory in directories {
                    // Copilot resolves relative paths from cwd and does not expand ~.
                    read_root(
                        &cwd.join(directory),
                        &mut state.custom,
                        &mut BTreeMap::new(),
                        Harness::Copilot,
                    )?;
                }
            }
            if let Some(disabled) = value.get("disabledSkills") {
                let names: Vec<String> = serde_json::from_value(disabled.clone())
                    .map_err(|error| failure(error.to_string()))?;
                state.disabled.extend(names);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(failure(error.to_string())),
    }
    let mut personal = vec![home.join("skills")];
    if let Some(real_home) =
        real_home.filter(|_| harness_home.mode == ayran_core::launch::HomeMode::Shared)
    {
        personal.push(real_home.join(".agents/skills"));
    }
    for root in &personal {
        read_root(
            root,
            &mut state.personal,
            &mut BTreeMap::new(),
            Harness::Copilot,
        )?;
    }
    for directory in cwd.ancestors() {
        for relative in [".github/skills", ".agents/skills", ".claude/skills"] {
            let root = directory.join(relative);
            if !personal.iter().any(|personal| same_root(&root, personal))
                && (relative == ".github/skills"
                    || real_home.is_none_or(|home| !same_root(&root, &home.join(relative))))
            {
                read_root(
                    &root,
                    &mut state.project,
                    &mut BTreeMap::new(),
                    Harness::Copilot,
                )?;
            }
        }
        if exists(&directory.join(".git"))? {
            break;
        }
    }
    Ok(())
}
