use std::ffi::OsString;
use std::fmt;

use crate::config::{ConfigLayers, PluginBinding};
use crate::diagnostic::{Diagnostic, Severity};
use crate::enumerate::InstalledPlugins;
use crate::harness::{Effort, Harness};
use crate::launch::{LaunchPlan, ResolutionTrace};

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Request {
    pub alias: Option<String>,
    #[serde(skip)]
    pub harness: Option<Harness>,
    pub model: Option<String>,
    pub effort: Option<Effort>,
    pub plugins: Vec<String>,
    pub skills: Vec<String>,
    pub mcp: Vec<String>,
    pub profiles: Vec<String>,
    pub no_plugins: Vec<String>,
    pub no_skills: Vec<String>,
    pub no_mcp: Vec<String>,
    pub no_profiles: Vec<String>,
    pub no_defaults: bool,
    #[serde(skip)]
    pub passthrough: Vec<OsString>,
}

pub(crate) enum CapabilityOrigin<'a> {
    Default,
    DefaultProfile(&'a str),
    Profile(&'a str),
    Cli(&'static str),
    Alias(&'a str),
}

impl CapabilityOrigin<'_> {
    pub(crate) fn is_direct_selection(&self) -> bool {
        matches!(self, Self::Cli(_) | Self::Alias(_))
    }
}

impl fmt::Display for CapabilityOrigin<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => f.write_str("Default"),
            Self::DefaultProfile(name) => write!(f, "Default (via Profile {name})"),
            Self::Profile(name) => write!(f, "via Profile {name}"),
            Self::Cli(flag) => write!(f, "explicit ({flag})"),
            Self::Alias(name) => write!(f, "explicit (Alias {name})"),
        }
    }
}

impl Request {
    /// Choose the Harness before the caller enumerates its installed state.
    pub fn resolve_harness(
        &self,
        layers: &ConfigLayers,
    ) -> Result<(Harness, String), Vec<Diagnostic>> {
        let alias = self
            .alias
            .as_ref()
            .map(|name| {
                layers
                    .aliases
                    .get(name)
                    .ok_or_else(|| {
                        vec![Diagnostic::error(
                            "unknown-alias",
                            format!("unknown Alias {name}"),
                            None,
                        )]
                    })
                    .map(|alias| (name, alias))
            })
            .transpose()?;
        if let (Some((name, definition)), Some(harness)) = (alias, self.harness)
            && harness != definition.harness
        {
            return Err(vec![Diagnostic::error(
                "usage",
                format!("Harness flag conflicts with Alias {name}"),
                None,
            )]);
        }
        self.harness
            .map(|value| (value, "flag".to_owned()))
            .or_else(|| alias.map(|(name, value)| (value.harness, format!("Alias {name}"))))
            .or_else(|| {
                layers
                    .default_harness
                    .as_ref()
                    .map(|source| (source.value, source.path.display().to_string()))
            })
            .ok_or_else(|| {
                vec![Diagnostic::error(
                    "no-harness",
                    "no Harness chosen",
                    Some("pass --claude, --codex, or --copilot, or set default_harness in user config"),
                )]
            })
    }
}

pub fn resolve(
    request: Request,
    layers: &ConfigLayers,
    installed: &InstalledPlugins,
    skill_state: &crate::skills::SkillState,
    mcp_state: &crate::mcp::McpState,
) -> Result<LaunchPlan, Vec<Diagnostic>> {
    layers
        .validate_profiles()
        .map_err(|diagnostic| vec![diagnostic])?;
    let (harness, harness_source) = request.resolve_harness(layers)?;
    let alias = request
        .alias
        .as_ref()
        .map(|name| (name, &layers.aliases[name]));
    let alias_source = alias.map(|(name, _)| format!("Alias {name}"));
    let settings = layers.settings(harness);
    let model = request
        .model
        .as_deref()
        .map(|value| (value, "flag".to_owned()))
        .or_else(|| {
            alias.and_then(|(_, value)| {
                value
                    .model
                    .as_deref()
                    .map(|model| (model, alias_source.clone().unwrap()))
            })
        })
        .or_else(|| {
            settings
                .model
                .as_ref()
                .map(|source| (source.value.as_str(), source.path.display().to_string()))
        });
    let effort = request
        .effort
        .map(|value| (value, "flag".to_owned()))
        .or_else(|| {
            alias.and_then(|(_, value)| {
                value
                    .effort
                    .map(|effort| (effort, alias_source.clone().unwrap()))
            })
        })
        .or_else(|| {
            settings
                .effort
                .as_ref()
                .map(|source| (source.value, source.path.display().to_string()))
        });
    let mut options = harness.translate_model_and_effort(
        model.as_ref().map(|(value, _)| *value),
        effort.as_ref().map(|(value, _)| *value),
    );
    let mut plugins = Vec::new();
    let mut diagnostics = Vec::new();
    let leak = match harness {
        Harness::Claude => Some(
            "claude-synced-plugin: Claude account-synced Plugins cannot be completely enumerated before startup; unselected or organization-required Plugins may remain visible",
        ),
        Harness::Codex => Some(
            "codex-remote-plugin: Codex account/workspace remote Plugins cannot be enumerated or hidden from local Harness state; unselected remote Plugins may remain visible",
        ),
        Harness::Copilot => None,
    };
    if let Some(message) = leak {
        diagnostics.push(Diagnostic {
            code: "leak",
            severity: Severity::Warning,
            message: message.into(),
            hint: None,
            layer: None,
            harness: Some(harness),
            cause: Some(Box::new(message.split(':').next().unwrap().into())),
            item: None,
            ..Diagnostic::default()
        });
    }
    let mut plugin_paths = Vec::new();
    let mut native_plugin_paths = Vec::new();
    let mut env_set = Vec::new();
    let mut enabled = std::collections::BTreeMap::new();
    let mut selections = std::collections::BTreeMap::new();
    for (name, plugin) in &layers.plugins {
        if plugin.value.default {
            selections.insert(name, CapabilityOrigin::Default);
        }
    }
    let (profile_selections, profile_trace, profile_diagnostics) =
        profile_selections(&request, layers)?;
    let skill_selection =
        crate::skills::select_from_profiles(&request, layers, harness, profile_selections.skills)?;
    let mcp_profiles = profile_selections.mcp;
    selections.extend(profile_selections.plugins);
    diagnostics.extend(profile_diagnostics);
    if let Some((alias_name, definition)) = alias {
        for name in &definition.plugins {
            selections.insert(name, CapabilityOrigin::Alias(alias_name));
        }
    }
    for name in &request.plugins {
        selections.insert(name, CapabilityOrigin::Cli("--plugin"));
    }
    for name in layers
        .plugins
        .keys()
        .filter(|name| !selections.contains_key(name))
    {
        let reason = if request.no_plugins.contains(name) {
            "disabled by --no-plugin"
        } else {
            "unselected"
        };
        plugins.push(format!("Plugin {name}: {reason}"));
    }
    let mut selection_order: Vec<_> = request.plugins.iter().collect();
    if let Some((_, definition)) = alias {
        selection_order.extend(&definition.plugins);
    }
    selection_order.extend(selections.keys().copied());
    let mut seen = std::collections::BTreeSet::new();
    for name in selection_order {
        let Some(origin) = selections.get(name) else {
            continue;
        };
        if !seen.insert(name) {
            continue;
        }
        if request.no_plugins.contains(name) {
            plugins.push(format!("Plugin {name}: disabled by --no-plugin"));
            continue;
        }
        if matches!(origin, CapabilityOrigin::Default) {
            let reason = if request.no_defaults {
                Some("disabled by --no-defaults".to_owned())
            } else if let Some((alias_name, definition)) = alias
                && !definition.defaults
            {
                Some(format!("disabled by Alias {alias_name} (defaults = false)"))
            } else {
                layers
                    .disabled_defaults
                    .get(name)
                    .map(|path| format!("disabled by layer {} (Defaults)", path.display()))
            };
            if let Some(reason) = reason {
                plugins.push(format!("Plugin {name}: {reason}"));
                continue;
            }
        }
        if !origin.is_direct_selection()
            && let Some(path) = layers.disabled_plugins.get(name)
        {
            plugins.push(format!(
                "Plugin {name}: disabled by layer {}",
                path.display()
            ));
            continue;
        }
        if !origin.is_direct_selection()
            && let Some((alias_name, definition)) = alias
            && definition.disabled_plugins.contains(name)
        {
            plugins.push(format!("Plugin {name}: disabled by Alias {alias_name}"));
            continue;
        }
        let plugin = layers.plugins.get(name).ok_or_else(|| {
            vec![Diagnostic::error(
                "unknown-plugin",
                format!("unknown Plugin {name}"),
                None,
            )]
        })?;
        let binding = plugin.value.binding(harness).ok_or_else(|| {
            vec![
                Diagnostic::error(
                    "missing-binding",
                    format!("Plugin {name} has no Binding for {}", harness.binary()),
                    None,
                )
                .for_capability(
                    harness,
                    crate::diagnostic::CapabilityKind::Plugin,
                    name,
                    &plugin.path,
                ),
            ]
        })?;
        let item = match binding {
            PluginBinding::Native(id) => {
                if harness == Harness::Copilot && !valid_copilot_native_id(id) {
                    return Err(vec![Diagnostic::error(
                        "unsupported-binding",
                        format!("Copilot native Plugin Binding {id} must use name@marketplace"),
                        Some(
                            "use a path Binding for a Plugin installed directly from a repository",
                        ),
                    ).for_capability(harness, crate::diagnostic::CapabilityKind::Plugin, name, &plugin.path).with_item(id)]);
                }
                let installed = if harness == Harness::Copilot {
                    installed.paths.contains_key(id)
                } else {
                    installed.user.contains(id) || installed.project.contains(id)
                };
                if !installed {
                    return Err(vec![
                        Diagnostic::error(
                            "native-not-found",
                            format!(
                                "Plugin {name} binds {id}, which is not installed on {}",
                                harness.binary()
                            ),
                            None,
                        )
                        .for_capability(
                            harness,
                            crate::diagnostic::CapabilityKind::Plugin,
                            name,
                            &plugin.path,
                        )
                        .with_item(id),
                    ]);
                }
                enabled.insert(id, true);
                id.clone()
            }
            PluginBinding::Path(path) => {
                if harness == Harness::Codex {
                    return Err(vec![
                        Diagnostic::error(
                            "unsupported-binding",
                            format!("Plugin {name} has a path Binding, which codex cannot load"),
                            None,
                        )
                        .for_capability(
                            harness,
                            crate::diagnostic::CapabilityKind::Plugin,
                            name,
                            &plugin.path,
                        )
                        .with_item(path.display().to_string()),
                    ]);
                }
                if !plugin_paths.contains(path) {
                    plugin_paths.push(path.clone());
                }
                format!("path {}", path.display())
            }
            PluginBinding::Absent => {
                if !origin.is_direct_selection() {
                    let message = format!(
                        "Plugin {name}: skipped because of a false Binding for {} ({origin}, {})",
                        harness.binary(),
                        plugin.path.display()
                    );
                    plugins.push(message.clone());
                    diagnostics.push(
                        Diagnostic {
                            code: "binding-skipped",
                            severity: Severity::Note,
                            message,
                            hint: None,
                            layer: Some(plugin.path.display().to_string()),
                            ..Diagnostic::default()
                        }
                        .for_capability(
                            harness,
                            crate::diagnostic::CapabilityKind::Plugin,
                            name,
                            &plugin.path,
                        ),
                    );
                    continue;
                }
                return Err(vec![
                    Diagnostic::error(
                        "binding-absent",
                        format!(
                            "explicitly selected Plugin {name} is deliberately absent for {}",
                            harness.binary()
                        ),
                        None,
                    )
                    .for_capability(
                        harness,
                        crate::diagnostic::CapabilityKind::Plugin,
                        name,
                        &plugin.path,
                    ),
                ]);
            }
        };
        plugins.push(format!(
            "Plugin {name}: {origin} → {item} ({})",
            plugin.path.display()
        ));
    }
    for id in &installed.user {
        if !enabled.contains_key(id) && !installed.project.contains(id) {
            plugins.push(format!("Plugin {id}: hidden (unselected user install)"));
            if harness == Harness::Copilot {
                if env_set.is_empty() {
                    env_set.push(("COPILOT_PLUGIN_DIR_ONLY".into(), "true".into()));
                }
            } else {
                enabled.insert(id, false);
            }
        } else if harness == Harness::Claude
            && !enabled.contains_key(id)
            && installed.project.contains(id)
            && id.ends_with("@skills-dir")
        {
            diagnostics.push(Diagnostic {
                code: "leak",
                severity: Severity::Warning,
                message: format!(
                    "claude-project-plugin-shadow: unselected personal Plugin {id} may remain visible because hiding it would also hide the project Plugin"
                ),
                hint: None,
                layer: None,
                harness: Some(harness),
                cause: Some(Box::new("claude-project-plugin-shadow".into())),
                item: Some(Box::new(id.clone())),
                ..Diagnostic::default()
            });
        }
    }
    for path in &installed.direct {
        diagnostics.push(Diagnostic {
            code: "copilot-direct-plugin",
            severity: Severity::Note,
            message: format!(
                "Copilot direct Plugin {} can only be selected by a path Binding",
                path.display()
            ),
            hint: None,
            layer: None,
            item: Some(Box::new(path.display().to_string())),
            ..Diagnostic::default()
        });
        if !plugin_paths.iter().any(|selected| {
            installed.canonical_paths.get(selected).unwrap_or(selected)
                == installed.canonical_paths.get(path).unwrap_or(path)
        }) {
            if env_set.is_empty() {
                env_set.push(("COPILOT_PLUGIN_DIR_ONLY".into(), "true".into()));
            }
            plugins.push(format!(
                "Plugin path {}: hidden (unselected direct install)",
                path.display()
            ));
        }
    }
    if harness == Harness::Copilot
        && settings.home_mode == crate::launch::HomeMode::Isolated
        && !enabled.is_empty()
        && env_set.is_empty()
    {
        // Native Bindings load from the shared home, even when the Isolated home
        // has an install with the same ID. Only the explicit directories should load.
        env_set.push(("COPILOT_PLUGIN_DIR_ONLY".into(), "true".into()));
    }
    let resolved_skills = crate::skills::resolve(skill_selection, layers, harness, skill_state)?;
    diagnostics.extend(resolved_skills.diagnostics);
    let mut resolved_mcp = crate::mcp::resolve(&request, layers, harness, mcp_state, mcp_profiles)?;
    diagnostics.append(&mut resolved_mcp.diagnostics);
    if harness == Harness::Claude {
        let mut settings = serde_json::json!({"syncClaudeAiSkills": false});
        if resolved_mcp.connectors.is_empty() {
            settings["disableClaudeAiConnectors"] = serde_json::json!(true);
        }
        if !enabled.is_empty() {
            settings["enabledPlugins"] = serde_json::json!(enabled);
            plugins.push(format!(
                "Plugin settings: {}",
                serde_json::json!({"enabledPlugins": enabled})
            ));
        }
        if !resolved_skills.overrides.is_empty() {
            settings["skillOverrides"] = serde_json::json!(resolved_skills.overrides);
        }
        if !resolved_mcp.denied.is_empty() {
            settings["deniedMcpServers"] = serde_json::json!(
                resolved_mcp
                    .denied
                    .iter()
                    .map(|name| serde_json::json!({"serverName": name}))
                    .collect::<Vec<_>>()
            );
        }
        options
            .args
            .extend(["--settings".into(), settings.to_string().into()]);
    } else if !enabled.is_empty() {
        match harness {
            Harness::Claude => unreachable!("Claude overrides are merged above"),
            Harness::Codex => {
                for (id, enabled) in &enabled {
                    let key = toml::Value::String((*id).clone());
                    // Codex splits override keys on dots and preserves quotes literally.
                    let setting = if id.contains('.') {
                        format!("plugins={{{key}={{enabled={enabled}}}}}")
                    } else {
                        format!("plugins.{id}.enabled={enabled}")
                    };
                    options.args.extend(["-c".into(), setting.into()]);
                }
            }
            Harness::Copilot => {
                for id in enabled.keys() {
                    let path = installed.paths.get(*id).ok_or_else(|| {
                        vec![Diagnostic::error(
                            "enumeration-failed",
                            format!("Copilot Plugin {id} has no enumerated install path"),
                            None,
                        )]
                    })?;
                    native_plugin_paths.push(path.clone());
                }
            }
        }
    }
    if !resolved_skills.path_overrides.is_empty() {
        let rules = resolved_skills
            .path_overrides
            .iter()
            .map(|(path, enabled)| {
                format!(
                    "{{path={},enabled={enabled}}}",
                    toml::Value::String(path.to_string_lossy().into_owned())
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        options
            .args
            .extend(["-c".into(), format!("skills.config=[{rules}]").into()]);
    }
    for path in native_plugin_paths.iter().chain(&plugin_paths) {
        options
            .args
            .extend(["--plugin-dir".into(), path.as_os_str().to_os_string()]);
    }
    let skills = resolved_skills.trace;
    let generated_skills = resolved_skills.generated;
    if let Some(cache) = &generated_skills {
        let (flag, directory) = if harness == Harness::Copilot {
            ("--plugin-dir", cache.directory.join("ayran"))
        } else {
            ("--add-dir", cache.directory.clone())
        };
        options
            .args
            .extend([flag.into(), directory.into_os_string()]);
    }
    for value in &resolved_mcp.overrides {
        options.args.extend(["-c".into(), value.into()]);
    }
    options
        .args
        .extend(resolved_mcp.args.iter().map(Into::into));
    options.args.extend(request.passthrough);
    for diagnostic in &mut diagnostics {
        diagnostic.harness.get_or_insert(harness);
    }
    Ok(LaunchPlan {
        home_mode: settings.home_mode,
        program: harness.binary().into(),
        args: options.args,
        env_set,
        env_remove: options.env_remove,
        trace: ResolutionTrace {
            plugins,
            skills,
            profiles: profile_trace,
            harness: format!("{} ({harness_source})", harness.binary()),
            model: model.map_or_else(
                || "none".into(),
                |(value, source)| format!("{value} ({source})"),
            ),
            effort: effort.map_or_else(
                || "none".into(),
                |(value, source)| format!("{} ({source})", value.as_str()),
            ),
        },
        plugin_paths,
        native_plugins: enabled
            .iter()
            .filter(|(_, enabled)| **enabled)
            .map(|(id, _)| (*id).clone())
            .collect(),
        generated_skills,
        mcp: resolved_mcp,
        diagnostics,
    })
}

/// Whether a Copilot native Plugin ID names a marketplace install.
pub fn valid_copilot_native_id(id: &str) -> bool {
    id.split_once('@').is_some_and(|(name, marketplace)| {
        marketplace != "_direct"
            && [name, marketplace].iter().all(|part| {
                !part.is_empty()
                    && !matches!(*part, "." | "..")
                    && !part.contains(['/', '\\', ':', '@'])
            })
    })
}

// The merged graph has already been checked for cycles. A shared Profile is
// expanded once; explicit Plugin selections are applied after this step.
fn expand_profile<'a>(
    name: &'a str,
    layers: &'a ConfigLayers,
    selections: &mut ProfileSelections<'a>,
    expanded: &mut std::collections::BTreeSet<&'a str>,
    default_profile: Option<&'a str>,
    trace: &mut Vec<String>,
    disable: &impl Fn(&str) -> Option<String>,
) -> Result<Vec<Diagnostic>, Vec<Diagnostic>> {
    if let Some(reason) = disable(name) {
        let message = format!("Profile {name}: {reason}");
        if !trace.contains(&message) {
            trace.push(message);
        }
        return Ok(Vec::new());
    }
    let profile = layers
        .profiles
        .get(name)
        .ok_or_else(|| unknown_profile(name))?;
    if !expanded.insert(name) {
        return Ok(Vec::new());
    }
    let dangling_member = |kind: &str, member: &str| {
        let diagnostic = Diagnostic {
            code: "dangling-ref",
            severity: if default_profile.is_some() {
                Severity::Note
            } else {
                Severity::Error
            },
            message: format!("Profile {name} references undefined {kind} {member}"),
            hint: None,
            layer: Some(profile.path.display().to_string()),
            ..Diagnostic::default()
        };
        if default_profile.is_some() {
            Ok(diagnostic)
        } else {
            Err(vec![diagnostic])
        }
    };
    let mut diagnostics = Vec::new();
    for plugin in &profile.value.plugins {
        if !layers.plugins.contains_key(plugin) {
            diagnostics.push(dangling_member("Plugin", plugin)?);
            continue;
        }
        selections.plugins.insert(
            plugin,
            default_profile.map_or(
                CapabilityOrigin::Profile(name),
                CapabilityOrigin::DefaultProfile,
            ),
        );
    }
    for skill in &profile.value.skills {
        if !layers.skills.contains_key(skill) {
            diagnostics.push(dangling_member("Skill", skill)?);
            continue;
        }
        selections.skills.insert(
            skill,
            default_profile.map_or(
                CapabilityOrigin::Profile(name),
                CapabilityOrigin::DefaultProfile,
            ),
        );
    }
    for server in &profile.value.mcp {
        if !layers.mcp.contains_key(server) {
            diagnostics.push(dangling_member("MCP server", server)?);
            continue;
        }
        selections.mcp.insert(
            server,
            default_profile.map_or(
                CapabilityOrigin::Profile(name),
                CapabilityOrigin::DefaultProfile,
            ),
        );
    }
    for member in &profile.value.profiles {
        if !layers.profiles.contains_key(member) && disable(member).is_none() {
            diagnostics.push(dangling_member("Profile", member)?);
            continue;
        }
        diagnostics.extend(expand_profile(
            member,
            layers,
            selections,
            expanded,
            default_profile,
            trace,
            disable,
        )?);
    }
    Ok(diagnostics)
}

fn unknown_profile(name: &str) -> Vec<Diagnostic> {
    vec![Diagnostic::error(
        "unknown-profile",
        format!("unknown Profile {name}"),
        None,
    )]
}

#[derive(Default)]
pub(crate) struct ProfileSelections<'a> {
    pub plugins: std::collections::BTreeMap<&'a String, CapabilityOrigin<'a>>,
    pub skills: std::collections::BTreeMap<&'a String, CapabilityOrigin<'a>>,
    pub mcp: std::collections::BTreeMap<&'a String, CapabilityOrigin<'a>>,
}

pub(crate) fn profile_selections<'a>(
    request: &'a Request,
    layers: &'a ConfigLayers,
) -> Result<(ProfileSelections<'a>, Vec<String>, Vec<Diagnostic>), Vec<Diagnostic>> {
    layers
        .validate_profiles()
        .map_err(|diagnostic| vec![diagnostic])?;
    let alias = request.alias.as_ref().and_then(|name| {
        layers
            .aliases
            .get(name)
            .map(|definition| (name, definition))
    });
    let mut selections = ProfileSelections::default();
    let mut diagnostics = Vec::new();
    let mut selected_profiles: Vec<_> = request.profiles.iter().collect();
    if let Some((_, definition)) = alias {
        selected_profiles.extend(&definition.profiles);
    }
    // Explicit names remain errors even when a Disable prevents expansion.
    for name in &selected_profiles {
        if !layers.profiles.contains_key(name.as_str()) {
            return Err(unknown_profile(name));
        }
    }
    let mut profile_trace = Vec::new();
    let mut expanded = std::collections::BTreeSet::new();
    let profile_disable = |name: &str| {
        if request.no_profiles.iter().any(|disabled| disabled == name) {
            return Some("disabled by --no-profile".to_owned());
        }
        if selected_profiles
            .iter()
            .any(|selected| selected.as_str() == name)
        {
            return None;
        }
        if let Some(path) = layers.disabled_profiles.get(name) {
            return Some(format!("disabled by layer {}", path.display()));
        }
        if let Some((alias_name, definition)) = alias
            && definition
                .disabled_profiles
                .iter()
                .any(|disabled| disabled == name)
        {
            return Some(format!("disabled by Alias {alias_name}"));
        }
        None
    };
    if !request.no_defaults && alias.is_none_or(|(_, definition)| definition.defaults) {
        for (name, profile) in &layers.profiles {
            if profile.value.default && !layers.disabled_default_profiles.contains_key(name) {
                diagnostics.extend(expand_profile(
                    name,
                    layers,
                    &mut selections,
                    &mut expanded,
                    Some(name),
                    &mut profile_trace,
                    &profile_disable,
                )?);
            }
        }
    }
    // Explicit Profile routes must replace Default origins even for a shared
    // nested Profile that was already expanded through a Default.
    expanded.clear();
    for name in &selected_profiles {
        diagnostics.extend(expand_profile(
            name,
            layers,
            &mut selections,
            &mut expanded,
            None,
            &mut profile_trace,
            &profile_disable,
        )?);
    }
    Ok((selections, profile_trace, diagnostics))
}
