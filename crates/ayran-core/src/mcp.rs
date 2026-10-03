//! MCP Bindings, validation and pure per-Session definition generation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::config::{ConfigLayers, Sourced, invalid, parse_bool, parse_names};
use crate::diagnostic::{Diagnostic, Severity};
use crate::harness::Harness;
use crate::resolve::{CapabilityOrigin, Request};

pub enum McpBinding {
    Native(String),
    Connector(String),
    Definition(McpDefinition),
    Absent,
}

pub enum McpDefinition {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        env_vars: Vec<String>,
    },
    Http {
        url: String,
        headers: BTreeMap<String, String>,
        env_headers: BTreeMap<String, String>,
        bearer_token_env: Option<String>,
    },
}

#[derive(Default)]
pub struct McpServer {
    pub all: Option<McpBinding>,
    pub claude: Option<McpBinding>,
    pub codex: Option<McpBinding>,
    pub copilot: Option<McpBinding>,
    pub description: Option<String>,
    pub default: bool,
}

impl McpServer {
    pub fn binding(&self, harness: Harness) -> Option<&McpBinding> {
        match harness {
            Harness::Claude => self.claude.as_ref(),
            Harness::Codex => self.codex.as_ref(),
            Harness::Copilot => self.copilot.as_ref(),
        }
        .or(self.all.as_ref())
    }
}

pub(crate) fn parse(
    value: &toml::Value,
    path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
    collect: bool,
) -> Result<BTreeMap<String, Sourced<McpServer>>, Diagnostic> {
    let servers = value
        .as_table()
        .ok_or_else(|| invalid(path, "mcp must be a table"))?;
    let mut result = BTreeMap::new();
    for (name, value) in servers {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            crate::config::report(
                Diagnostic::error(
                    "invalid-name",
                    format!("{}: invalid MCP server name {name:?}", path.display()),
                    None,
                ),
                diagnostics,
                collect,
            )?;
        }
        let fields = value
            .as_table()
            .ok_or_else(|| invalid(path, format!("mcp.{name} must be a table")))?;
        let mut server = McpServer::default();
        for (field, value) in fields {
            let key = format!("mcp.{name}.{field}");
            match field.as_str() {
                "all" | "claude" | "codex" | "copilot" => {
                    let binding = parse_binding(value, path, &key, diagnostics, collect)?;
                    let slot = match field.as_str() {
                        "all" => &mut server.all,
                        "claude" => &mut server.claude,
                        "codex" => &mut server.codex,
                        "copilot" => &mut server.copilot,
                        _ => unreachable!(),
                    };
                    *slot = Some(binding);
                }
                "default" => server.default = parse_bool(value, path, &key)?,
                "description" => server.description = Some(string(value, path, &key)?),
                _ => return Err(invalid(path, format!("unknown key {key}"))),
            }
        }
        result.insert(
            name.clone(),
            Sourced {
                value: server,
                path: path.to_path_buf(),
            },
        );
    }
    Ok(result)
}

fn string(value: &toml::Value, path: &Path, key: &str) -> Result<String, Diagnostic> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid(path, format!("{key} must be a string")))
}

fn literal(
    value: &str,
    path: &Path,
    key: &str,
    diagnostics: &mut Vec<Diagnostic>,
    collect: bool,
) -> Result<(), Diagnostic> {
    if value.contains("${") {
        return crate::config::report(
            Diagnostic::error(
                "literal-interpolation",
                format!("{}: {key} cannot contain '${{'", path.display()),
                None,
            ),
            diagnostics,
            collect,
        );
    }
    Ok(())
}

fn strings(
    value: &toml::Value,
    path: &Path,
    key: &str,
    literals: bool,
    diagnostics: &mut Vec<Diagnostic>,
    collect: bool,
) -> Result<BTreeMap<String, String>, Diagnostic> {
    let fields = value
        .as_table()
        .ok_or_else(|| invalid(path, format!("{key} must be a table of strings")))?;
    fields
        .iter()
        .map(|(name, value)| {
            let value = string(value, path, key)?;
            if literals {
                literal(&value, path, key, diagnostics, collect)?;
            }
            Ok((name.clone(), value))
        })
        .collect()
}

fn parse_binding(
    value: &toml::Value,
    path: &Path,
    key: &str,
    diagnostics: &mut Vec<Diagnostic>,
    collect: bool,
) -> Result<McpBinding, Diagnostic> {
    match value {
        toml::Value::String(name) => return Ok(McpBinding::Native(name.clone())),
        toml::Value::Boolean(false) => return Ok(McpBinding::Absent),
        _ => {}
    }
    let fields = value.as_table().ok_or_else(|| {
        invalid(
            path,
            format!("{key} must be a native string, definition table, or false"),
        )
    })?;
    if let Some(id) = fields.get("connector") {
        if fields.len() != 1 {
            return Err(invalid(
                path,
                format!("{key}: connector Binding accepts only connector"),
            ));
        }
        return Ok(McpBinding::Connector(string(
            id,
            path,
            &format!("{key}.connector"),
        )?));
    }
    let stdio = fields.contains_key("command");
    if stdio == fields.contains_key("url") {
        return Err(invalid(
            path,
            format!("{key} requires exactly one of command or url"),
        ));
    }
    let allowed = if stdio {
        &["command", "args", "env", "env_vars"][..]
    } else {
        &["url", "headers", "env_headers", "bearer_token_env"][..]
    };
    for field in fields.keys() {
        if !allowed.contains(&field.as_str()) {
            return Err(invalid(path, format!("unknown key {key}.{field}")));
        }
    }
    let definition = if stdio {
        let mut command = string(&fields["command"], path, key)?;
        literal(&command, path, key, diagnostics, collect)?;
        if command.contains(['/', '\\']) || Path::new(&command).is_absolute() {
            let mut resolved = PathBuf::from(&command);
            if !resolved.is_absolute() {
                resolved = path.parent().unwrap_or(Path::new(".")).join(resolved);
            }
            if !resolved.is_absolute() {
                resolved = std::env::current_dir()
                    .map_err(|e| invalid(path, e))?
                    .join(resolved);
            }
            command = resolved
                .components()
                .collect::<PathBuf>()
                .to_string_lossy()
                .into_owned();
        }
        let args = fields
            .get("args")
            .map(|v| parse_names(v, path, key))
            .transpose()?
            .unwrap_or_default();
        for arg in &args {
            literal(arg, path, key, diagnostics, collect)?;
        }
        McpDefinition::Stdio {
            command,
            args,
            env: fields
                .get("env")
                .map(|v| strings(v, path, key, true, diagnostics, collect))
                .transpose()?
                .unwrap_or_default(),
            env_vars: fields
                .get("env_vars")
                .map(|v| parse_names(v, path, key))
                .transpose()?
                .unwrap_or_default(),
        }
    } else {
        let url = string(&fields["url"], path, key)?;
        literal(&url, path, key, diagnostics, collect)?;
        McpDefinition::Http {
            url,
            headers: fields
                .get("headers")
                .map(|v| strings(v, path, key, true, diagnostics, collect))
                .transpose()?
                .unwrap_or_default(),
            env_headers: fields
                .get("env_headers")
                .map(|v| strings(v, path, key, false, diagnostics, collect))
                .transpose()?
                .unwrap_or_default(),
            bearer_token_env: fields
                .get("bearer_token_env")
                .map(|v| string(v, path, key))
                .transpose()?,
        }
    };
    Ok(McpBinding::Definition(definition))
}

#[derive(Default)]
pub struct McpState {
    /// User and user-private local servers active for this working directory.
    pub user: BTreeSet<String>,
    /// Proven effective Codex user/profile values.
    pub codex_enabled: BTreeMap<String, bool>,
    /// Claude names proven off by native settings or per-project toggles.
    pub claude_off: BTreeSet<String>,
    /// Copilot user settings disables, which can prove a hide redundant.
    pub copilot_user_off: BTreeSet<String>,
    /// Copilot repository disables, read conservatively regardless of trust.
    pub copilot_project_off: BTreeSet<String>,
    /// Project servers, including those awaiting trust approval.
    pub project: BTreeSet<String>,
    pub disabled: BTreeSet<String>,
    /// Account connector names remembered by the Harness, not a complete inventory.
    pub connectors: BTreeSet<String>,
    /// Codex apps explicitly enabled in the user config.
    pub enabled_apps: BTreeSet<String>,
    /// Bare MCP server names shipped by surviving Plugin selections.
    pub plugins: BTreeSet<String>,
}

#[derive(Default)]
pub struct ResolvedMcp {
    pub config: Option<serde_json::Value>,
    pub overrides: Vec<String>,
    pub args: Vec<String>,
    pub command_paths: Vec<PathBuf>,
    pub trace: Vec<String>,
    pub denied: Vec<String>,
    pub connectors: BTreeSet<String>,
    pub diagnostics: Vec<Diagnostic>,
}

pub(crate) fn resolve<'a>(
    request: &'a Request,
    layers: &'a ConfigLayers,
    harness: Harness,
    state: &McpState,
    profiles: BTreeMap<&'a String, CapabilityOrigin<'a>>,
) -> Result<ResolvedMcp, Vec<Diagnostic>> {
    let mut result = ResolvedMcp::default();
    let mut definitions = BTreeMap::new();
    let mut selected_native = BTreeSet::new();
    let alias = request
        .alias
        .as_ref()
        .and_then(|name| layers.aliases.get(name).map(|value| (name, value)));
    let mut selected = BTreeMap::new();
    for (name, server) in &layers.mcp {
        if server.value.default {
            selected.insert(name, CapabilityOrigin::Default);
        }
    }
    selected.extend(profiles);
    if let Some((name, definition)) = alias {
        for server in &definition.mcp {
            selected.insert(server, CapabilityOrigin::Alias(name));
        }
    }
    for name in &request.mcp {
        selected.insert(name, CapabilityOrigin::Cli("--mcp"));
    }
    for (name, origin) in &selected {
        if origin.is_direct_selection() && !layers.mcp.contains_key(*name) {
            return Err(vec![Diagnostic::error(
                "unknown-mcp",
                format!("unknown MCP server {name}"),
                None,
            )]);
        }
    }
    let considered: BTreeSet<_> = selected.keys().copied().collect();
    for name in layers.mcp.keys().filter(|name| !considered.contains(name)) {
        let reason = if request.no_mcp.contains(name) {
            "disabled by --no-mcp"
        } else {
            "unselected"
        };
        result.trace.push(format!("MCP server {name}: {reason}"));
    }
    for (name, origin) in selected {
        let reason = if request.no_mcp.contains(name) {
            Some("disabled by --no-mcp".to_owned())
        } else if matches!(origin, CapabilityOrigin::Default) && request.no_defaults {
            Some("disabled by --no-defaults".to_owned())
        } else if matches!(origin, CapabilityOrigin::Default)
            && alias.is_some_and(|(_, value)| !value.defaults)
        {
            Some(format!(
                "disabled by Alias {} (defaults = false)",
                alias.unwrap().0
            ))
        } else if matches!(origin, CapabilityOrigin::Default)
            && layers.disabled_default_mcp.contains_key(name)
        {
            Some(format!(
                "disabled by layer {} (Defaults)",
                layers.disabled_default_mcp[name].display()
            ))
        } else if !origin.is_direct_selection() && layers.disabled_mcp.contains_key(name) {
            Some(format!(
                "disabled by layer {}",
                layers.disabled_mcp[name].display()
            ))
        } else if !origin.is_direct_selection()
            && alias.is_some_and(|(_, value)| value.disabled_mcp.contains(name))
        {
            Some(format!("disabled by Alias {}", alias.unwrap().0))
        } else {
            None
        };
        if let Some(reason) = reason {
            result
                .trace
                .push(format!("MCP server {name}: {reason} ({origin})"));
            continue;
        }
        let server = layers.mcp.get(name).ok_or_else(|| {
            vec![Diagnostic::error(
                "unknown-mcp",
                format!("unknown MCP server {name}"),
                None,
            )]
        })?;
        let binding = server.value.binding(harness).ok_or_else(|| {
            vec![
                Diagnostic::error(
                    "missing-binding",
                    format!("MCP server {name} has no Binding for {}", harness.binary()),
                    None,
                )
                .for_capability(
                    harness,
                    crate::diagnostic::CapabilityKind::Mcp,
                    name,
                    &server.path,
                ),
            ]
        })?;
        let detail = match binding {
            McpBinding::Absent if !origin.is_direct_selection() => {
                let message = format!(
                    "MCP server {name}: skipped because of a false Binding for {} ({origin}, {})",
                    harness.binary(),
                    server.path.display()
                );
                result.trace.push(message.clone());
                result.diagnostics.push(
                    Diagnostic {
                        code: "binding-skipped",
                        severity: Severity::Note,
                        message,
                        hint: None,
                        layer: Some(server.path.display().to_string()),
                        ..Diagnostic::default()
                    }
                    .for_capability(
                        harness,
                        crate::diagnostic::CapabilityKind::Mcp,
                        name,
                        &server.path,
                    ),
                );
                continue;
            }
            McpBinding::Absent => {
                return Err(vec![
                    Diagnostic::error(
                        "binding-absent",
                        format!(
                            "MCP server {name} is deliberately absent on {}",
                            harness.binary()
                        ),
                        None,
                    )
                    .for_capability(
                        harness,
                        crate::diagnostic::CapabilityKind::Mcp,
                        name,
                        &server.path,
                    ),
                ]);
            }
            McpBinding::Connector(id) => {
                if !matches!(harness, Harness::Claude | Harness::Codex)
                    || match harness {
                        Harness::Claude => server.value.claude.is_none(),
                        Harness::Codex => server.value.codex.is_none(),
                        Harness::Copilot => true,
                    }
                {
                    return Err(vec![Diagnostic::error(
                        "unsupported-binding",
                        format!("MCP server {name}: connector Binding requires a claude or codex Binding"),
                        None,
                    ).for_capability(harness, crate::diagnostic::CapabilityKind::Mcp, name, &server.path)]);
                }
                if result.connectors.insert(id.clone())
                    && harness == Harness::Claude
                    && state.disabled.contains(id)
                {
                    result.diagnostics.push(Diagnostic {
                        code: "mcp-disabled",
                        severity: Severity::Note,
                        message: format!("Claude Account connector {id} is disabled for this project"),
                        hint: Some("enable it with /mcp in Claude; no per-launch setting overrides this toggle".into()),
                        ..Diagnostic::default()
                    }.for_capability(harness, crate::diagnostic::CapabilityKind::Mcp, name, &server.path).with_item(id));
                }
                format!("connector {id}")
            }
            McpBinding::Native(native) => {
                let account_connector = harness == Harness::Codex && native == "codex_apps";
                if !account_connector
                    && !state.user.contains(native)
                    && !state.project.contains(native)
                {
                    return Err(vec![Diagnostic::error(
                        "native-not-found",
                        format!(
                            "{} MCP server {native} bound by {name} was not found",
                            harness.binary()
                        ),
                        Some(
                            if harness == Harness::Claude && native.starts_with("claude.ai") {
                                "Account connectors require a connector Binding: claude = { connector = \"<server name>\" }"
                            } else {
                                "bind a configured user, local, or project MCP server"
                            },
                        ),
                    ).for_capability(harness, crate::diagnostic::CapabilityKind::Mcp, name, &server.path).with_item(native)]);
                }
                let newly_selected = selected_native.insert(native.clone());
                if newly_selected && harness == Harness::Codex && !account_connector {
                    if state.codex_enabled.get(native) == Some(&true) {
                        result.trace.push(format!(
                            "MCP server {native}: natively on (no override needed)"
                        ));
                    } else {
                        result
                            .overrides
                            .push(codex_enabled("mcp_servers", native, true));
                    }
                } else if newly_selected && harness == Harness::Copilot {
                    if state.copilot_user_off.contains(native)
                        || state.copilot_project_off.contains(native)
                    {
                        result
                            .args
                            .extend(["--enable-mcp-server".into(), native.clone()]);
                    } else {
                        result.trace.push(format!(
                            "MCP server {native}: natively on (no override needed)"
                        ));
                    }
                } else if newly_selected
                    && harness == Harness::Claude
                    && state.disabled.contains(native)
                {
                    result.diagnostics.push(Diagnostic {
                            code: "mcp-disabled",
                            severity: Severity::Note,
                            message: format!("Claude MCP server {native} is disabled for this project"),
                            hint: Some("enable it with /mcp in Claude; no per-launch setting overrides this toggle".into()),
                            layer: None,
                            ..Diagnostic::default()
                        }.for_capability(harness, crate::diagnostic::CapabilityKind::Mcp, name, &server.path).with_item(native));
                }
                format!("native {native}")
            }
            McpBinding::Definition(definition) => {
                if harness == Harness::Codex
                    && (state.user.contains(name) || state.project.contains(name))
                {
                    return Err(vec![Diagnostic::error(
                        "mcp-definition-collision",
                        format!(
                            "MCP definition {name} would merge with a configured Codex MCP server"
                        ),
                        Some(&format!(
                            "use codex = \"{name}\" to select the configured server"
                        )),
                    ).for_capability(harness, crate::diagnostic::CapabilityKind::Mcp, name, &server.path).with_item(name)]);
                }
                if matches!(harness, Harness::Codex | Harness::Copilot)
                    && state.plugins.contains(name)
                {
                    result.diagnostics.push(Diagnostic {
                        code: "plugin-server-shadowed",
                        severity: Severity::Warning,
                        message: format!(
                            "MCP definition {name} overrides a selected Plugin's MCP server"
                        ),
                        hint: None,
                        layer: Some(server.path.display().to_string()),
                        harness: Some(harness),
                        capability: Some(Box::new(crate::diagnostic::Capability {
                            kind: crate::diagnostic::CapabilityKind::Mcp,
                            name: name.clone(),
                        })),
                        item: Some(Box::new(name.clone())),
                        ..Diagnostic::default()
                    });
                }
                let value = match definition {
                    McpDefinition::Stdio {
                        command,
                        args,
                        env,
                        env_vars,
                    } => {
                        if Path::new(command).is_absolute() {
                            result.command_paths.push(command.into());
                        }
                        if harness == Harness::Codex {
                            serde_json::json!({"command": command, "args": args, "env": env, "env_vars": env_vars})
                        } else {
                            let mut env = env.clone();
                            for variable in env_vars {
                                env.insert(variable.clone(), format!("${{{variable}}}"));
                            }
                            let mut value =
                                serde_json::json!({"command": command, "args": args, "env": env});
                            if harness == Harness::Copilot {
                                value["type"] = "local".into();
                                value["tools"] = serde_json::json!(["*"]);
                            }
                            value
                        }
                    }
                    McpDefinition::Http {
                        url,
                        headers,
                        env_headers,
                        bearer_token_env,
                    } => {
                        if harness == Harness::Codex {
                            let mut value = serde_json::json!({"url": url, "http_headers": headers, "env_http_headers": env_headers});
                            if let Some(variable) = bearer_token_env {
                                value["bearer_token_env_var"] = variable.clone().into();
                            }
                            value
                        } else {
                            let mut headers = headers.clone();
                            for (header, variable) in env_headers {
                                headers.insert(header.clone(), format!("${{{variable}}}"));
                            }
                            if let Some(variable) = bearer_token_env {
                                headers.insert(
                                    "Authorization".into(),
                                    format!("Bearer ${{{variable}}}"),
                                );
                            }
                            let mut value =
                                serde_json::json!({"type": "http", "url": url, "headers": headers});
                            if harness == Harness::Copilot {
                                value["tools"] = serde_json::json!(["*"]);
                            }
                            value
                        }
                    }
                };
                if harness == Harness::Codex {
                    let table = toml::Value::try_from(&value)
                        .expect("MCP definition fields are TOML-compatible");
                    result.overrides.push(format!("mcp_servers.{name}={table}"));
                } else {
                    definitions.insert(name.clone(), value);
                }
                "definition".into()
            }
        };
        result.trace.push(format!(
            "MCP server {name}: {detail} ({origin}; layer {})",
            server.path.display()
        ));
    }
    if harness == Harness::Codex && !selected_native.contains("codex_apps") {
        if result.connectors.is_empty() {
            result.args.extend(["--disable".into(), "apps".into()]);
        } else {
            result.overrides.push("apps._default.enabled=false".into());
            for id in state.enabled_apps.difference(&result.connectors) {
                if id != "_default" {
                    result.overrides.push(codex_enabled("apps", id, false));
                }
            }
            for id in &result.connectors {
                result.overrides.push(codex_enabled("apps", id, true));
            }
        }
    }
    let trailing_args = request.trailing_args(layers, harness);
    if matches!(harness, Harness::Codex | Harness::Copilot) {
        for native in state.user.difference(&selected_native) {
            if harness == Harness::Copilot
                && (native == "github-mcp-server" || definitions.contains_key(native))
            {
                continue;
            }
            if state.project.contains(native) {
                result.diagnostics.push(Diagnostic {
                    code: "leak",
                    severity: Severity::Warning,
                    message: format!("mcp-project-shadow: user MCP server {native} stays visible because disabling it would also hide a project server"),
                    hint: None,
                    layer: None,
                    harness: Some(harness),
                cause: Some(Box::new("mcp-project-shadow".into())),
                item: Some(Box::new(native.clone())),
                ..Diagnostic::default()
                });
                result.trace.push(format!(
                    "MCP server {native}: left visible (mcp-project-shadow)"
                ));
                continue;
            }
            if state.plugins.contains(native) {
                result.diagnostics.push(Diagnostic {
                    code: "leak",
                    severity: Severity::Warning,
                    message: format!("plugin-server-shadow: user MCP server {native} stays visible because disabling it would also hide a selected Plugin's server"),
                    hint: None,
                    layer: None,
                    harness: Some(harness),
                cause: Some(Box::new("plugin-server-shadow".into())),
                item: Some(Box::new(native.clone())),
                ..Diagnostic::default()
                });
                result.trace.push(format!(
                    "MCP server {native}: left visible (plugin-server-shadow)"
                ));
                continue;
            }
            let copilot_off = harness == Harness::Copilot
                && state.copilot_user_off.contains(native)
                && !trailing_args.iter().enumerate().any(|(index, arg)| {
                    arg.to_str() == Some(format!("--enable-mcp-server={native}").as_str())
                        || (arg == "--enable-mcp-server"
                            && trailing_args.get(index + 1).and_then(|arg| arg.to_str())
                                == Some(native.as_str()))
                });
            if (harness == Harness::Codex && state.codex_enabled.get(native) == Some(&false))
                || copilot_off
            {
                result.trace.push(format!(
                    "MCP server {native}: hidden (natively off, no override needed)"
                ));
                continue;
            }
            if harness == Harness::Codex {
                result
                    .overrides
                    .push(codex_enabled("mcp_servers", native, false));
            } else {
                result
                    .args
                    .extend(["--disable-mcp-server".into(), native.clone()]);
            }
            result.trace.push(format!(
                "MCP server {native}: hidden (unselected user server)"
            ));
        }
    }
    if harness == Harness::Claude {
        if !result.connectors.is_empty() {
            result
                .diagnostics
                .push(claude_connector_leak(Severity::Warning));
            for id in state.connectors.difference(&result.connectors) {
                result.denied.push(id.clone());
                result.trace.push(format!(
                    "Account connector {id}: hidden (unselected cached connector)"
                ));
            }
        }
        let mut project_shadows = Vec::new();
        for native in &state.user {
            if selected_native.contains(native)
                || result.connectors.contains(native)
                || definitions.contains_key(native)
            {
                continue;
            }
            if state.project.contains(native) {
                project_shadows.push(native.as_str());
                result.trace.push(format!(
                    "MCP server {native}: left visible (mcp-project-shadow)"
                ));
            } else {
                if state.claude_off.contains(native) {
                    result.trace.push(format!(
                        "MCP server {native}: hidden (natively off, no override needed)"
                    ));
                } else {
                    result.denied.push(native.clone());
                    result.trace.push(format!(
                        "MCP server {native}: hidden (unselected user/local server)"
                    ));
                }
            }
        }
        if !project_shadows.is_empty() {
            result.diagnostics.push(Diagnostic {
                code: "leak",
                severity: Severity::Warning,
                message: format!("mcp-project-shadow: user/local MCP servers {} stay visible because a deny would also hide project servers", project_shadows.join(", ")),
                hint: None,
                layer: None,
                harness: Some(harness),
                cause: Some(Box::new("mcp-project-shadow".into())),
                item: Some(Box::new(project_shadows.join(", "))),
                ..Diagnostic::default()
            });
        }
    }
    if !definitions.is_empty() {
        result.config = Some(serde_json::json!({"mcpServers": definitions}));
    }
    Ok(result)
}

fn codex_enabled(table: &str, name: &str, enabled: bool) -> String {
    // Codex splits CLI override keys on dots; TOML quoting only works in values.
    if name.contains('.') {
        let key = toml::Value::String(name.into());
        format!("{table}={{{key}={{enabled={enabled}}}}}")
    } else {
        format!("{table}.{name}.enabled={enabled}")
    }
}

pub(crate) fn claude_connector_leak(severity: Severity) -> Diagnostic {
    Diagnostic {
        code: "leak",
        severity,
        message: "claude-connector: Account connectors that have never connected may stay visible because they cannot be denied by name".into(),
        harness: Some(Harness::Claude),
        cause: Some(Box::new("claude-connector".into())),
        ..Diagnostic::default()
    }
}
