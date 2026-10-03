use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use ayran_core::diagnostic::Diagnostic;
use ayran_core::enumerate::InstalledPlugins;
use ayran_core::harness::Harness;

pub fn read(
    harness: Harness,
    real_home: Option<&Path>,
    home: &crate::harness_home::HarnessHome,
) -> Result<InstalledPlugins, Diagnostic> {
    match harness {
        Harness::Codex => return read_codex(home),
        Harness::Copilot => return read_copilot(home),
        Harness::Claude => {}
    }
    let home = home.directory.clone();
    let mut installed = InstalledPlugins::default();
    let personal = home.join("skills");
    read_claude_skills_dir_plugins(&personal, &mut installed.user)?;
    let cwd = env::current_dir()
        .map_err(|error| Diagnostic::error("enumeration-failed", error.to_string(), None))?;
    for directory in cwd.ancestors() {
        let root = directory.join(".claude/skills");
        // HOME stays user-level, including inside a dotfiles repository.
        if !crate::skill_enumeration::same_root(&root, &personal)
            && real_home.is_none_or(|home| {
                !crate::skill_enumeration::same_root(&root, &home.join(".claude/skills"))
            })
        {
            read_claude_skills_dir_plugins(&root, &mut installed.project)?;
        }
        if crate::skill_enumeration::exists(&directory.join(".git"))? {
            break;
        }
    }
    let path = home.join("plugins/installed_plugins.json");
    let failure = |message: String| {
        Diagnostic::error(
            "enumeration-failed",
            format!(
                "cannot enumerate Claude Plugins from {}: {message}",
                path.display()
            ),
            None,
        )
    };
    let contents = match fs::read(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(installed);
        }
        Err(error) => return Err(failure(error.to_string())),
    };
    let value: serde_json::Value =
        serde_json::from_slice(&contents).map_err(|error| failure(error.to_string()))?;
    if value.get("version").and_then(serde_json::Value::as_u64) != Some(2) {
        return Err(failure(
            "expected installed Plugins registry version 2".into(),
        ));
    }
    let plugins = value
        .get("plugins")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| failure("plugins must be an object".into()))?;
    for (id, entries) in plugins {
        let entries = entries
            .as_array()
            .ok_or_else(|| failure(format!("Plugin {id} installs must be an array")))?;
        for entry in entries {
            match entry.get("scope").and_then(serde_json::Value::as_str) {
                Some("user") => {
                    installed.user.insert(id.clone());
                }
                Some("project" | "local") => {
                    let project = entry
                        .get("projectPath")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| failure(format!("Plugin {id} has no projectPath")))?;
                    if cwd.starts_with(project) {
                        installed.project.insert(id.clone());
                    }
                }
                _ => return Err(failure(format!("Plugin {id} has an invalid install scope"))),
            }
        }
    }
    Ok(installed)
}

fn read_claude_skills_dir_plugins(
    root: &Path,
    plugins: &mut std::collections::BTreeSet<String>,
) -> Result<(), Diagnostic> {
    let failure = |path: &Path, message: String| {
        Diagnostic::error(
            "enumeration-failed",
            format!(
                "cannot enumerate Claude Plugins from {}: {message}",
                path.display()
            ),
            None,
        )
    };
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(failure(root, error.to_string())),
    };
    for entry in entries {
        let entry = entry.map_err(|error| failure(root, error.to_string()))?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with('.') || name.eq_ignore_ascii_case("synced"))
        {
            continue;
        }
        let path = entry.path();
        if !fs::metadata(&path)
            .map_err(|error| failure(&path, error.to_string()))?
            .is_dir()
        {
            continue;
        }
        let manifest = path.join(".claude-plugin/plugin.json");
        let contents = match fs::read(&manifest) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(failure(&manifest, error.to_string())),
        };
        let value: serde_json::Value = serde_json::from_slice(&contents)
            .map_err(|error| failure(&manifest, error.to_string()))?;
        let name = value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .filter(|name| {
                !name.is_empty()
                    && !name.chars().any(|character| {
                        character.is_whitespace()
                            || character.is_control()
                            || matches!(character, '@' | ':' | '/' | '\\'
                                | '\u{061c}' | '\u{200e}' | '\u{200f}'
                                | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                    })
            })
            .ok_or_else(|| {
                failure(
                    &manifest,
                    "Plugin name must be a valid nonempty identifier".into(),
                )
            })?;
        plugins.insert(format!("{name}@skills-dir"));
    }
    Ok(())
}

/// Native Copilot Bindings use the shared home even in an isolated Session.
/// Inspect only declared Bindings, so unrelated shared installs cannot block launch.
pub fn copilot_native_paths<'a>(
    home: &Path,
    bindings: impl Iterator<Item = &'a str>,
) -> Result<std::collections::BTreeMap<String, PathBuf>, Diagnostic> {
    let bindings: std::collections::BTreeSet<_> = bindings.collect();
    let mut paths = std::collections::BTreeMap::new();
    for id in bindings
        .iter()
        .copied()
        .filter(|id| ayran_core::resolve::valid_copilot_native_id(id))
    {
        let (name, market) = id.split_once('@').unwrap();
        let path = home.join("installed-plugins").join(market).join(name);
        if path.is_dir() {
            paths.insert(id.to_owned(), path);
        }
    }
    paths.extend(copilot_registry_paths(home, Some(&bindings))?);
    Ok(paths)
}

fn copilot_enumeration_failure(path: &Path, message: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error(
        "enumeration-failed",
        format!(
            "cannot enumerate Copilot Plugins from {}: {message}",
            path.display()
        ),
        None,
    )
}

pub(crate) fn copilot_json(path: &Path) -> Result<Option<serde_json::Value>, Diagnostic> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(copilot_enumeration_failure(path, error)),
    };
    let value: serde_json::Value = serde_json_lenient::from_str(&contents)
        .map_err(|error| copilot_enumeration_failure(path, error))?;
    if !value.is_object() {
        return Err(copilot_enumeration_failure(path, "expected an object"));
    }
    Ok(Some(value))
}

/// Include live roots which never appear in installed-plugins. Reading only
/// requested IDs keeps isolated Sessions independent of unrelated shared roots.
fn copilot_registry_paths(
    home: &Path,
    bindings: Option<&std::collections::BTreeSet<&str>>,
) -> Result<std::collections::BTreeMap<String, PathBuf>, Diagnostic> {
    let mut paths = std::collections::BTreeMap::new();
    if bindings.is_some_and(|bindings| bindings.is_empty()) {
        return Ok(paths);
    }
    let wanted = |id: &str| bindings.is_none_or(|bindings| bindings.contains(id));
    let config = home.join("config.json");
    if let Some(value) = copilot_json(&config)?
        && let Some(plugins) = value.get("installedPlugins")
    {
        let plugins = plugins.as_array().ok_or_else(|| {
            copilot_enumeration_failure(&config, "installedPlugins must be an array")
        })?;
        for plugin in plugins {
            if let Some(bindings) = bindings {
                let id = plugin
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .zip(
                        plugin
                            .get("marketplace")
                            .and_then(serde_json::Value::as_str),
                    )
                    .map(|(name, market)| format!("{name}@{market}"));
                if id.as_ref().is_none_or(|id| !bindings.contains(id.as_str())) {
                    continue;
                }
            }
            let name = plugin
                .get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    copilot_enumeration_failure(&config, "installed Plugin needs a name")
                })?;
            let market = match plugin.get("marketplace") {
                None | Some(serde_json::Value::Null) => "",
                Some(value) => value.as_str().ok_or_else(|| {
                    copilot_enumeration_failure(&config, "Plugin marketplace must be a string")
                })?,
            };
            // Direct installs have no marketplace and cannot be native Bindings.
            if market.is_empty() {
                continue;
            }
            let id = format!("{name}@{market}");
            if !wanted(&id) {
                continue;
            }
            if !ayran_core::resolve::valid_copilot_native_id(&id) {
                return Err(copilot_enumeration_failure(
                    &config,
                    format!("invalid Plugin ID {id}"),
                ));
            }
            match plugin.get("cache_path") {
                // Older copied installs may omit their default installed root.
                None | Some(serde_json::Value::Null) => {}
                Some(value) => {
                    let path = value
                        .as_str()
                        .filter(|path| !path.is_empty())
                        .ok_or_else(|| {
                            copilot_enumeration_failure(
                                &config,
                                "Plugin cache_path must be a nonempty string",
                            )
                        })?;
                    paths.insert(id, PathBuf::from(path));
                }
            }
        }
    }

    let settings = home.join("settings.json");
    if let Some(value) = copilot_json(&settings)?
        && let Some(enabled) = value.get("enabledPlugins")
    {
        let enabled = enabled.as_object().ok_or_else(|| {
            copilot_enumeration_failure(&settings, "enabledPlugins must be an object")
        })?;
        if value
            .get("extraKnownMarketplaces")
            .is_some_and(|markets| !markets.is_object())
        {
            return Err(copilot_enumeration_failure(
                &settings,
                "extraKnownMarketplaces must be an object",
            ));
        }
        for (id, enabled) in enabled.iter().filter(|(id, _)| wanted(id)) {
            if !enabled.is_boolean() {
                return Err(copilot_enumeration_failure(
                    &settings,
                    "enabledPlugins entries must be booleans",
                ));
            }
            if !ayran_core::resolve::valid_copilot_native_id(id) {
                continue;
            }
            let (name, market) = id.split_once('@').unwrap();
            let Some(source) = value
                .get("extraKnownMarketplaces")
                .and_then(|markets| markets.get(market))
                .and_then(|market| market.get("source"))
            else {
                continue;
            };
            if source.get("source").and_then(serde_json::Value::as_str) != Some("directory") {
                continue;
            }
            let root = source
                .get("path")
                .and_then(serde_json::Value::as_str)
                .filter(|path| !path.is_empty())
                .ok_or_else(|| {
                    copilot_enumeration_failure(&settings, "directory marketplace needs a path")
                })?;
            let root = Path::new(root);
            let mut catalog = None;
            for relative in [
                "marketplace.json",
                ".plugin/marketplace.json",
                ".github/plugin/marketplace.json",
                ".claude-plugin/marketplace.json",
            ] {
                let path = root.join(relative);
                if let Some(value) = copilot_json(&path)? {
                    catalog = Some((path, value));
                    break;
                }
            }
            let Some((path, catalog)) = catalog else {
                return Err(copilot_enumeration_failure(
                    root,
                    "marketplace manifest is missing",
                ));
            };
            let plugins = catalog
                .get("plugins")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| copilot_enumeration_failure(&path, "plugins must be an array"))?;
            if let Some(plugin) = plugins
                .iter()
                .find(|plugin| plugin.get("name").and_then(serde_json::Value::as_str) == Some(name))
            {
                let source = plugin.get("source").ok_or_else(|| {
                    copilot_enumeration_failure(&path, "Plugin source is missing")
                })?;
                if source.is_object() {
                    if matches!(
                        source.get("source").and_then(serde_json::Value::as_str),
                        Some("github" | "url")
                    ) {
                        continue;
                    }
                    return Err(copilot_enumeration_failure(
                        &path,
                        "invalid remote Plugin source",
                    ));
                }
                let source = source
                    .as_str()
                    .filter(|source| !source.is_empty())
                    .ok_or_else(|| {
                        copilot_enumeration_failure(
                            &path,
                            "Plugin source must be a path string or remote source object",
                        )
                    })?;
                let plugin_root = catalog
                    .get("metadata")
                    .and_then(|metadata| metadata.get("pluginRoot"));
                let plugin_root = match plugin_root {
                    None => "",
                    Some(value) => value.as_str().ok_or_else(|| {
                        copilot_enumeration_failure(&path, "pluginRoot must be a string")
                    })?,
                };
                paths.insert(id.clone(), root.join(plugin_root).join(source));
            }
        }
    }
    let mut found = std::collections::BTreeMap::new();
    for (id, path) in paths {
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => {
                fs::read_dir(&path).map_err(|error| copilot_enumeration_failure(&path, error))?;
                found.insert(id, path);
            }
            Ok(_) => {
                return Err(copilot_enumeration_failure(
                    &path,
                    "Plugin root must be a directory",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(copilot_enumeration_failure(&path, error)),
        }
    }
    Ok(found)
}

fn read_copilot(home: &crate::harness_home::HarnessHome) -> Result<InstalledPlugins, Diagnostic> {
    let home = home.directory.clone();
    let root = home.join("installed-plugins");
    let failure = |message: String| {
        Diagnostic::error(
            "enumeration-failed",
            format!(
                "cannot enumerate Copilot Plugins from {}: {message}",
                root.display()
            ),
            None,
        )
    };
    let mut installed = InstalledPlugins {
        paths: copilot_registry_paths(&home, None)?,
        ..InstalledPlugins::default()
    };
    installed.user.extend(installed.paths.keys().cloned());
    let marketplaces = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(installed);
        }
        Err(error) => return Err(failure(error.to_string())),
    };
    for marketplace in marketplaces {
        let marketplace = marketplace.map_err(|error| failure(error.to_string()))?;
        if !marketplace
            .metadata()
            .map_err(|error| failure(error.to_string()))?
            .is_dir()
        {
            continue;
        }
        let market = marketplace
            .file_name()
            .into_string()
            .map_err(|_| failure("marketplace name is not UTF-8".into()))?;
        let entries = fs::read_dir(marketplace.path())
            .map_err(|error| failure(format!("{}: {error}", marketplace.path().display())))?;
        for entry in entries {
            let entry = entry.map_err(|error| failure(error.to_string()))?;
            if !entry
                .metadata()
                .map_err(|error| failure(error.to_string()))?
                .is_dir()
            {
                continue;
            }
            // Refuse unreadable Plugin directories instead of silently hiding an
            // incomplete enumeration. No contents are modified or executed.
            fs::read_dir(entry.path())
                .map_err(|error| failure(format!("{}: {error}", entry.path().display())))?;
            if market == "_direct" {
                installed.direct.insert(entry.path());
            } else {
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| failure("Plugin name is not UTF-8".into()))?;
                let id = format!("{name}@{market}");
                installed.user.insert(id.clone());
                installed.paths.entry(id).or_insert_with(|| entry.path());
            }
        }
    }
    Ok(installed)
}

fn read_codex(home: &crate::harness_home::HarnessHome) -> Result<InstalledPlugins, Diagnostic> {
    let mut installed = InstalledPlugins::default();
    read_codex_layer(
        &home.directory,
        &home.directory.join("config.toml"),
        &mut installed,
    )?;
    Ok(installed)
}

pub(crate) fn read_codex_layer(
    home: &Path,
    path: &Path,
    installed: &mut InstalledPlugins,
) -> Result<(), Diagnostic> {
    let value = crate::codex_config::read_table(path)?;
    let failure = |message: String| {
        Diagnostic::error(
            "enumeration-failed",
            format!(
                "cannot enumerate Codex Plugins from {}: {message}",
                path.display()
            ),
            None,
        )
    };
    if let Some(plugins) = value.get("plugins") {
        let plugins = plugins
            .as_table()
            .ok_or_else(|| failure("plugins must be a table".into()))?;
        for (id, plugin) in plugins {
            let plugin = plugin
                .as_table()
                .ok_or_else(|| failure(format!("Plugin {id} must be a table")))?;
            if plugin
                .get("enabled")
                .is_some_and(|enabled| !enabled.is_bool())
            {
                return Err(failure(format!("Plugin {id} enabled must be a boolean")));
            }
            let (name, marketplace) = id
                .split_once('@')
                .filter(|(name, marketplace)| {
                    [name, marketplace].iter().all(|part| {
                        !part.is_empty()
                            && !matches!(**part, "." | "..")
                            && !part.contains(['/', '\\', ':', '@'])
                    })
                })
                .ok_or_else(|| failure(format!("Plugin {id} must use name@marketplace")))?;
            let cache = home.join("plugins/cache").join(marketplace).join(name);
            if active_codex_plugin_root(&cache)?.is_some() {
                installed.user.insert(id.clone());
            }
        }
    }
    Ok(())
}

/// Match Codex's installed-version selection without opening Plugin payloads.
pub(crate) fn active_codex_plugin_root(cache: &Path) -> Result<Option<PathBuf>, Diagnostic> {
    let failure = |error: std::io::Error| {
        Diagnostic::error(
            "enumeration-failed",
            format!(
                "cannot enumerate Codex Plugin versions from {}: {error}",
                cache.display()
            ),
            None,
        )
    };
    let entries = match fs::read_dir(cache) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failure(error)),
    };
    let mut versions = Vec::new();
    for entry in entries {
        let entry = entry.map_err(failure)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.is_empty()
            && !matches!(name, "." | "..")
            && name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '+')
            })
            && entry.file_type().map_err(failure)?.is_dir()
        {
            versions.push(name.to_owned());
        }
    }
    versions.sort_unstable_by(|left, right| {
        match (semver::Version::parse(left), semver::Version::parse(right)) {
            (Ok(left), Ok(right)) => left.cmp(&right),
            _ => left.cmp(right),
        }
    });
    let version = if versions.iter().any(|name| name == "local") {
        Some("local".to_owned())
    } else {
        versions.pop()
    };
    Ok(version.map(|version| cache.join(version)))
}
