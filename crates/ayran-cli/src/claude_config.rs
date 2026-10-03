//! Read-only effective Claude settings for conservative native-state proofs.

use std::collections::BTreeMap;
use std::ffi::OsString;

use ayran_core::diagnostic::Diagnostic;
use ayran_core::enumerate::InstalledPlugins;
use ayran_core::harness::Harness;
use ayran_core::mcp::McpState;
use ayran_core::skills::SkillState;

use crate::harness_home::HarnessHome;
use crate::mcp_enumeration::read_json_object;

pub fn apply(
    home: &HarnessHome,
    args: &[OsString],
    installed: &mut InstalledPlugins,
    skills: &mut SkillState,
    mcp: &mut McpState,
) -> Result<(), Diagnostic> {
    // The interaction of multiple --settings arguments has not been verified.
    if args.iter().any(|arg| {
        arg.to_str()
            .is_some_and(|arg| arg == "--settings" || arg.starts_with("--settings="))
    }) {
        return Ok(());
    }
    let cwd = std::env::current_dir()
        .map_err(|error| Diagnostic::error("enumeration-failed", error.to_string(), None))?;
    for directory in cwd.ancestors() {
        let git = directory.join(".git");
        if crate::skill_enumeration::exists(&git)? {
            // Worktrees can use the main checkout's local settings. Nested
            // Sessions can use root or legacy cwd local settings, depending on
            // platform and ownership. Keep overrides when that path is uncertain.
            if !git.is_dir()
                || (directory != cwd
                    && crate::skill_enumeration::exists(
                        &directory.join(".claude/settings.local.json"),
                    )?)
            {
                return Ok(());
            }
            break;
        }
    }
    let mut paths = vec![home.directory.join("settings.json")];
    // Shared project settings are read only from the primary working directory;
    // they are not inherited from parents like project Skills or MCP configs.
    for relative in [".claude/settings.json", ".claude/settings.local.json"] {
        let path = cwd.join(relative);
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    let mut plugins = BTreeMap::new();
    let mut overrides = BTreeMap::new();
    let mut denied = mcp.disabled.clone();
    for path in paths {
        let Some(settings) = read_json_object(&path, Harness::Claude)? else {
            continue;
        };
        if let Some(entries) = settings.get("enabledPlugins") {
            let Some(entries) = entries.as_object() else {
                // Invalid containers do not prove any effective per-entry value.
                return Ok(());
            };
            plugins.extend(
                entries
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
        }
        if let Some(entries) = settings.get("skillOverrides") {
            let Some(entries) = entries.as_object() else {
                return Ok(());
            };
            overrides.extend(
                entries
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
        }
        if let Some(entries) = settings.get("deniedMcpServers") {
            let Some(entries) = entries.as_array() else {
                return Ok(());
            };
            // Native denies accumulate across layers. Command/URL matchers are
            // deliberately excluded: they do not prove a particular name off.
            denied.extend(entries.iter().filter_map(|entry| {
                entry
                    .get("serverName")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            }));
        }
    }
    installed.claude_enabled = plugins;
    skills.claude_overrides = Some(overrides);
    mcp.claude_off = denied;
    Ok(())
}
