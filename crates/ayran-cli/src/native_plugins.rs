//! Read-only native Plugin state shared by list, completion and doctor.
use ayran_core::{
    config::{ConfigLayers, PluginBinding},
    diagnostic::Diagnostic,
    doctor::{HARNESSES, reachable_plugins},
    harness::Harness,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NativeState {
    On,
    Off,
    Unknown,
}
impl NativeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
            Self::Unknown => "unknown",
        }
    }
    pub fn from_enabled(on: bool) -> Self {
        if on { Self::On } else { Self::Off }
    }
}
use NativeState::{Off, On, Unknown};
type States = BTreeMap<String, (NativeState, String)>;

#[derive(Clone)]
pub struct Plugin {
    pub harness: Harness,
    pub id: String,
    pub state: NativeState,
    pub layer: String,
    pub logical: Vec<String>,
    pub default: bool,
    pub reachable: bool,
}
pub fn installed() -> Vec<Harness> {
    HARNESSES
        .into_iter()
        .filter(|h| crate::program_on_path(std::ffi::OsStr::new(h.binary())).is_some())
        .collect()
}
pub fn home(
    layers: &ConfigLayers,
    harness: Harness,
) -> Result<crate::harness_home::HarnessHome, Diagnostic> {
    let real_home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    crate::harness_home::HarnessHome::resolve(
        harness,
        layers.settings(harness).home_mode,
        real_home.as_deref(),
    )
}
pub fn read(layers: &ConfigLayers, harnesses: &[Harness]) -> Result<Vec<Plugin>, Diagnostic> {
    let real_home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let mut result = Vec::new();
    let installed = installed();
    for &h in harnesses {
        let home = home(layers, h)?;
        let mut inventory = crate::plugin_enumeration::read(h, real_home.as_deref(), &home)?;
        let args: Vec<_> = layers
            .settings(h)
            .args
            .as_ref()
            .map(|args| args.value.iter().map(std::ffi::OsString::from).collect())
            .unwrap_or_default();
        let profiles = crate::codex_config::profiles(&args);
        if h == Harness::Codex {
            for name in &profiles {
                crate::plugin_enumeration::read_codex_layer(
                    &home.directory,
                    &home.directory.join(format!("{name}.config.toml")),
                    &mut inventory,
                )?;
            }
        }
        if h == Harness::Copilot {
            inventory.user.extend(
                inventory
                    .direct
                    .iter()
                    .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_owned)),
            );
        }
        let mut states = BTreeMap::new();
        match h {
            Harness::Codex => {
                for path in std::iter::once(home.directory.join("config.toml")).chain(
                    profiles
                        .iter()
                        .map(|n| home.directory.join(format!("{n}.config.toml"))),
                ) {
                    codex_values(&path, profiles.len() > 1, &mut states)?;
                }
                // Project trust is unknown; preserve uncertainty as in launch proofs.
                let cwd = std::env::current_dir().map_err(|e| failure(Path::new("."), e))?;
                for dir in cwd.ancestors() {
                    let path = dir.join(".codex/config.toml");
                    if !crate::skill_enumeration::same_root(
                        &path,
                        &home.directory.join("config.toml"),
                    ) && real_home.as_ref().is_none_or(|r| {
                        !crate::skill_enumeration::same_root(&path, &r.join(".codex/config.toml"))
                    }) {
                        codex_values(&path, true, &mut states)?;
                    }
                }
            }
            Harness::Claude => {
                let cwd = std::env::current_dir().map_err(|e| failure(Path::new("."), e))?;
                for path in [
                    home.directory.join("settings.json"),
                    cwd.join(".claude/settings.json"),
                    cwd.join(".claude/settings.local.json"),
                ] {
                    claude_values(&path, &mut states)?;
                }
                for id in inventory.user.union(&inventory.project) {
                    if !states.contains_key(id) {
                        states.insert(id.clone(), claude_user_state(&home.directory, &cwd, id)?);
                    }
                }
                let uncertain = args.iter().any(|a| {
                    a.to_str()
                        .is_some_and(|a| a == "--settings" || a.starts_with("--settings="))
                }) || cwd.ancestors().any(|dir| {
                    dir.join(".git").is_file()
                        || (dir != cwd && dir.join(".claude/settings.local.json").exists())
                });
                if uncertain {
                    for value in states.values_mut() {
                        *value = (
                            Unknown,
                            "Harness settings or local settings path uncertain".into(),
                        );
                    }
                }
                // File policy is known; cached remote policy may be stale or ineligible.
                let mut managed = States::new();
                for path in claude_managed_paths()? {
                    claude_values(&path, &mut managed)?;
                }
                let remote = home.directory.join("remote-settings.json");
                if json(&remote, h)?.is_some_and(|v| {
                    v.as_object().is_some_and(|map| {
                        map.iter().any(|(key, value)| {
                            !value.is_null()
                                && !matches!(
                                    key.as_str(),
                                    "managedSourcesBehavior" | "wslInheritsWindowsSettings"
                                )
                        })
                    })
                }) {
                    claude_values(&remote, &mut managed)?;
                    for value in managed.values_mut() {
                        *value = (Unknown, remote.display().to_string());
                    }
                }
                states.extend(managed);
            }
            Harness::Copilot => {
                let config = home.directory.join("config.json");
                if let Some(value) = json(&config, h)?
                    && let Some(plugins) = value.get("installedPlugins").and_then(|v| v.as_array())
                {
                    for plugin in plugins {
                        if let Some(name) = plugin.get("name").and_then(|v| v.as_str()) {
                            let market = plugin
                                .get("marketplace")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let id = if market.is_empty() {
                                name.into()
                            } else {
                                format!("{name}@{market}")
                            };
                            states.insert(
                                id,
                                (
                                    plugin
                                        .get("enabled")
                                        .and_then(|v| v.as_bool())
                                        .map_or(On, NativeState::from_enabled),
                                    config.display().to_string(),
                                ),
                            );
                        }
                    }
                }
                let settings = home.directory.join("settings.json");
                if let Some(value) = json(&settings, h)?
                    && let Some(enabled) = value.get("enabledPlugins")
                {
                    let map = enabled
                        .as_object()
                        .ok_or_else(|| failure(&settings, "enabledPlugins must be an object"))?;
                    for id in inventory.user.union(&inventory.project) {
                        states.insert(
                            id.clone(),
                            (
                                map.get(id).map_or(Off, |v| {
                                    v.as_bool().map_or(Unknown, NativeState::from_enabled)
                                }),
                                settings.display().to_string(),
                            ),
                        );
                    }
                }
            }
        }
        let reachable = reachable_plugins(layers, &installed, h);
        for id in inventory.user.union(&inventory.project) {
            let logical: Vec<_> = layers.plugins.iter().filter(|(_, p)| matches!(p.value.binding(h), Some(PluginBinding::Native(binding)) if binding == id)).map(|(n, _)| n.clone()).collect();
            let (state, layer) = states.get(id).cloned().unwrap_or_else(|| {
                (
                    if h == Harness::Copilot { On } else { Unknown },
                    "native default".into(),
                )
            });
            result.push(Plugin {
                harness: h,
                id: id.clone(),
                state,
                layer,
                default: logical.iter().any(|n| layers.plugins[n].value.default),
                reachable: logical.iter().any(|n| reachable.contains(n)),
                logical,
            });
        }
    }
    Ok(result)
}
fn failure(path: &Path, error: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error(
        "enumeration-failed",
        format!(
            "cannot read native Plugin state from {}: {error}",
            path.display()
        ),
        None,
    )
}
fn json(path: &Path, h: Harness) -> Result<Option<serde_json::Value>, Diagnostic> {
    if h == Harness::Copilot {
        crate::plugin_enumeration::copilot_json(path)
    } else {
        crate::mcp_enumeration::read_json_object(path, h)
    }
}
fn codex_values(path: &Path, uncertain: bool, states: &mut States) -> Result<(), Diagnostic> {
    let config = crate::codex_config::read_table(path)?;
    if let Some(plugins) = config.get("plugins") {
        let plugins = plugins
            .as_table()
            .ok_or_else(|| failure(path, "plugins must be a table"))?;
        for (id, value) in plugins {
            if let Some(enabled) = value.get("enabled") {
                let on = enabled
                    .as_bool()
                    .ok_or_else(|| failure(path, "Plugin enabled must be a boolean"))?;
                states.insert(
                    id.clone(),
                    (
                        if uncertain {
                            Unknown
                        } else {
                            NativeState::from_enabled(on)
                        },
                        path.display().to_string(),
                    ),
                );
            }
        }
    }
    Ok(())
}
pub fn codex_user_state(home: &Path, id: &str) -> Result<NativeState, Diagnostic> {
    let mut states = States::new();
    codex_values(&home.join("config.toml"), false, &mut states)?;
    Ok(states.get(id).map_or(Unknown, |(state, _)| *state))
}
pub fn claude_user_state(
    home: &Path,
    cwd: &Path,
    id: &str,
) -> Result<(NativeState, String), Diagnostic> {
    let path = home.join("settings.json");
    let mut states = States::new();
    claude_values(&path, &mut states)?;
    if let Some(state) = states.remove(id) {
        return Ok(state);
    }
    if id.ends_with("@skills-dir") {
        claude_default(home, cwd, id)
    } else {
        Ok((Off, "marketplace default".into()))
    }
}
fn claude_values(path: &Path, states: &mut States) -> Result<(), Diagnostic> {
    if let Some(value) = json(path, Harness::Claude)?
        && let Some(enabled) = value.get("enabledPlugins")
    {
        let map = enabled
            .as_object()
            .ok_or_else(|| failure(path, "enabledPlugins must be an object"))?;
        for (id, enabled) in map {
            states.insert(
                id.clone(),
                (
                    enabled.as_bool().map_or(Unknown, NativeState::from_enabled),
                    path.display().to_string(),
                ),
            );
        }
    }
    Ok(())
}
fn claude_managed_paths() -> Result<Vec<PathBuf>, Diagnostic> {
    #[cfg(target_os = "macos")]
    let root = Path::new("/Library/Application Support/ClaudeCode");
    #[cfg(windows)]
    let root = Path::new(r"C:\Program Files\ClaudeCode");
    #[cfg(not(any(target_os = "macos", windows)))]
    let root = Path::new("/etc/claude-code");
    let dir = root.join("managed-settings.d");
    let mut paths = Vec::new();
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            for entry in entries {
                let path = entry.map_err(|e| failure(&dir, e))?.path();
                if path.extension().is_some_and(|e| e == "json")
                    && path
                        .file_name()
                        .is_some_and(|n| !n.to_string_lossy().starts_with('.'))
                {
                    paths.push(path);
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(failure(&dir, e)),
    }
    paths.sort();
    paths.insert(0, root.join("managed-settings.json"));
    Ok(paths)
}
fn claude_default(home: &Path, cwd: &Path, id: &str) -> Result<(NativeState, String), Diagnostic> {
    for dir in std::iter::once(home.join("skills"))
        .chain(cwd.ancestors().map(|p| p.join(".claude/skills")))
    {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(failure(&dir, e)),
        };
        for entry in entries {
            let entry = entry.map_err(|e| failure(&dir, e))?;
            let path = entry.path().join(".claude-plugin/plugin.json");
            if let Some(value) = json(&path, Harness::Claude)?
                && value
                    .get("name")
                    .and_then(|v| v.as_str())
                    .is_some_and(|n| format!("{n}@skills-dir") == id)
            {
                return Ok((
                    value
                        .get("defaultEnabled")
                        .and_then(|v| v.as_bool())
                        .map_or(On, NativeState::from_enabled),
                    path.display().to_string(),
                ));
            }
        }
        if dir
            .parent()
            .and_then(Path::parent)
            .is_some_and(|p| p.join(".git").exists())
        {
            break;
        }
    }
    Ok((Unknown, "skills-dir manifest".into()))
}
