//! Owned full-tree snapshots, verified without writing during launch or inspection.
use ayran_core::diagnostic::Diagnostic;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const RECORD: &str = ".ayran-snapshot.json";
#[derive(Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Identity {
    version: u32,
    pub source: PathBuf,
    pub name: String,
    hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<GitIdentity>,
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GitIdentity {
    pub binding: ayran_core::config::GitSkillBinding,
    pub commit: String,
    pub logical: String,
}
pub(crate) fn error(code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::error(code, message, None)
}
fn invalid(e: impl std::fmt::Display) -> Diagnostic {
    error("skill-install-invalid", e.to_string())
}
fn files(
    root: &Path,
    relative: &Path,
    result: &mut BTreeMap<PathBuf, Vec<u8>>,
) -> Result<(), Diagnostic> {
    for entry in fs::read_dir(root.join(relative)).map_err(invalid)? {
        let entry = entry.map_err(invalid)?;
        let path = relative.join(entry.file_name());
        if path == Path::new(RECORD) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(invalid)?;
        if metadata.is_dir() {
            // Include empty directories in the snapshot identity.
            result.insert(path.clone(), vec![0]);
            files(root, &path, result)?;
        } else if metadata.is_file() {
            let mut bytes = vec![1];
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                bytes.extend_from_slice(&(metadata.permissions().mode() & 0o777).to_le_bytes());
            }
            bytes.extend(fs::read(entry.path()).map_err(invalid)?);
            result.insert(path, bytes);
        } else {
            return Err(invalid(format!(
                "{}: symlinks and special files are not snapshot sources",
                entry.path().display()
            )));
        }
    }
    Ok(())
}
fn hash(root: &Path) -> Result<String, Diagnostic> {
    let mut entries = BTreeMap::new();
    files(root, Path::new(""), &mut entries)?;
    hash_entries(entries)
}
fn hash_entries(entries: BTreeMap<PathBuf, Vec<u8>>) -> Result<String, Diagnostic> {
    let mut digest = Sha256::new();
    for (path, content) in entries {
        let path = path
            .to_str()
            .ok_or_else(|| invalid("Skill tree paths must be UTF-8"))?;
        digest.update((path.len() as u64).to_le_bytes());
        digest.update(path.as_bytes());
        digest.update((content.len() as u64).to_le_bytes());
        digest.update(content);
    }
    Ok(format!("{:x}", digest.finalize()))
}
/// Hash the embedded tree using the same identity as installed snapshots.
pub(crate) fn builtin(name: &str) -> Result<Identity, Diagnostic> {
    let mut entries = BTreeMap::new();
    for relative in crate::builtin_skills::directories() {
        entries.insert(PathBuf::from(relative), vec![0]);
    }
    for (relative, bytes) in crate::builtin_skills::files() {
        let path = PathBuf::from(relative);
        let mut content = vec![1];
        #[cfg(unix)]
        content.extend_from_slice(&0o644_u32.to_le_bytes());
        content.extend_from_slice(bytes);
        entries.insert(path, content);
    }
    Ok(Identity {
        version: 1,
        source: PathBuf::from(format!("builtin:{name}")),
        name: name.to_owned(),
        hash: hash_entries(entries)?,
        git: None,
    })
}
pub(crate) fn source(path: &Path) -> Result<Identity, Diagnostic> {
    let source = path.canonicalize().map_err(invalid)?;
    if crate::skill_enumeration::belongs_to_plugin(&source.join("SKILL.md"))? {
        return Err(invalid(
            "Plugin Skills cannot be installed as standalone Skills",
        ));
    }
    if source.join(RECORD).exists() {
        return Err(invalid("source contains reserved snapshot metadata"));
    }
    let text = fs::read_to_string(source.join("SKILL.md")).map_err(invalid)?;
    let frontmatter = text
        .trim_start_matches('\u{feff}')
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
        .ok_or_else(|| invalid("Skill install requires frontmatter name and description"))?;
    let fields: serde_yaml::Value = serde_yaml::from_str(
        &frontmatter
            .lines()
            .take_while(|l| *l != "---" && *l != "...")
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .map_err(invalid)?;
    for field in ["name", "description"] {
        if fields
            .get(field)
            .and_then(serde_yaml::Value::as_str)
            .is_none_or(|v| v.trim().is_empty())
        {
            return Err(invalid(format!("Skill frontmatter requires {field}")));
        }
    }
    let name = crate::skill_activation::skill_name(&source, &text)?;
    if name.starts_with('.')
        || name.eq_ignore_ascii_case("synced")
        || name.contains([':', '<', '>', '"', '|', '?', '*'])
        || name.chars().any(char::is_control)
        || name.ends_with(['.', ' '])
    {
        return Err(invalid(
            "Skill name is reserved or invalid for a user Skill root",
        ));
    }
    let hash = hash(&source)?;
    Ok(Identity {
        version: 1,
        git: None,
        source,
        name,
        hash,
    })
}
pub(crate) fn verify(destination: &Path, expected: &Identity) -> Result<bool, Diagnostic> {
    let metadata = match fs::symlink_metadata(destination) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(invalid(e)),
    };
    let conflict = || {
        error(
            "skill-install-conflict",
            format!(
                "{} is not an identical owned Skill snapshot",
                destination.display()
            ),
        )
    };
    if !metadata.is_dir() {
        return Err(conflict());
    }
    let record = destination.join(RECORD);
    if fs::symlink_metadata(&record)
        .map_err(|_| conflict())?
        .file_type()
        .is_symlink()
    {
        return Err(conflict());
    }
    let actual: Identity = serde_json::from_slice(&fs::read(record).map_err(|_| conflict())?)
        .map_err(|_| conflict())?;
    if actual != *expected || hash(destination)? != expected.hash {
        return Err(conflict());
    }
    Ok(true)
}
/// Read and validate an existing snapshot independently of the new declaration.
pub(crate) fn installed(destination: &Path) -> Result<Option<Identity>, Diagnostic> {
    match fs::symlink_metadata(destination) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(invalid(e)),
        Ok(_) => {}
    }
    let conflict = || {
        error(
            "skill-install-conflict",
            format!(
                "{} is not an unmodified owned Skill snapshot",
                destination.display()
            ),
        )
    };
    let identity: Identity =
        serde_json::from_slice(&fs::read(destination.join(RECORD)).map_err(|_| conflict())?)
            .map_err(|_| conflict())?;
    if identity.version != 1
        || destination
            .file_name()
            .is_none_or(|n| n != identity.name.as_str())
        || !verify(destination, &identity).map_err(|_| conflict())?
    {
        return Err(conflict());
    }
    let text = fs::read_to_string(destination.join("SKILL.md")).map_err(|_| conflict())?;
    if crate::skill_activation::skill_name(destination, &text)? != identity.name
        || crate::skill_enumeration::belongs_to_plugin(&destination.join("SKILL.md"))?
    {
        return Err(conflict());
    }
    Ok(Some(identity))
}

fn copy(source: &Path, destination: &Path) -> Result<(), Diagnostic> {
    fs::create_dir_all(destination).map_err(invalid)?;
    for entry in fs::read_dir(source).map_err(invalid)? {
        let entry = entry.map_err(invalid)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(invalid)?;
        let target = destination.join(entry.file_name());
        if metadata.is_dir() {
            copy(&entry.path(), &target)?;
        } else if metadata.is_file() {
            fs::copy(entry.path(), target).map_err(invalid)?;
        } else {
            return Err(invalid("source changed to a symlink or special file"));
        }
    }
    Ok(())
}
pub(crate) fn publish(
    destination: &Path,
    identity: &Identity,
    previous: Option<&(Identity, PathBuf)>,
    copy_source: &Path,
) -> Result<&'static str, Diagnostic> {
    // Revalidate the plan before staging and immediately before publication.
    let validate = || -> Result<(), Diagnostic> {
        if let Some((old, path)) = previous {
            if !verify(path, old)? {
                return Err(error(
                    "skill-install-conflict",
                    "previous snapshot disappeared; retry",
                ));
            }
            if path != destination && installed(destination)?.is_some() {
                return Err(error(
                    "skill-install-conflict",
                    "replacement destination appeared; retry",
                ));
            }
        } else if installed(destination)?.is_some() {
            return Err(error(
                "skill-install-conflict",
                "snapshot destination appeared; retry",
            ));
        }
        Ok(())
    };
    validate()?;
    if previous.is_some_and(|(old, path)| old == identity && path == destination) {
        return Ok("unchanged");
    }
    let root = destination.parent().unwrap();
    fs::create_dir_all(root).map_err(invalid)?;
    let stage = tempfile::tempdir_in(root).map_err(invalid)?;
    copy(copy_source, stage.path())?;
    if hash(stage.path())? != identity.hash {
        return Err(invalid("source changed during snapshot copy; retry"));
    }
    fs::write(
        stage.path().join(RECORD),
        serde_json::to_vec(identity).map_err(invalid)?,
    )
    .map_err(invalid)?;
    validate()?;
    if let Some((_, old_path)) = previous {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            rustix::fs::renameat_with(
                rustix::fs::CWD,
                stage.path(),
                rustix::fs::CWD,
                old_path,
                rustix::fs::RenameFlags::EXCHANGE,
            )
            .map_err(invalid)?;
            // Keep a visible snapshot until the native-name move has succeeded.
            // The staging directory holds the old tree until we commit the move.
            if old_path != destination
                && let Err(e) = publish_new(old_path, destination)
            {
                if let Err(restore) = rustix::fs::renameat_with(
                    rustix::fs::CWD,
                    stage.path(),
                    rustix::fs::CWD,
                    old_path,
                    rustix::fs::RenameFlags::EXCHANGE,
                ) {
                    let retained = stage.keep();
                    return Err(invalid(format!(
                        "{}; could not restore previous snapshot: {restore}; retained at {}",
                        e.message,
                        retained.display()
                    )));
                }
                return Err(e);
            }
            return Ok("replaced");
        }
        // Platforms without directory exchange need a rollback slot.
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let backup = tempfile::tempdir_in(root).map_err(invalid)?;
            let saved = backup.path().join("previous");
            fs::rename(old_path, &saved).map_err(invalid)?;
            if let Err(e) = publish_new(stage.path(), destination) {
                if let Err(restore) = fs::rename(&saved, old_path) {
                    let retained = backup.keep();
                    return Err(invalid(format!(
                        "{}; could not restore previous snapshot: {restore}; retained at {}",
                        e.message,
                        retained.join("previous").display()
                    )));
                }
                return Err(e);
            }
            return Ok("replaced");
        }
    }
    publish_new(stage.path(), destination)?;
    Ok("added")
}

fn publish_new(stage: &Path, destination: &Path) -> Result<(), Diagnostic> {
    // Refuse even an empty destination created by a concurrent installer.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        stage,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|e| {
        error(
            "skill-install-conflict",
            format!("cannot publish snapshot: {e}"),
        )
    })?;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fs::rename(stage, destination).map_err(invalid)?;
    Ok(())
}

/// Project an owned path Binding as native after revalidating snapshot ownership/content.
/// Invalid/uninstalled copies retain their original Binding and its normal Diagnostics.
pub(crate) fn apply(
    layers: &mut ayran_core::config::ConfigLayers,
    harness: ayran_core::harness::Harness,
    home: &Path,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    use ayran_core::{config::SkillBinding, harness::Harness};
    let rows = crate::native_capabilities::read(
        layers,
        &[harness],
        ayran_core::diagnostic::CapabilityKind::Skill,
    )
    .unwrap_or_default();
    let Ok(launch_home) = crate::native_plugins::home(layers, harness) else {
        return diagnostics;
    };
    for (logical, skill) in &mut layers.skills {
        let Some(binding) = skill.value.binding(harness) else {
            continue;
        };
        let Ok(Some((identity, destination))) =
            owned_binding(binding, logical, &home.join("skills"))
        else {
            continue;
        };
        if let SkillBinding::Git(g) = binding
            && g.r#ref.is_none()
        {
            diagnostics.push(Diagnostic {
                severity: ayran_core::diagnostic::Severity::Note,
                ..error(
                    "skill-unpinned",
                    format!("Skill {logical} follows the default branch"),
                )
                .for_capability(
                    harness,
                    ayran_core::diagnostic::CapabilityKind::Skill,
                    logical,
                    &skill.path,
                )
            });
        }
        if let SkillBinding::Git(g) = binding
            && identity.git.as_ref().is_some_and(|i| i.binding != *g)
        {
            diagnostics.push(Diagnostic {
                severity: ayran_core::diagnostic::Severity::Warning,
                hint: Some(format!("ayran install --skill {logical}")),
                ..error(
                    "skill-outdated",
                    format!("Skill {logical} uses a snapshot from an older git declaration"),
                )
                .for_capability(
                    harness,
                    ayran_core::diagnostic::CapabilityKind::Skill,
                    logical,
                    &skill.path,
                )
            });
        }
        if let SkillBinding::Builtin(name) = binding
            && builtin(name).is_ok_and(|current| current.hash != identity.hash)
        {
            diagnostics.push(Diagnostic {
                severity: ayran_core::diagnostic::Severity::Warning,
                hint: Some(format!(
                    "ayran install --skill {logical} --{}",
                    harness.binary()
                )),
                ..error(
                    "skill-outdated",
                    format!("Skill {logical} uses an older built-in snapshot"),
                )
                .for_capability(
                    harness,
                    ayran_core::diagnostic::CapabilityKind::Skill,
                    logical,
                    &skill.path,
                )
            });
        }
        if check_personal(&launch_home, harness, &identity.name, &destination).is_err() {
            continue;
        }
        if rows.iter().filter(|r| r.name == identity.name).any(|r| {
            if harness == Harness::Codex {
                r.path.as_ref().is_none_or(|p| {
                    !crate::skill_enumeration::same_root(p, &destination.join("SKILL.md"))
                })
            } else {
                r.source != "personal"
            }
        }) {
            continue;
        }
        let binding = Some(SkillBinding::Native(identity.name));
        match harness {
            Harness::Claude => skill.value.claude = binding,
            Harness::Codex => skill.value.codex = binding,
            Harness::Copilot => skill.value.copilot = binding,
        }
    }
    diagnostics
}

/// A snapshot is independent of future source bytes; reinstall compares those separately.
pub(crate) fn owned(source: &Path, root: &Path) -> Result<Option<(Identity, PathBuf)>, Diagnostic> {
    let canonical = if source.to_str().is_some_and(|s| s.starts_with("builtin:")) {
        source.to_path_buf()
    } else {
        source
            .canonicalize()
            .unwrap_or_else(|_| source.to_path_buf())
    };
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(invalid(e)),
    };
    let mut found = None;
    for entry in entries {
        let destination = entry.map_err(invalid)?.path();
        let Ok(bytes) = fs::read(destination.join(RECORD)) else {
            continue;
        };
        let Ok(identity) = serde_json::from_slice::<Identity>(&bytes) else {
            continue;
        };
        if identity.source != canonical {
            continue;
        }
        let Some(identity) = installed(&destination)? else {
            continue;
        };
        if found.is_some() {
            return Err(error(
                "skill-install-conflict",
                "source has multiple owned snapshots",
            ));
        }
        found = Some((identity, destination));
    }
    Ok(found)
}

/// Name-set inventories cannot distinguish two personal directories with one name.
pub(crate) fn check_personal(
    home: &crate::harness_home::HarnessHome,
    harness: ayran_core::harness::Harness,
    name: &str,
    destination: &Path,
) -> Result<(), Diagnostic> {
    use ayran_core::{harness::Harness, launch::HomeMode};
    let mut roots = vec![home.directory.join("skills")];
    if harness == Harness::Copilot
        && home.mode == HomeMode::Shared
        && let Some(real) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        roots.push(PathBuf::from(real).join(".agents/skills"));
    }
    for root in roots {
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(invalid(e)),
        };
        for entry in entries {
            let path = entry.map_err(invalid)?.path();
            if crate::skill_enumeration::same_root(&path, destination) {
                continue;
            }
            let skill = path.join("SKILL.md");
            let text = match fs::read_to_string(&skill) {
                Ok(text) => text,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) =>
                {
                    continue;
                }
                Err(e) => return Err(invalid(e)),
            };
            if crate::skill_activation::skill_name(&path, &text)? == name
                && !crate::skill_enumeration::belongs_to_plugin(
                    &skill.canonicalize().map_err(invalid)?,
                )?
            {
                return Err(error(
                    "skill-install-conflict",
                    format!("{name} also belongs to personal Skill {}", path.display()),
                ));
            }
        }
    }
    Ok(())
}

/// Git snapshots are associated with the logical Skill so older declarations remain usable.
pub(crate) fn owned_binding(
    binding: &ayran_core::config::SkillBinding,
    logical: &str,
    root: &Path,
) -> Result<Option<(Identity, PathBuf)>, Diagnostic> {
    use ayran_core::config::SkillBinding;
    if let SkillBinding::Path(p) = binding {
        return owned(p, root);
    }
    if let SkillBinding::Builtin(name) = binding {
        return owned(Path::new(&format!("builtin:{name}")), root);
    }
    if !matches!(binding, SkillBinding::Git(_)) {
        return Ok(None);
    }
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(invalid(e)),
    };
    let mut found = None;
    for entry in entries {
        let destination = entry.map_err(invalid)?.path();
        let Ok(bytes) = fs::read(destination.join(RECORD)) else {
            continue;
        };
        let Ok(identity) = serde_json::from_slice::<Identity>(&bytes) else {
            continue;
        };
        if !identity.git.as_ref().is_some_and(|g| g.logical == logical) {
            continue;
        }
        if let Some(identity) = installed(&destination)? {
            if found.is_some() {
                return Err(error(
                    "skill-install-conflict",
                    "Skill has multiple owned snapshots",
                ));
            }
            found = Some((identity, destination));
        }
    }
    Ok(found)
}
