//! Read-only Copilot MCP discovery; selection and hiding remain in the pure core.

use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};

use crate::mcp_enumeration::{read_json_object, read_json_servers};
use ayran_core::diagnostic::Diagnostic;
use ayran_core::harness::Harness;
use ayran_core::mcp::McpState;
use serde_json::Value;

pub fn read(harness_home: &crate::harness_home::HarnessHome) -> Result<McpState, Diagnostic> {
    let home = harness_home.directory.clone();
    let mut state = McpState::default();
    let path = home.join("mcp-config.json");
    if let Some(config) = read_json_object(&path, Harness::Copilot)? {
        read_json_servers(&config, &path, &mut state.user, Harness::Copilot)?;
    }
    let cwd = env::current_dir().map_err(|error| failure(Path::new("."), error))?;
    for directory in cwd.ancestors() {
        for relative in [".mcp.json", ".github/mcp.json"] {
            let path = directory.join(relative);
            if let Some(config) = read_json_object(&path, Harness::Copilot)? {
                read_json_servers(&config, &path, &mut state.project, Harness::Copilot)?;
            }
        }
        if crate::skill_enumeration::exists(&directory.join(".git"))? {
            break;
        }
    }
    Ok(state)
}

pub fn read_plugins<'a>(
    roots: impl Iterator<Item = &'a PathBuf>,
    state: &mut McpState,
) -> Result<(), Diagnostic> {
    for root in roots {
        let mut manifest = None;
        for relative in [
            ".plugin/plugin.json",
            ".claude-plugin/plugin.json",
            "plugin.json",
        ] {
            let path = root.join(relative);
            if let Some(value) = read_json_object(&path, Harness::Copilot)? {
                manifest = Some((path, value));
                break;
            }
        }
        let Some((path, manifest)) = manifest else {
            continue;
        };
        let agent_plugin = manifest.get("$schema").and_then(Value::as_str)
            == Some("https://agent-plugins.org/schemas/1.0.0/plugin.schema.json");
        if agent_plugin {
            let path = root.join("mcp.json");
            if let Some(config) = read_json_object(&path, Harness::Copilot)? {
                read_plugin_servers(&config, &path, &mut state.plugins)?;
            }
            continue;
        }
        let mut found = false;
        for relative in [".mcp.json", ".github/mcp.json"] {
            let path = root.join(relative);
            if let Some(config) = read_json_object(&path, Harness::Copilot)? {
                read_plugin_servers(&config, &path, &mut state.plugins)?;
                found = true;
                break;
            }
        }
        if found {
            continue;
        }
        match manifest.get("mcpServers") {
            Some(Value::String(relative)) => {
                let path = root.join(relative);
                if let Some(config) = read_json_object(&path, Harness::Copilot)? {
                    read_plugin_servers(&config, &path, &mut state.plugins)?;
                }
            }
            Some(value) if value.is_object() => {
                read_plugin_servers(value, &path, &mut state.plugins)?
            }
            _ => {}
        }
    }
    Ok(())
}

fn read_plugin_servers(
    config: &Value,
    path: &Path,
    names: &mut BTreeSet<String>,
) -> Result<(), Diagnostic> {
    let servers = config.get("mcpServers").unwrap_or(config);
    read_json_servers(
        &serde_json::json!({"mcpServers": servers}),
        path,
        names,
        Harness::Copilot,
    )
}

fn failure(path: &Path, message: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error(
        "enumeration-failed",
        format!(
            "cannot enumerate Copilot MCP servers from {}: {message}",
            path.display()
        ),
        None,
    )
}
