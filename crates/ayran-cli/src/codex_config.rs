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

pub(crate) fn read_table(path: &Path) -> Result<toml::Table, Diagnostic> {
    match fs::read_to_string(path) {
        Ok(contents) => contents.parse().map_err(|error| failure(path, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(error) => Err(failure(path, error)),
    }
}

/// Extract Codex's file-profile arguments without interpreting other Harness flags.
pub(crate) fn profiles(args: &[std::ffi::OsString]) -> Vec<String> {
    let mut profiles = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let Some(arg) = arg.to_str() else { continue };
        if arg == "--" {
            break;
        }
        if matches!(arg, "-c" | "--config") {
            args.next();
        } else if matches!(arg, "-p" | "--profile") {
            if let Some(name) = args.next().and_then(|name| name.to_str()) {
                profiles.push(name.to_owned());
            }
        } else if let Some(name) = arg
            .strip_prefix("--profile=")
            .or_else(|| arg.strip_prefix("-p").filter(|name| !name.is_empty()))
        {
            profiles.push(name.to_owned());
        }
    }
    profiles
}

/// Enumerate profile items and prove native state from readable layers only.
pub(crate) fn apply(
    home: &crate::harness_home::HarnessHome,
    real_home: Option<&Path>,
    profiles: &[String],
    plugins: &mut ayran_core::enumerate::InstalledPlugins,
    skills: &mut ayran_core::skills::SkillState,
    mcp: &mut ayran_core::mcp::McpState,
) -> Result<(), Diagnostic> {
    let mut plugin_enabled = std::collections::BTreeMap::new();
    let mut server_enabled = std::collections::BTreeMap::new();
    let mut rules = Vec::new();
    for path in std::iter::once(home.directory.join("config.toml")).chain(
        profiles
            .iter()
            .map(|name| home.directory.join(format!("{name}.config.toml"))),
    ) {
        let config = read_table(&path)?;
        crate::plugin_enumeration::read_codex_layer(&home.directory, &path, plugins)?;
        crate::mcp_enumeration::read_codex_config(
            &path,
            &mut mcp.user,
            crate::mcp_enumeration::CodexScope::User(&mut mcp.enabled_apps),
        )?;
        enabled_values(&config, "plugins", &path, &mut plugin_enabled)?;
        enabled_values(&config, "mcp_servers", &path, &mut server_enabled)?;
        if let Some(servers) = config.get("mcp_servers").and_then(toml::Value::as_table) {
            for (name, server) in servers {
                if path != home.directory.join("config.toml")
                    && server
                        .as_table()
                        .is_some_and(|t| t.keys().any(|key| key != "enabled"))
                {
                    // A profile transport overrides the user definition; do not adopt the user copy.
                    mcp.user_definitions.remove(name);
                }
                mcp.sources.insert((
                    name.clone(),
                    if path == home.directory.join("config.toml") {
                        "user"
                    } else {
                        "profile"
                    },
                ));
                if server.get("enabled").is_some() {
                    mcp.native_layers
                        .insert(name.clone(), path.display().to_string());
                }
            }
        }
        read_skill_rules(&config, &path, &mut rules, skills)?;
    }
    // Trust is deliberately ignored: any project value makes the proof uncertain.
    let cwd = std::env::current_dir().map_err(|error| failure(Path::new("."), error))?;
    let user = home.directory.join("config.toml");
    // Profiles and session flags can change root markers. Scan every ancestor for
    // proof; normal item discovery retains its own project boundary.
    for directory in cwd.ancestors() {
        let path = directory.join(".codex/config.toml");
        if crate::skill_enumeration::same_root(&path, &user)
            || real_home.is_some_and(|home| {
                crate::skill_enumeration::same_root(&path, &home.join(".codex/config.toml"))
            })
        {
            continue;
        }
        let config = read_table(&path)?;
        let mut project_plugins = std::collections::BTreeMap::new();
        let mut project_servers = std::collections::BTreeMap::new();
        enabled_values(&config, "plugins", &path, &mut project_plugins)?;
        enabled_values(&config, "mcp_servers", &path, &mut project_servers)?;
        plugin_enabled.retain(|id, _| !project_plugins.contains_key(id));
        server_enabled.retain(|id, _| !project_servers.contains_key(id));
        for id in project_servers.keys() {
            mcp.native_layers.insert(
                id.clone(),
                format!("{} (project trust uncertain)", path.display()),
            );
        }
    }
    if profiles.len() <= 1 {
        plugins.codex_enabled = plugin_enabled;
        mcp.codex_enabled = server_enabled;
        skills.codex_skip_redundant = true;
    }
    for (path, skill) in &mut skills.codex {
        for rule in &rules {
            if rule.matches(path, &skill.name) {
                skill.personal = true;
                skills.codex_layers.insert(path.clone(), rule.layer.clone());
                if rule.enabled {
                    skills.codex_off.remove(path);
                } else {
                    skills.codex_off.insert(path.clone());
                }
            }
        }
    }
    Ok(())
}

fn enabled_values(
    config: &toml::Table,
    key: &str,
    path: &Path,
    values: &mut std::collections::BTreeMap<String, bool>,
) -> Result<(), Diagnostic> {
    if let Some(items) = config.get(key) {
        let items = items
            .as_table()
            .ok_or_else(|| failure(path, format!("{key} must be a table")))?;
        for (id, item) in items {
            let item = item
                .as_table()
                .ok_or_else(|| failure(path, format!("{key}.{id} must be a table")))?;
            if let Some(enabled) = item.get("enabled") {
                let enabled = enabled.as_bool().ok_or_else(|| {
                    failure(path, format!("{key}.{id}.enabled must be a boolean"))
                })?;
                values.insert(id.clone(), enabled);
            }
        }
    }
    Ok(())
}

struct SkillRule {
    path: Option<PathBuf>,
    name: Option<String>,
    enabled: bool,
    layer: String,
}

impl SkillRule {
    fn matches(&self, path: &Path, name: &str) -> bool {
        self.path
            .as_ref()
            .is_some_and(|selector| crate::skill_enumeration::same_root(selector, path))
            || self.name.as_deref() == Some(name)
    }
}

fn read_skill_rules(
    config: &toml::Table,
    path: &Path,
    rules: &mut Vec<SkillRule>,
    skills: &mut ayran_core::skills::SkillState,
) -> Result<(), Diagnostic> {
    let Some(config) = config.get("skills").and_then(|skills| skills.get("config")) else {
        return Ok(());
    };
    let config = config
        .as_array()
        .ok_or_else(|| failure(path, "skills.config must be an array"))?;
    for rule in config {
        let string = |key| -> Result<Option<String>, Diagnostic> {
            rule.get(key)
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        failure(path, format!("skills.config {key} must be a string"))
                    })
                })
                .transpose()
        };
        let selector = string("path")?.map(PathBuf::from);
        let name = string("name")?.map(|name| name.trim().to_owned());
        // Codex ignores ambiguous and empty selectors instead of applying both.
        if selector.is_some() && name.is_some() || name.as_deref() == Some("") {
            continue;
        }
        if selector.is_none() && name.is_none() {
            return Err(failure(path, "skills.config rule needs path or name"));
        }
        let enabled = rule
            .get("enabled")
            .and_then(toml::Value::as_bool)
            .ok_or_else(|| failure(path, "skills.config enabled must be a boolean"))?;
        if let Some(selector) = &selector {
            crate::codex_skill_enumeration::read_configured_path(selector, skills)?;
        }
        rules.push(SkillRule {
            path: selector,
            name,
            enabled,
            layer: path.display().to_string(),
        });
    }
    Ok(())
}
