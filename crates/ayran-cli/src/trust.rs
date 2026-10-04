//! Install-source Trust belongs to ayran user state, never to a Harness home.
use ayran_core::{config::ConfigLayers, diagnostic::Diagnostic};
use clap::{Arg, ArgAction, ArgMatches, Command};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

fn failure(error: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error("trust-store-failed", error.to_string(), None)
}
fn store_path() -> Result<PathBuf, Diagnostic> {
    #[cfg(windows)]
    let root = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let root = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")));
    Ok(root
        .ok_or_else(|| failure("cannot locate ayran user state"))?
        .join("ayran/trust.json"))
}
#[derive(Default)]
pub struct Store(BTreeMap<PathBuf, Approval>);
#[derive(serde::Serialize, serde::Deserialize)]
struct Approval {
    hash: String,
    // Preserve the declaring directory: symlinked layers can resolve different path sources.
    layer: PathBuf,
}
impl Store {
    pub fn read() -> Result<Self, Diagnostic> {
        let path = store_path()?;
        let text = match fs::read(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(failure(e)),
        };
        Ok(Self(serde_json::from_slice(&text).map_err(failure)?))
    }
    pub fn trusted(&self, path: &Path) -> Result<bool, Diagnostic> {
        let canonical = path.canonicalize().map_err(failure)?;
        let Some(expected) = self.0.get(&canonical) else {
            return Ok(false);
        };
        Ok(expected.hash == fingerprint(path)?)
    }
    fn write(&self) -> Result<(), Diagnostic> {
        let path = store_path()?;
        let parent = path.parent().unwrap();
        fs::create_dir_all(parent).map_err(failure)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(failure)?;
        temporary
            .write_all(&serde_json::to_vec_pretty(&self.0).map_err(failure)?)
            .map_err(failure)?;
        temporary.as_file().sync_all().map_err(failure)?;
        temporary.persist(path).map_err(failure)?;
        Ok(())
    }
}
fn fingerprint(path: &Path) -> Result<String, Diagnostic> {
    // Hash install sources semantically; descriptions, Defaults and native Bindings are irrelevant.
    let layer = ConfigLayers::read(path, false)?
        .ok_or_else(|| failure(format!("{} is missing", path.display())))?;
    let declarations: BTreeMap<_, _> = layer
        .marketplaces
        .iter()
        .map(|(name, value)| (name, &value.value))
        .collect();
    let definitions: BTreeMap<_, _> = layer
        .mcp
        .iter()
        .filter_map(|(name, server)| {
            let bindings: BTreeMap<_, _> = ayran_core::doctor::HARNESSES
                .into_iter()
                .filter_map(|h| {
                    if let Some(ayran_core::mcp::McpBinding::Definition(d)) =
                        server.value.binding(h)
                    {
                        Some((h.binary(), d.native_value(h)))
                    } else {
                        None
                    }
                })
                .collect();
            (!bindings.is_empty()).then_some((name, bindings))
        })
        .collect();
    let mut git = BTreeMap::new();
    let mut paths = BTreeMap::new();
    for (name, skill) in &layer.skills {
        let mut git_bindings = BTreeMap::new();
        let mut bindings = BTreeMap::new();
        for h in ayran_core::doctor::HARNESSES {
            if let Some(ayran_core::config::SkillBinding::Git(g)) = skill.value.binding(h) {
                git_bindings.insert(h.binary(), g);
            }
            if let Some(ayran_core::config::SkillBinding::Path(p)) = skill.value.binding(h) {
                bindings.insert(h.binary(), p.canonicalize().map_err(failure)?);
            }
        }
        if !git_bindings.is_empty() {
            git.insert(name, git_bindings);
        }
        if !bindings.is_empty() {
            paths.insert(name, bindings);
        }
    }
    // Keep existing Marketplace-only approvals valid until a new source category is declared.
    let bytes = if !git.is_empty() {
        serde_json::to_vec(&(declarations, definitions, paths, git))
    } else if !paths.is_empty() {
        serde_json::to_vec(&(declarations, definitions, paths))
    } else if definitions.is_empty() {
        serde_json::to_vec(&declarations)
    } else {
        serde_json::to_vec(&(declarations, definitions))
    }
    .map_err(failure)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
pub fn command() -> Command {
    Command::new("trust")
        .about("Record Trust for a project layer's install sources")
        .arg(
            Arg::new("path")
                .value_parser(clap::builder::PathBufValueParser::new())
                .conflicts_with("list"),
        )
        .arg(
            Arg::new("list")
                .long("list")
                .action(ArgAction::SetTrue)
                .conflicts_with("revoke"),
        )
        .arg(Arg::new("revoke").long("revoke").action(ArgAction::SetTrue))
}
fn layer_path(matches: &ArgMatches) -> Result<PathBuf, Diagnostic> {
    let path = if let Some(path) = matches.get_one::<PathBuf>("path") {
        path.clone()
    } else {
        let user = ayran_core::config::user_config_path()?;
        ayran_core::config::directory_config_paths(&user)?
            .into_iter()
            .rev()
            .find(|p| p.file_name().is_some_and(|n| n == "ayran.toml") && p.is_file())
            .ok_or_else(|| failure("no project ayran.toml found; specify a layer path"))?
    };
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir().map_err(failure)?.join(path)
    };
    match path.canonicalize() {
        Ok(_) => Ok(path),
        Err(e) if matches.get_flag("revoke") && e.kind() == std::io::ErrorKind::NotFound => {
            let absolute = if path.is_absolute() {
                path
            } else {
                std::env::current_dir().map_err(failure)?.join(path)
            };
            let parent = absolute
                .parent()
                .ok_or_else(|| failure("layer path has no parent"))?
                .canonicalize()
                .map_err(failure)?;
            Ok(parent.join(
                absolute
                    .file_name()
                    .ok_or_else(|| failure("layer path has no file name"))?,
            ))
        }
        Err(e) => Err(failure(e)),
    }
}
pub fn run(matches: &ArgMatches) -> i32 {
    let result = (|| {
        let mut store = Store::read()?;
        if matches.get_flag("list") {
            crate::list_command::print_row(vec![
                "layer".into(),
                "canonical".into(),
                "trusted".into(),
            ]);
            for (canonical, approval) in &store.0 {
                // A removed or now invalid layer no longer has matching Trust.
                crate::list_command::print_row(vec![
                    approval.layer.display().to_string(),
                    canonical.display().to_string(),
                    store.trusted(&approval.layer).unwrap_or(false).to_string(),
                ]);
            }
        } else {
            let path = layer_path(matches)?;
            if matches.get_flag("revoke") {
                store
                    .0
                    .remove(&path.canonicalize().unwrap_or_else(|_| path.clone()));
                store.write()?;
                println!("Trust revoked: {}", path.display());
            } else {
                store.0.insert(
                    path.canonicalize().map_err(failure)?,
                    Approval {
                        hash: fingerprint(&path)?,
                        layer: path.clone(),
                    },
                );
                store.write()?;
                println!("Trusted: {}", path.display());
            }
        }
        Ok::<_, Diagnostic>(())
    })();
    match result {
        Ok(()) => 0,
        Err(d) => {
            crate::render(&d, false);
            3
        }
    }
}
