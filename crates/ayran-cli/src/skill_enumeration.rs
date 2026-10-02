//! Read-only discovery of standalone Claude Skills at the filesystem boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ayran_core::diagnostic::Diagnostic;
use ayran_core::harness::Harness;
use ayran_core::skills::SkillState;

pub fn read_claude(
    state: &mut SkillState,
    real_home: Option<&Path>,
    harness_home: &crate::harness_home::HarnessHome,
) -> Result<(), Diagnostic> {
    let home = harness_home.directory.clone();
    let personal = home.join("skills");
    let mut aliases = BTreeMap::new();
    read_root(
        &personal,
        &mut state.personal,
        &mut aliases,
        Harness::Claude,
    )?;
    state.aliases.extend(aliases);
    if !env::var("CLAUDE_CODE_DISABLE_POLICY_SKILLS").is_ok_and(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    }) {
        let mut aliases = BTreeMap::new();
        read_root(
            &managed_skill_root(),
            &mut state.enterprise,
            &mut aliases,
            Harness::Claude,
        )?;
        state.aliases.extend(aliases);
    }
    // Bundled Skills are embedded, rather than installed in a directory.
    // This bounded list comes from the unconditional Skill entries in Claude's
    // command reference.
    state.bundled.extend(
        [
            "batch",
            "claude-api",
            "code-review",
            "dataviz",
            "debug",
            "doctor",
            "fewer-permission-prompts",
            "loop",
            "run",
            "run-skill-generator",
            "simplify",
            "update-config",
            "verify",
        ]
        .map(str::to_owned),
    );
    let cwd = env::current_dir().map_err(|error| failure(Path::new("."), error))?;
    for directory in cwd.ancestors() {
        let root = directory.join(".claude/skills");
        // The HOME skill root remains personal even in a dotfiles repository.
        if !same_root(&root, &personal)
            && real_home.is_none_or(|home| !same_root(&root, &home.join(".claude/skills")))
        {
            let mut aliases = BTreeMap::new();
            read_root(&root, &mut state.project, &mut aliases, Harness::Claude)?;
            for (alias, name) in aliases {
                state.aliases.entry(alias).or_insert(name);
            }
        }
        if exists(&directory.join(".git"))? {
            break;
        }
    }
    Ok(())
}

fn managed_skill_root() -> PathBuf {
    #[cfg(target_os = "macos")]
    let directory = "/Library/Application Support/ClaudeCode";
    #[cfg(windows)]
    let directory = r"C:\Program Files\ClaudeCode";
    #[cfg(not(any(target_os = "macos", windows)))]
    let directory = "/etc/claude-code";
    Path::new(directory).join(".claude/skills")
}

pub(crate) fn same_root(left: &Path, right: &Path) -> bool {
    left == right
        || matches!((left.canonicalize(), right.canonicalize()), (Ok(left), Ok(right)) if left == right)
}

pub(crate) fn exists(path: &Path) -> Result<bool, Diagnostic> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(failure(path, error)),
    }
}

pub(crate) fn read_root(
    root: &Path,
    names: &mut BTreeSet<String>,
    aliases: &mut BTreeMap<String, String>,
    harness: Harness,
) -> Result<(), Diagnostic> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(failure(root, error)),
    };
    for entry in entries {
        let entry = entry.map_err(|error| failure(root, error))?;
        let path = entry.path();
        if harness == Harness::Claude
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.eq_ignore_ascii_case("synced"))
        {
            continue;
        }
        if !fs::metadata(&path)
            .map_err(|error| failure(&path, error))?
            .is_dir()
        {
            continue;
        }
        // Read the directory even if it has no SKILL.md: unreadable state is an error.
        fs::read_dir(&path).map_err(|error| failure(&path, error))?;
        if harness == Harness::Claude && exists(&path.join(".claude-plugin/plugin.json"))? {
            continue;
        }
        let skill = path.join("SKILL.md");
        let contents = match fs::read_to_string(&skill) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(failure(&skill, error)),
        };
        let name = crate::skill_activation::skill_name(&path, &contents)
            .map_err(|diagnostic| failure(&skill, diagnostic.message))?;
        let directory = entry
            .file_name()
            .into_string()
            .map_err(|_| failure(&path, "Skill directory name is not UTF-8"))?;
        if directory != name {
            aliases.insert(directory, name.clone());
        }
        names.insert(name);
    }
    Ok(())
}

fn failure(path: &Path, message: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error(
        "enumeration-failed",
        format!("cannot enumerate Skills from {}: {message}", path.display()),
        None,
    )
}
