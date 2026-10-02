//! Doctor-only Plugin inventories and overlap checks. No launch projection uses these facts.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::config::{ConfigLayers, SkillBinding};
use crate::diagnostic::{CapabilityKind, Diagnostic, Severity};
use crate::doctor::DoctorState;
use crate::harness::Harness;
use serde_json::Value;

pub struct PluginContents {
    pub id: String,
    pub namespace: String,
    pub root: PathBuf,
    /// Effective canonical skill names (qualified on Claude and Codex).
    pub skills: BTreeSet<String>,
    pub servers: BTreeMap<String, Value>,
}

#[derive(Default)]
pub struct PluginInventory {
    pub plugins: Vec<PluginContents>,
    /// Eligible Claude manual servers, excluding disabled and unapproved project entries.
    pub manual: BTreeMap<String, Value>,
}

pub fn audit(
    harness: Harness,
    inventory: &PluginInventory,
    layers: &ConfigLayers,
    state: &DoctorState,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut servers: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for plugin in &inventory.plugins {
        if harness != Harness::Claude {
            for key in plugin.servers.keys() {
                servers.entry(key).or_default().push(&plugin.id);
            }
        }
    }
    for (key, owners) in servers.into_iter().filter(|(_, owners)| owners.len() > 1) {
        let precedence = match harness {
            Harness::Codex => format!(
                "Codex keeps {} (alphabetically first Plugin ID)",
                owners.iter().min().unwrap()
            ),
            Harness::Copilot => {
                "Copilot keeps the later scanned Plugin (winner depends on loading order)".into()
            }
            Harness::Claude => unreachable!(),
        };
        diagnostics.push(Diagnostic {
            code: "plugin-item-clash",
            severity: Severity::Warning,
            message: format!(
                "Plugins {} compete for MCP server {key}; {precedence}",
                owners.join(" and ")
            ),
            harness: Some(harness),
            item: Some(Box::new(key.into())),
            ..Default::default()
        });
    }
    if harness == Harness::Claude {
        let mut skills: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for plugin in &inventory.plugins {
            for key in &plugin.skills {
                skills.entry(key).or_default().push(&plugin.id);
            }
        }
        for (key, owners) in skills.into_iter().filter(|(_, owners)| owners.len() > 1) {
            diagnostics.push(Diagnostic {
                code: "plugin-item-clash", severity: Severity::Warning,
                message: format!("Plugins {} compete for skill {key}; Claude keeps the first loaded skill (winner depends on loading order)", owners.join(" and ")),
                harness: Some(harness), item: Some(Box::new(key.into())), ..Default::default()
            });
        }
    }
    for (logical, skill) in &layers.skills {
        let (name, native) = match skill.value.binding(harness) {
            Some(SkillBinding::Native(name)) => {
                let effective = state
                    .harnesses
                    .iter()
                    .find(|s| s.harness == harness)
                    .and_then(|s| s.skills.as_ref())
                    .and_then(|s| s.native_name(harness, name));
                (effective.unwrap_or(name), true)
            }
            Some(SkillBinding::Path(path)) => {
                let Some(name) = state.skill_names.get(path) else {
                    continue;
                };
                (name, false)
            }
            _ => continue,
        };
        let project_shadows = state
            .harnesses
            .iter()
            .find(|s| s.harness == harness)
            .and_then(|s| s.skills.as_ref())
            .is_some_and(|s| {
                s.project.contains(name)
                    && (!native || (harness == Harness::Copilot && s.personal.contains(name)))
            });
        let plugin_shadows: Vec<_> = inventory
            .plugins
            .iter()
            // Native-installed Copilot versus personal precedence remains unverified.
            .filter(|p| harness == Harness::Claude && p.skills.contains(name))
            .map(|p| p.id.as_str())
            .collect();
        if project_shadows || !plugin_shadows.is_empty() {
            let owner = if project_shadows {
                "a project skill".into()
            } else {
                format!("Plugin {}", plugin_shadows.join(", "))
            };
            diagnostics.push(
                Diagnostic {
                    code: "skill-shadowed",
                    severity: Severity::Warning,
                    message: format!("Skill {logical} ({name}) is shadowed by {owner}"),
                    ..Default::default()
                }
                .for_capability(harness, CapabilityKind::Skill, logical, &skill.path)
                .with_item(name),
            );
        }
    }
    if harness == Harness::Claude {
        claude_dedup(inventory, layers, &mut diagnostics);
    }
    diagnostics
}

#[derive(Clone, Copy)]
enum SignatureMode {
    ManualComparison,
    PluginComparison,
}

// Unknown expansions must never produce a claim of suppression.
fn signature(server: &Value, mode: SignatureMode) -> Option<String> {
    if server
        .get("configError")
        .is_some_and(|v| v != false && !v.is_null())
    {
        return None;
    }
    let fields = if matches!(mode, SignatureMode::PluginComparison) {
        server.clone()
    } else {
        serde_json::json!({"command":server.get("command"),"args":server.get("args"),"url":server.get("url")})
    };
    let text = serde_json::to_string(&fields).ok()?;
    if text.contains("${") || text.contains("$CLAUDE") {
        return None;
    }
    if let Some(command) = server.get("command").and_then(Value::as_str)
        && server.get("type").is_none_or(|v| v == "stdio")
    {
        let args = server.get("args").cloned().unwrap_or(serde_json::json!([]));
        if !args.as_array()?.iter().all(Value::is_string) {
            return None;
        }
        let env = if matches!(mode, SignatureMode::PluginComparison) {
            let mut env = server.get("env").cloned().unwrap_or(serde_json::json!({}));
            let fields = env.as_object_mut()?;
            fields.remove("CLAUDE_PLUGIN_ROOT");
            fields.remove("CLAUDE_PLUGIN_DATA");
            env
        } else {
            Value::Null
        };
        return Some(serde_json::json!(["stdio", command, args, env]).to_string());
    }
    let original = server.get("url")?.as_str()?;
    let mut url = url::Url::parse(original).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    // Claude 2.1.283 CUr recognizes these ingress path markers, independent of host.
    if [
        "/v2/session_ingress/shttp/mcp/",
        "/v2/session_ingress/mcp/ws/",
        "/v2/ccr-sessions/",
        "/v1/code/",
    ]
    .iter()
    .any(|host| original.contains(host))
        && let Some(inner) = url
            .query_pairs()
            .find(|(key, _)| key == "mcp_url")
            .map(|(_, v)| v.into_owned())
    {
        url = url::Url::parse(&inner).ok()?;
    }
    url.set_fragment(None);
    let path = url
        .path()
        .strip_suffix('/')
        .unwrap_or(url.path())
        .to_owned();
    url.set_path(if path.is_empty() { "/" } else { &path });
    let normalized = url.to_string();
    Some(format!(
        "url:{}",
        if url.query().is_none() {
            normalized.strip_suffix('/').unwrap_or(&normalized)
        } else {
            &normalized
        }
    ))
}

fn claude_dedup(
    inventory: &PluginInventory,
    layers: &ConfigLayers,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut configured = inventory.manual.clone();
    for (name, server) in &layers.mcp {
        use crate::mcp::{McpBinding, McpDefinition};
        let value = match server.value.binding(Harness::Claude) {
            Some(McpBinding::Definition(McpDefinition::Stdio { command, args, .. })) => {
                serde_json::json!({"command":command,"args":args})
            }
            Some(McpBinding::Definition(McpDefinition::Http { url, .. })) => {
                serde_json::json!({"url":url})
            }
            _ => continue,
        };
        configured.insert(format!("definition {name} (when selected)"), value);
    }
    let manual: BTreeMap<_, _> = configured
        .iter()
        .filter_map(|(name, server)| {
            signature(server, SignatureMode::ManualComparison).map(|s| (s, name))
        })
        .collect();
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for plugin in &inventory.plugins {
        for (key, server) in &plugin.servers {
            let item = format!("plugin:{}:{key}", plugin.namespace);
            if let Some(retained) =
                signature(server, SignatureMode::ManualComparison).and_then(|s| manual.get(&s))
            {
                diagnostics.push(Diagnostic {
                    code: "plugin-server-dedup", severity: Severity::Note,
                    message: format!("{item} from {} duplicates eligible manual server {retained}; Claude keeps {retained}", plugin.id),
                    harness: Some(Harness::Claude), item: Some(Box::new(item)),
                    ..Default::default()
                });
            } else if let Some(signature) = signature(server, SignatureMode::PluginComparison) {
                groups
                    .entry(signature)
                    .or_default()
                    .push(format!("{item} ({})", plugin.id));
            }
        }
    }
    for items in groups.into_values().filter(|items| items.len() > 1) {
        diagnostics.push(Diagnostic {
            code: "plugin-server-dedup", severity: Severity::Note,
            message: format!("Plugin servers {} duplicate an endpoint; Claude keeps the first eligible entry (retained and suppressed entries depend on loading order)", items.join(" and ")),
            harness: Some(Harness::Claude), item: Some(Box::new(items.join(", "))),
            ..Default::default()
        });
    }
}
