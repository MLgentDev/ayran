//! Read-only Plugin payload inventory used only by doctor.
use crate::mcp_enumeration::read_json_object;
use ayran_core::{
    diagnostic::Diagnostic,
    enumerate::InstalledPlugins,
    harness::Harness,
    plugin_audit::{PluginContents, PluginInventory},
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
enum PluginRootKind {
    Installed,
    SkillsDirectory,
}
struct PluginRoot {
    id: String,
    path: PathBuf,
    kind: PluginRootKind,
}
impl PluginRoot {
    fn installed(id: String, path: PathBuf) -> Self {
        Self {
            id,
            path,
            kind: PluginRootKind::Installed,
        }
    }
}

pub fn read(
    harness: Harness,
    home: &crate::harness_home::HarnessHome,
    installed: &InstalledPlugins,
) -> Result<PluginInventory, Diagnostic> {
    let roots = match harness {
        Harness::Codex => {
            let mut roots = Vec::new();
            for id in installed.user.union(&installed.project) {
                let (name, market) = id.split_once('@').expect("validated Plugin ID");
                let cache = home.directory.join("plugins/cache").join(market).join(name);
                if let Some(root) = crate::plugin_enumeration::active_codex_plugin_root(&cache)? {
                    roots.push(PluginRoot::installed(id.clone(), root));
                }
            }
            roots
        }
        Harness::Copilot => installed
            .paths
            .iter()
            .map(|(id, p)| PluginRoot::installed(id.clone(), p.clone()))
            .chain(
                installed
                    .direct
                    .iter()
                    .map(|p| PluginRoot::installed(p.display().to_string(), p.clone())),
            )
            .collect(),
        Harness::Claude => claude_roots(home, installed)?,
    };
    let mut inventory = PluginInventory::default();
    let mut seen_skills = BTreeSet::new();
    for root in roots {
        if let Some(mut plugin) =
            payload(harness, root.id, &root.path, root.kind, &mut seen_skills)?
        {
            if harness == Harness::Claude {
                marketplace_components(home, &mut plugin, &mut seen_skills)?;
            }
            inventory.plugins.push(plugin);
        }
    }
    if harness == Harness::Claude {
        inventory.manual = claude_manual(home)?;
    }
    Ok(inventory)
}

fn failure(path: &Path, message: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error(
        "enumeration-failed",
        format!(
            "cannot enumerate Plugin contents from {}: {message}",
            path.display()
        ),
        None,
    )
}
fn directories(path: &Path) -> Result<Vec<PathBuf>, Diagnostic> {
    let entries = match fs::read_dir(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(failure(path, e)),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| failure(path, e))?;
        if fs::metadata(entry.path())
            .map_err(|e| failure(&entry.path(), e))?
            .is_dir()
        {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}
fn claude_roots(
    home: &crate::harness_home::HarnessHome,
    installed: &InstalledPlugins,
) -> Result<Vec<PluginRoot>, Diagnostic> {
    let mut roots = Vec::new();
    let registry = home.directory.join("plugins/installed_plugins.json");
    let cwd = std::env::current_dir().map_err(|e| failure(Path::new("."), e))?;
    if let Some(value) = read_json_object(&registry, Harness::Claude)?
        && let Some(plugins) = value.get("plugins").and_then(Value::as_object)
    {
        for (id, entries) in plugins {
            for entry in entries
                .as_array()
                .ok_or_else(|| failure(&registry, "invalid installs"))?
            {
                let applicable = entry["scope"] == "user"
                    || entry
                        .get("projectPath")
                        .and_then(Value::as_str)
                        .is_some_and(|p| cwd.starts_with(p));
                if applicable {
                    let path = entry
                        .get("installPath")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            failure(&registry, format!("Plugin {id} has no installPath"))
                        })?;
                    roots.push(PluginRoot::installed(id.clone(), PathBuf::from(path)));
                }
            }
        }
    }
    let mut skill_roots = vec![home.directory.join("skills")];
    for ancestor in cwd.ancestors() {
        skill_roots.push(ancestor.join(".claude/skills"));
        if ancestor.join(".git").exists() {
            break;
        }
    }
    let mut seen = BTreeSet::new();
    for skills in skill_roots {
        for root in directories(&skills)? {
            let basename = root.file_name().unwrap().to_string_lossy();
            if basename.starts_with('.') || basename.eq_ignore_ascii_case("synced") {
                continue;
            }
            if let Some(manifest) =
                read_json_object(&root.join(".claude-plugin/plugin.json"), Harness::Claude)?
            {
                let name = manifest.get("name").and_then(Value::as_str).unwrap_or("");
                let id = format!("{name}@skills-dir");
                // The existing discovery owns scope/trust exclusions.
                if (installed.user.contains(&id) || installed.project.contains(&id))
                    && seen.insert(root.canonicalize().map_err(|e| failure(&root, e))?)
                {
                    roots.push(PluginRoot {
                        id,
                        path: root,
                        kind: PluginRootKind::SkillsDirectory,
                    });
                }
            }
        }
    }
    Ok(roots)
}

fn payload(
    harness: Harness,
    id: String,
    root: &Path,
    kind: PluginRootKind,
    seen_skills: &mut BTreeSet<PathBuf>,
) -> Result<Option<PluginContents>, Diagnostic> {
    let candidates: &[&str] = match harness {
        Harness::Claude => &[".claude-plugin/plugin.json"],
        Harness::Codex => &[
            "plugin.json",
            ".codex-plugin/plugin.json",
            ".claude-plugin/plugin.json",
            ".cursor-plugin/plugin.json",
        ],
        Harness::Copilot => &[
            ".plugin/plugin.json",
            ".claude-plugin/plugin.json",
            "plugin.json",
        ],
    };
    let mut manifest = None;
    for relative in candidates {
        let path = root.join(relative);
        if let Some(value) = read_json_object(&path, harness)? {
            manifest = Some((path, value));
            break;
        }
    }
    let Some((path, manifest)) = manifest else {
        return Err(failure(root, "installed Plugin has no readable manifest"));
    };
    let namespace = manifest
        .get("name")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| failure(&path, "manifest name must be a nonempty string"))?
        .to_owned();
    let agent = manifest
        .get("$schema")
        .and_then(Value::as_str)
        .is_some_and(|s| s.starts_with("https://agent-plugins.org/schemas/"));
    if agent && manifest["$schema"] != "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json"
    {
        return Err(failure(&path, "unsupported Agent Plugins schema"));
    }
    let mut plugin = PluginContents {
        id,
        namespace,
        root: root.into(),
        skills: BTreeSet::new(),
        servers: BTreeMap::new(),
    };
    let extra = component_paths(manifest.get("skills"), &path)?;
    let mut skill_paths = if harness == Harness::Claude || extra.is_empty() {
        vec![root.join("skills")]
    } else {
        Vec::new()
    };
    skill_paths.extend(extra.iter().map(|p| root.join(p)));
    if harness == Harness::Codex && !agent {
        skill_paths.push(root.join(".codex-plugin/migrated-command-skills"));
    }
    let mut seen = BTreeSet::new();
    for skills in skill_paths {
        if kind == PluginRootKind::SkillsDirectory
            && skills.canonicalize().ok() == root.canonicalize().ok()
        {
            continue;
        }
        scan_skills(
            harness,
            &skills,
            &mut plugin,
            if harness == Harness::Codex && !agent {
                SkillDiscovery::Recursive
            } else {
                SkillDiscovery::Children
            },
            SkillPosition::Root,
            &mut seen,
            seen_skills,
        )?;
    }
    if harness == Harness::Claude
        && kind != PluginRootKind::SkillsDirectory
        && plugin.skills.is_empty()
        && extra.is_empty()
    {
        scan_skills(
            harness,
            root,
            &mut plugin,
            SkillDiscovery::Children,
            SkillPosition::Root,
            &mut seen,
            seen_skills,
        )?;
    }
    if agent {
        if let Some(config) = read_json_object(&root.join("mcp.json"), harness)? {
            merge_servers(&config, &path, &mut plugin.servers)?;
        }
    } else if harness == Harness::Claude {
        if let Some(config) = read_json_object(&root.join(".mcp.json"), harness)? {
            merge_servers(&config, &path, &mut plugin.servers)?;
        }
        if let Some(config) = manifest.get("mcpServers") {
            load_servers(harness, root, config, &path, &mut plugin.servers)?;
        }
    } else {
        let defaults: &[&str] = if harness == Harness::Copilot {
            &[".mcp.json", ".github/mcp.json"]
        } else {
            &[]
        };
        let mut found = false;
        for default in defaults {
            let config_path = root.join(default);
            if let Some(config) = read_json_object(&config_path, harness)? {
                merge_servers(&config, &config_path, &mut plugin.servers)?;
                found = true;
                break;
            }
        }
        if !found {
            match manifest.get("mcpServers") {
                Some(value) => load_servers(harness, root, value, &path, &mut plugin.servers)?,
                None if harness == Harness::Codex => {
                    if let Some(config) = read_json_object(&root.join(".mcp.json"), harness)? {
                        merge_servers(&config, &path, &mut plugin.servers)?;
                    }
                }
                None => {}
            }
        }
    }
    Ok(Some(plugin))
}
fn component_paths(value: Option<&Value>, path: &Path) -> Result<Vec<String>, Diagnostic> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::String(s)) => Ok(vec![s.clone()]),
        Some(Value::Array(values)) => values
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| failure(path, "component paths must be strings"))
            })
            .collect(),
        _ => Err(failure(path, "component paths must be a string or array")),
    }
}
#[derive(Clone, Copy, PartialEq)]
enum SkillDiscovery {
    Children,
    Recursive,
}
#[derive(Clone, Copy, PartialEq)]
enum SkillPosition {
    Root,
    Child,
}

fn scan_skills(
    harness: Harness,
    path: &Path,
    plugin: &mut PluginContents,
    discovery: SkillDiscovery,
    position: SkillPosition,
    seen: &mut BTreeSet<PathBuf>,
    seen_skills: &mut BTreeSet<PathBuf>,
) -> Result<(), Diagnostic> {
    let identity = match path.canonicalize() {
        Ok(p) => p,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(failure(path, e)),
    };
    if !seen.insert(identity) {
        return Ok(());
    }
    let file = path.join("SKILL.md");
    match fs::read_to_string(&file) {
        Ok(contents) => {
            let display = crate::skill_activation::skill_name(path, &contents)
                .map_err(|d| failure(&file, d.message))?;
            let bare = if harness == Harness::Claude && position == SkillPosition::Child {
                path.file_name().unwrap().to_string_lossy().into_owned()
            } else {
                display.clone()
            };
            let bare = if harness == Harness::Claude {
                bare.strip_prefix(&format!("{}:", plugin.namespace))
                    .unwrap_or(&bare)
                    .trim()
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                            c
                        } else {
                            '-'
                        }
                    })
                    .collect::<String>()
            } else {
                bare
            };
            let canonical = if harness == Harness::Copilot {
                bare
            } else {
                format!("{}:{bare}", plugin.namespace)
            };
            let identity = file.canonicalize().map_err(|e| failure(&file, e))?;
            if harness != Harness::Claude || seen_skills.insert(identity) {
                plugin.skills.insert(canonical);
            }
            return Ok(());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(failure(&file, e)),
    }
    for child in directories(path)? {
        if discovery == SkillDiscovery::Recursive || position == SkillPosition::Root {
            scan_skills(
                harness,
                &child,
                plugin,
                discovery,
                SkillPosition::Child,
                seen,
                seen_skills,
            )?;
        }
    }
    Ok(())
}
fn merge_servers(
    value: &Value,
    path: &Path,
    servers: &mut BTreeMap<String, Value>,
) -> Result<(), Diagnostic> {
    let map = value
        .get("mcpServers")
        .unwrap_or(value)
        .as_object()
        .ok_or_else(|| failure(path, "MCP servers must be an object"))?;
    for (name, server) in map {
        if !server.is_object() {
            return Err(failure(
                path,
                format!("MCP server {name} must be an object"),
            ));
        }
        servers.insert(name.clone(), server.clone());
    }
    Ok(())
}
fn load_servers(
    harness: Harness,
    root: &Path,
    value: &Value,
    path: &Path,
    servers: &mut BTreeMap<String, Value>,
) -> Result<(), Diagnostic> {
    match value {
        Value::String(relative) => {
            if !relative.ends_with(".json") {
                return Err(failure(
                    path,
                    "incomplete inventory: MCP bundles are not inspected",
                ));
            }
            let file = root.join(relative);
            let config = read_json_object(&file, harness)?
                .ok_or_else(|| failure(&file, "declared MCP file is missing"))?;
            merge_servers(&config, &file, servers)
        }
        Value::Array(entries) if harness == Harness::Claude => {
            for entry in entries {
                load_servers(harness, root, entry, path, servers)?;
            }
            Ok(())
        }
        Value::Object(_) => merge_servers(value, path, servers),
        _ => Err(failure(path, "unsupported MCP component")),
    }
}
fn claude_manual(
    home: &crate::harness_home::HarnessHome,
) -> Result<BTreeMap<String, Value>, Diagnostic> {
    let path = &home.claude_mcp;
    let mut servers = BTreeMap::new();
    let cwd = std::env::current_dir().map_err(|e| failure(path, e))?;
    let root = cwd
        .ancestors()
        .find(|p| p.join(".git").exists())
        .unwrap_or(&cwd);
    let mut disabled = BTreeSet::new();
    let mut settings = serde_json::json!({});
    if let Some(config) = read_json_object(path, Harness::Claude)? {
        if let Some(value) = config.get("mcpServers") {
            merge_servers(value, path, &mut servers)?;
        }
        if let Some(local) = config
            .get("projects")
            .and_then(|v| v.get(root.to_string_lossy().as_ref()))
        {
            if let Some(value) = local.get("mcpServers") {
                merge_servers(value, path, &mut servers)?;
            }
            if let Some(values) = local.get("disabledMcpServers").and_then(Value::as_array) {
                disabled.extend(values.iter().filter_map(Value::as_str).map(str::to_owned));
            }
        }
    }
    let mut setting_paths = vec![home.directory.join("settings.json")];
    let ancestors: Vec<_> = cwd.ancestors().collect();
    for directory in ancestors.iter().rev() {
        for relative in [".claude/settings.json", ".claude/settings.local.json"] {
            let path = directory.join(relative);
            if !setting_paths.contains(&path) {
                setting_paths.push(path);
            }
        }
    }
    for path in setting_paths {
        if let Some(value) = read_json_object(&path, Harness::Claude)? {
            settings
                .as_object_mut()
                .unwrap()
                .extend(value.as_object().unwrap().clone());
        }
    }
    let approved: BTreeSet<_> = settings
        .get("enabledMcpjsonServers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let rejected: BTreeSet<_> = settings
        .get("disabledMcpjsonServers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    for directory in ancestors.iter().rev() {
        let path = directory.join(".mcp.json");
        if let Some(config) = read_json_object(&path, Harness::Claude)? {
            let mut project = BTreeMap::new();
            merge_servers(&config, &path, &mut project)?;
            for (name, server) in project {
                if (settings["enableAllProjectMcpServers"] == true
                    || approved.contains(name.as_str()))
                    && !rejected.contains(name.as_str())
                {
                    servers.insert(name, server);
                }
            }
        }
    }
    servers
        .retain(|name, server| !disabled.contains(name) && policy_allows(&settings, name, server));
    Ok(servers)
}

fn policy_allows(settings: &Value, name: &str, server: &Value) -> bool {
    let matches = |entry: &Value| {
        entry
            .get("serverName")
            .and_then(Value::as_str)
            .is_some_and(|s| s == name)
            || entry
                .get("serverCommand")
                .and_then(Value::as_array)
                .is_some_and(|command| {
                    let mut actual = vec![server.get("command").cloned().unwrap_or(Value::Null)];
                    actual.extend(
                        server
                            .get("args")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .cloned(),
                    );
                    command == &actual
                })
            || entry
                .get("serverUrl")
                .and_then(Value::as_str)
                .is_some_and(|s| server.get("url").and_then(Value::as_str) == Some(s))
    };
    let denied = settings
        .get("deniedMcpServers")
        .and_then(Value::as_array)
        .is_some_and(|entries| entries.iter().any(matches));
    let allowed = settings.get("allowedMcpServers").is_none_or(|entries| {
        entries
            .as_array()
            .is_some_and(|entries| entries.iter().any(matches))
    });
    !denied && allowed
}

fn marketplace_components(
    home: &crate::harness_home::HarnessHome,
    plugin: &mut PluginContents,
    seen_skills: &mut BTreeSet<PathBuf>,
) -> Result<(), Diagnostic> {
    let Some((name, market)) = plugin.id.split_once('@') else {
        return Ok(());
    };
    if market == "skills-dir" {
        return Ok(());
    }
    let known = home.directory.join("plugins/known_marketplaces.json");
    let Some(registry) = read_json_object(&known, Harness::Claude)? else {
        return Ok(());
    };
    let Some(location) = registry
        .get(market)
        .and_then(|v| v.get("installLocation"))
        .and_then(Value::as_str)
    else {
        return Ok(());
    };
    let path = Path::new(location).join(".claude-plugin/marketplace.json");
    let marketplace = read_json_object(&path, Harness::Claude)?
        .ok_or_else(|| failure(&path, "registered marketplace manifest is missing"))?;
    let entries = marketplace
        .get("plugins")
        .and_then(Value::as_array)
        .ok_or_else(|| failure(&path, "marketplace plugins must be an array"))?;
    if let Some(entry) = entries.iter().find(|entry| entry["name"] == name) {
        for relative in component_paths(entry.get("skills"), &path)? {
            let root = plugin.root.join(relative);
            let mut seen = BTreeSet::new();
            scan_skills(
                Harness::Claude,
                &root,
                plugin,
                SkillDiscovery::Children,
                SkillPosition::Root,
                &mut seen,
                seen_skills,
            )?;
        }
        if let Some(value) = entry.get("mcpServers") {
            load_servers(
                Harness::Claude,
                &plugin.root,
                value,
                &path,
                &mut plugin.servers,
            )?;
        }
    }
    Ok(())
}
