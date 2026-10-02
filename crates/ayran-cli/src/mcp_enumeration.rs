//! Read-only Harness MCP state; activation decisions stay in the pure core.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::Path;

use ayran_core::diagnostic::Diagnostic;
use ayran_core::harness::Harness;
use ayran_core::mcp::McpState;
use serde_json::Value;

pub fn read_codex(
    harness_home: &crate::harness_home::HarnessHome,
    real_home: Option<&Path>,
) -> Result<McpState, Diagnostic> {
    let home = harness_home.directory.clone();
    let mut state = McpState::default();
    let user_config = home.join("config.toml");
    read_codex_config(&user_config, &mut state.user, Some(&mut state.enabled_apps))?;
    let cwd = env::current_dir().map_err(|error| codex_failure(Path::new("."), error))?;
    for directory in crate::codex_config::project_directories(&home, &cwd)? {
        let path = directory.join(".codex/config.toml");
        if !crate::skill_enumeration::same_root(&path, &user_config)
            && real_home.is_none_or(|home| {
                !crate::skill_enumeration::same_root(&path, &home.join(".codex/config.toml"))
            })
        {
            read_codex_config(&path, &mut state.project, None)?;
        }
    }
    Ok(state)
}

/// Doctor-only account cache inventory; launch never checks connector existence.
pub fn read_codex_connectors(
    home: &crate::harness_home::HarnessHome,
) -> Result<BTreeSet<String>, Diagnostic> {
    let directory = home.directory.join("cache/codex_apps_tools");
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(error) => return Err(codex_failure(&directory, error)),
    };
    let mut connectors = BTreeSet::new();
    for entry in entries {
        let entry = entry.map_err(|error| codex_failure(&directory, error))?;
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        if let Some(cache) = read_codex_json(&path)?
            && let Some(tools) = cache.get("tools")
        {
            let tools = tools
                .as_array()
                .ok_or_else(|| codex_failure(&path, "tools must be an array"))?;
            for tool in tools {
                if let Some(id) = tool.get("connector_id").and_then(Value::as_str) {
                    connectors.insert(id.into());
                }
            }
        }
    }
    Ok(connectors)
}

pub fn read_codex_plugins(
    harness_home: &crate::harness_home::HarnessHome,
    plugins: &[String],
    state: &mut McpState,
) -> Result<(), Diagnostic> {
    let home = harness_home.directory.clone();
    for id in plugins {
        let (name, market) = id
            .split_once('@')
            .expect("native Plugin IDs were validated");
        let cache = home.join("plugins/cache").join(market).join(name);
        let root = crate::plugin_enumeration::active_codex_plugin_root(&cache)?
            .ok_or_else(|| codex_failure(&cache, "selected Plugin is no longer installed"))?;
        read_codex_plugin(&root, &mut state.plugins)?;
    }
    Ok(())
}

fn read_codex_plugin(root: &Path, names: &mut BTreeSet<String>) -> Result<(), Diagnostic> {
    let agent_path = root.join("plugin.json");
    if is_regular_file(&agent_path)? == Some(false) {
        return Ok(());
    }
    if let Some(manifest) = read_codex_json(&agent_path)?
        && let Some(schema) = manifest.get("$schema").and_then(Value::as_str)
        && schema.starts_with("https://agent-plugins.org/schemas/")
    {
        if schema != "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json" {
            return Err(codex_failure(
                &agent_path,
                "unsupported Agent Plugins schema",
            ));
        }
        let path = root.join("mcp.json");
        if is_regular_file(&path)? != Some(true) {
            return Ok(());
        }
        let resolved = path
            .canonicalize()
            .map_err(|error| codex_failure(&path, error))?;
        let root = root
            .canonicalize()
            .map_err(|error| codex_failure(root, error))?;
        if !resolved.starts_with(root) {
            return Ok(());
        }
        if let Some(config) = read_codex_json(&path)? {
            read_codex_plugin_servers(&config, &path, names)?;
        }
        return Ok(());
    }
    for relative in [
        ".codex-plugin/plugin.json",
        ".claude-plugin/plugin.json",
        ".cursor-plugin/plugin.json",
    ] {
        let path = root.join(relative);
        let parent = path.parent().expect("manifest has a parent");
        match fs::symlink_metadata(parent) {
            Ok(metadata) if !metadata.is_dir() => return Ok(()),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(codex_failure(parent, error)),
        }
        if is_regular_file(&path)? == Some(false) {
            return Ok(());
        }
        let Some(manifest) = read_codex_json(&path)? else {
            continue;
        };
        let config_path = match manifest.get("mcpServers") {
            Some(value) if value.is_object() => {
                read_codex_plugin_servers(value, &path, names)?;
                return Ok(());
            }
            Some(Value::String(value)) => value
                .strip_prefix("./")
                .filter(|relative| {
                    !relative.is_empty()
                        && !Path::new(relative).is_absolute()
                        && !Path::new(relative)
                            .components()
                            .any(|component| component == std::path::Component::ParentDir)
                })
                .map(|relative| root.join(relative))
                .unwrap_or_else(|| root.join(".mcp.json")),
            _ => root.join(".mcp.json"),
        };
        if let Some(config) = read_codex_json(&config_path)? {
            read_codex_plugin_servers(&config, &config_path, names)?;
        }
        return Ok(());
    }
    Ok(())
}

fn is_regular_file(path: &Path) -> Result<Option<bool>, Diagnostic> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata.file_type().is_file())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(codex_failure(path, error)),
    }
}

fn read_codex_json(path: &Path) -> Result<Option<Value>, Diagnostic> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(codex_failure(path, error)),
    };
    let value: Value =
        serde_json::from_str(&contents).map_err(|error| codex_failure(path, error))?;
    if !value.is_object() {
        return Err(codex_failure(path, "expected an object"));
    }
    Ok(Some(value))
}

fn read_codex_plugin_servers(
    config: &Value,
    path: &Path,
    names: &mut BTreeSet<String>,
) -> Result<(), Diagnostic> {
    let servers = config
        .get("mcpServers")
        .unwrap_or(config)
        .as_object()
        .ok_or_else(|| codex_failure(path, "MCP servers must be an object"))?;
    for (name, server) in servers {
        if !server.is_object() {
            return Err(codex_failure(
                path,
                format!("MCP server {name} must be an object"),
            ));
        }
        names.insert(name.clone());
    }
    Ok(())
}

fn codex_failure(path: &Path, message: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error(
        "enumeration-failed",
        format!(
            "cannot enumerate Codex MCP servers from {}: {message}",
            path.display()
        ),
        None,
    )
}

fn read_codex_config(
    path: &Path,
    names: &mut BTreeSet<String>,
    enabled_apps: Option<&mut BTreeSet<String>>,
) -> Result<(), Diagnostic> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(codex_failure(path, error)),
    };
    let config: toml::Table = contents
        .parse()
        .map_err(|error| codex_failure(path, error))?;
    if let Some(enabled_apps) = enabled_apps
        && let Some(apps) = config.get("apps")
    {
        let apps = apps
            .as_table()
            .ok_or_else(|| codex_failure(path, "apps must be a table"))?;
        for (id, app) in apps {
            let app = app
                .as_table()
                .ok_or_else(|| codex_failure(path, format!("app {id} must be a table")))?;
            if app.get("enabled").is_some_and(|value| !value.is_bool()) {
                return Err(codex_failure(
                    path,
                    format!("app {id} enabled must be a boolean"),
                ));
            }
            if app.get("enabled").and_then(toml::Value::as_bool) == Some(true) {
                enabled_apps.insert(id.clone());
            }
        }
    }
    if let Some(servers) = config.get("mcp_servers") {
        let servers = servers
            .as_table()
            .ok_or_else(|| codex_failure(path, "mcp_servers must be a table"))?;
        for (name, server) in servers {
            let server = server
                .as_table()
                .ok_or_else(|| codex_failure(path, format!("MCP server {name} must be a table")))?;
            if server.get("enabled").is_some_and(|value| !value.is_bool()) {
                return Err(codex_failure(
                    path,
                    format!("MCP server {name} enabled must be a boolean"),
                ));
            }
            names.insert(name.clone());
        }
    }
    Ok(())
}

pub fn read_claude(
    harness_home: &crate::harness_home::HarnessHome,
) -> Result<McpState, Diagnostic> {
    let path = harness_home.claude_mcp.clone();
    let cwd = env::current_dir()
        .map_err(|error| Diagnostic::error("enumeration-failed", error.to_string(), None))?;
    let mut state = McpState::default();
    if let Some(config) = read_json_object(&path, Harness::Claude)? {
        read_json_servers(&config, &path, &mut state.user, Harness::Claude)?;
        if let Some(connectors) = config.get("claudeAiMcpEverConnected") {
            let connectors = connectors
                .as_array()
                .ok_or_else(|| failure(&path, "claudeAiMcpEverConnected must be an array"))?;
            for id in connectors {
                let id = id.as_str().ok_or_else(|| {
                    failure(&path, "claudeAiMcpEverConnected must contain strings")
                })?;
                state.connectors.insert(id.into());
            }
        }
        let root = cwd
            .ancestors()
            .find(|directory| directory.join(".git").exists())
            .unwrap_or(&cwd);
        if let Some(projects) = config.get("projects") {
            let projects = projects
                .as_object()
                .ok_or_else(|| failure(&path, "projects must be an object"))?;
            if let Some(local) = projects.get(root.to_string_lossy().as_ref()) {
                if !local.is_object() {
                    return Err(failure(&path, "current project must be an object"));
                }
                read_json_servers(local, &path, &mut state.user, Harness::Claude)?;
                if let Some(disabled) = local.get("disabledMcpServers") {
                    let disabled = disabled
                        .as_array()
                        .ok_or_else(|| failure(&path, "disabledMcpServers must be an array"))?;
                    for name in disabled {
                        let name = name.as_str().ok_or_else(|| {
                            failure(&path, "disabledMcpServers must contain strings")
                        })?;
                        state.disabled.insert(name.into());
                    }
                }
            }
        }
    }
    // Claude discovers project MCP configs through every ancestor, beyond the git root.
    for directory in cwd.ancestors() {
        let path = directory.join(".mcp.json");
        if let Some(config) = read_json_object(&path, Harness::Claude)? {
            read_json_servers(&config, &path, &mut state.project, Harness::Claude)?;
        }
    }
    Ok(state)
}

fn failure(path: &Path, message: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error(
        "enumeration-failed",
        format!(
            "cannot enumerate Claude MCP servers from {}: {message}",
            path.display()
        ),
        None,
    )
}

pub fn read_json_object(path: &Path, harness: Harness) -> Result<Option<Value>, Diagnostic> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(json_failure(harness, path, error)),
    };
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|error| json_failure(harness, path, error))?;
    if !value.is_object() {
        return Err(json_failure(harness, path, "expected an object"));
    }
    Ok(Some(value))
}

pub fn read_json_servers(
    config: &Value,
    path: &Path,
    names: &mut BTreeSet<String>,
    harness: Harness,
) -> Result<(), Diagnostic> {
    if let Some(servers) = config.get("mcpServers") {
        let servers = servers
            .as_object()
            .ok_or_else(|| json_failure(harness, path, "mcpServers must be an object"))?;
        for (name, server) in servers {
            if !server.is_object() {
                return Err(json_failure(
                    harness,
                    path,
                    format!("MCP server {name} must be an object"),
                ));
            }
            // Plugin MCP servers are capabilities of their Plugin, never individual MCP selections.
            if harness != Harness::Claude || !name.starts_with("plugin:") {
                names.insert(name.clone());
            }
        }
    }
    Ok(())
}

fn json_failure(harness: Harness, path: &Path, message: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error(
        "enumeration-failed",
        format!(
            "cannot enumerate {harness:?} MCP servers from {}: {message}",
            path.display()
        ),
        None,
    )
}
