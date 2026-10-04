//! Explicit MCP installation: preflight all targets, retain add/off progress, never replace.
use crate::{
    harness_home::HarnessHome,
    install_plan::{Outcome, Plan},
    native_plugins::NativeState,
};
use ayran_core::{
    config::ConfigLayers,
    diagnostic::{Diagnostic, Severity},
    doctor::HARNESSES,
    harness::Harness,
    mcp::{McpBinding, McpDefinition, same_definition},
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Step {
    Planned,
    Added,
    Changed,
    Unchanged,
    Skipped,
    Failed,
    Unsupported,
}
impl std::fmt::Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Planned => "planned",
            Self::Added => "added",
            Self::Changed => "changed",
            Self::Unchanged => "unchanged",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
            Self::Unsupported => "unsupported",
        })
    }
}

pub struct Change {
    pub harness: Harness,
    pub logical: String,
    pub id: String,
    pub outcome: Outcome,
    pub add: Step,
    pub disable: Step,
    definition: Option<Value>,
    args: Option<Vec<String>>,
}
impl Change {
    pub fn json(&self) -> Value {
        serde_json::json!({"harness":self.harness,"logical":self.logical,"id":self.id,"outcome":self.outcome,"add":self.add,"disable":self.disable})
    }
}
fn error(h: Harness, code: &'static str, message: impl std::fmt::Display) -> Diagnostic {
    Diagnostic {
        harness: Some(h),
        ..Diagnostic::error(code, message.to_string(), None)
    }
}
fn note(h: Harness, code: &'static str, message: impl std::fmt::Display) -> Diagnostic {
    Diagnostic {
        severity: Severity::Note,
        ..error(h, code, message)
    }
}
pub fn plan(layers: &ConfigLayers, names: &[String], selected: Option<Harness>, plan: &mut Plan) {
    if names.is_empty() {
        return;
    }
    for name in names {
        if !layers.mcp.contains_key(name) {
            plan.diagnostics.push(Diagnostic::error(
                "unknown-mcp",
                format!("unknown MCP server {name}"),
                None,
            ));
        }
    }
    for harness in HARNESSES
        .into_iter()
        .filter(|h| selected.is_none_or(|s| s == *h))
    {
        let Some(binary) = crate::program_on_path(harness.binary().as_ref()) else {
            plan.diagnostics.push(note(
                harness,
                "harness-not-found",
                "Harness is not installed; skipped",
            ));
            continue;
        };
        let result = (|| {
            let version = crate::harness_version::check(harness.binary().as_ref(), &binary, false)?;
            let minimum = match harness {
                Harness::Claude => "2.1.288",
                Harness::Codex => "0.160.0",
                Harness::Copilot => "1.0.91",
            };
            if semver::Version::parse(&version).unwrap() < semver::Version::parse(minimum).unwrap()
            {
                return Err(error(
                    harness,
                    "harness-too-old",
                    format!("MCP install requires {minimum} or newer; found {version}"),
                ));
            }
            let home = crate::native_plugins::home(layers, harness)?;
            let real_home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from);
            let state = match harness {
                Harness::Claude => crate::mcp_enumeration::read_claude(&home)?,
                Harness::Codex => crate::mcp_enumeration::read_codex(&home, real_home.as_deref())?,
                Harness::Copilot => crate::copilot_mcp_enumeration::read(&home)?,
            };
            let installed = crate::plugin_enumeration::read(harness, real_home.as_deref(), &home)?;
            let inventory = crate::plugin_contents::read(harness, &home, &installed)?;
            let user = crate::mcp_enumeration::user_definitions(harness, &home)?;
            for name in names.iter().collect::<BTreeSet<_>>() {
                let Some(server) = layers.mcp.get(name) else {
                    continue;
                };
                let mut change = Change {
                    harness,
                    logical: name.clone(),
                    id: name.clone(),
                    outcome: Outcome::Skipped,
                    add: Step::Skipped,
                    disable: Step::Skipped,
                    definition: None,
                    args: None,
                };
                match server.value.binding(harness) {
                    Some(McpBinding::Native(id)) => {
                        change.id = id.clone();
                        if state.user.contains(id) || state.project.contains(id) {
                            change.outcome = Outcome::Unchanged;
                            change.add = Step::Unchanged;
                        } else {
                            plan.diagnostics.push(error(
                                harness,
                                "native-not-found",
                                format!("MCP server {id} bound by {name} is not configured"),
                            ));
                        }
                    }
                    Some(McpBinding::Definition(d)) => {
                        let value = d.native_value(harness);
                        let args = add_args(harness, name, d)?;
                        if let McpDefinition::Stdio { command, .. } = d
                            && Path::new(command).is_absolute()
                            && !Path::new(command).is_file()
                        {
                            return Err(error(
                                harness,
                                "path-not-found",
                                format!("MCP command {command} is not a file"),
                            ));
                        }
                        if state.project.contains(name)
                            || state.sources.contains(&(name.clone(), "local"))
                            || inventory.plugins.iter().any(|p| p.servers.contains_key(name))
                        {
                            plan.diagnostics.push(error(
                                harness,
                                "mcp-install-conflict",
                                format!("MCP server {name} collides with a project, local or Plugin definition"),
                            ));
                        }
                        if let Some(actual) = user.get(name)
                            && !same_definition(actual, &value, harness)
                        {
                            plan.diagnostics.push(error(
                                harness,
                                "mcp-install-conflict",
                                format!("MCP server {name} has a different user definition; refusing replacement"),
                            ));
                        }
                        let project = server.path.file_name().is_none_or(|n| n != "ayran.local.toml")
                            && !crate::skill_enumeration::same_root(
                                &server.path, &ayran_core::config::user_config_path()?,
                            );
                        if project && !crate::trust::Store::read()?.trusted(&server.path)? {
                            plan.diagnostics.push(Diagnostic {
                                layer: Some(server.path.display().to_string()),
                                hint: Some(format!("ayran trust {}", crate::shell_quote(server.path.as_os_str()))),
                                ..error(harness, "untrusted-layer", format!("MCP definition {name} requires Trust for its project layer"))
                            });
                        }
                        let off = harness != Harness::Claude;
                        let state = if off {
                            crate::native_mcp::user_state(harness, &home, name)?
                        } else { NativeState::On };
                        change.add = if user.contains_key(name) { Step::Unchanged } else { Step::Planned };
                        change.disable = if !off {
                            Step::Unsupported
                        } else if state == NativeState::Off && user.contains_key(name) {
                            Step::Unchanged
                        } else { Step::Planned };
                        change.outcome = if change.add == Step::Unchanged && change.disable != Step::Planned {
                            Outcome::Unchanged
                        } else { Outcome::Planned };
                        change.definition = Some(value);
                        change.args = args;
                        if !off {
                            plan.diagnostics.push(note(
                                harness,
                                "mcp-install-on",
                                format!("MCP server {name} remains natively on: Claude has no off state that Session selection can override"),
                            ));
                        }
                    }
                    _ => plan.diagnostics.push(note(
                        harness,
                        "install-skipped",
                        format!("MCP server {name}: missing, absent or Account connector Binding; skipped"),
                    )),
                }
                plan.mcp.push(change);
            }
            Ok::<_, Diagnostic>(())
        })();
        if let Err(d) = result {
            plan.diagnostics.push(d);
        }
    }
}

fn add_args(h: Harness, name: &str, d: &McpDefinition) -> Result<Option<Vec<String>>, Diagnostic> {
    fn contains_nul(value: &Value) -> bool {
        match value {
            Value::String(s) => s.contains('\0'),
            Value::Array(a) => a.iter().any(contains_nul),
            Value::Object(o) => o.iter().any(|(k, v)| k.contains('\0') || contains_nul(v)),
            _ => false,
        }
    }
    if contains_nul(&d.native_value(h)) {
        return Err(error(
            h,
            "unsupported-binding",
            format!("MCP server {name} contains a NUL argument"),
        ));
    }
    let mut args = vec!["mcp".into(), "add".into()];
    if h == Harness::Claude {
        args.extend(["--scope".into(), "user".into()]);
    }
    let reference = |v: &str| -> Result<String, Diagnostic> {
        if v.is_empty()
            || !v
                .chars()
                .enumerate()
                .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || i > 0 && c.is_ascii_digit())
        {
            return Err(error(
                h,
                "unsupported-binding",
                format!("MCP server {name} has an invalid environment reference"),
            ));
        }
        Ok(format!("${{{v}}}"))
    };
    match d {
        McpDefinition::Stdio {
            command,
            args: argv,
            env,
            env_vars,
        } => {
            if env_vars.iter().any(|v| env.contains_key(v)) {
                return Err(error(
                    h,
                    "unsupported-binding",
                    format!("MCP server {name} defines both literal and referenced env values"),
                ));
            }
            for v in env_vars {
                reference(v)?;
            }
            if env.keys().any(|k| k.is_empty() || k.contains('=')) {
                return Err(error(
                    h,
                    "unsupported-binding",
                    format!("MCP server {name} has an invalid environment key"),
                ));
            }
            if h == Harness::Codex && !env_vars.is_empty() {
                return Ok(None);
            }
            if h != Harness::Codex {
                args.extend(["--transport".into(), "stdio".into()]);
            }
            for (key, value) in env {
                args.extend(["--env".into(), format!("{key}={value}")]);
            }
            for v in env_vars {
                args.extend(["--env".into(), format!("{v}={}", reference(v)?)]);
            }
            args.extend([name.into(), "--".into(), command.clone()]);
            args.extend(argv.clone());
        }
        McpDefinition::Http {
            url,
            headers,
            env_headers,
            bearer_token_env,
        } => {
            let mut header_names = BTreeSet::new();
            if headers
                .keys()
                .chain(env_headers.keys())
                .any(|k| !header_names.insert(k.to_ascii_lowercase()))
                || bearer_token_env.is_some()
                    && headers
                        .keys()
                        .chain(env_headers.keys())
                        .any(|k| k.eq_ignore_ascii_case("authorization"))
            {
                return Err(error(
                    h,
                    "unsupported-binding",
                    format!("MCP server {name} has conflicting HTTP header fields"),
                ));
            }
            if headers.iter().any(|(k, v)| {
                k.is_empty() || k.contains([':', '\n', '\r']) || v.contains(['\n', '\r'])
            }) || env_headers
                .keys()
                .any(|k| k.is_empty() || k.contains([':', '\n', '\r']))
            {
                return Err(error(
                    h,
                    "unsupported-binding",
                    format!("MCP server {name} has an unsupported HTTP header"),
                ));
            }
            for v in env_headers.values().chain(bearer_token_env.iter()) {
                reference(v)?;
            }
            if h == Harness::Codex {
                if !headers.is_empty() || !env_headers.is_empty() {
                    return Ok(None);
                }
                args.extend([name.into(), "--url".into(), url.clone()]);
                if let Some(v) = bearer_token_env {
                    args.extend(["--bearer-token-env-var".into(), v.clone()]);
                }
            } else {
                args.extend(["--transport".into(), "http".into()]);
                for (key, value) in headers {
                    args.extend(["--header".into(), format!("{key}: {value}")]);
                }
                for (key, v) in env_headers {
                    args.extend(["--header".into(), format!("{key}: {}", reference(v)?)]);
                }
                if let Some(v) = bearer_token_env {
                    args.extend([
                        "--header".into(),
                        format!("Authorization: Bearer {}", reference(v)?),
                    ]);
                }
                args.extend([name.into(), url.clone()]);
            }
        }
    }
    if args.iter().any(|a| a.contains('\0')) {
        return Err(error(
            h,
            "unsupported-binding",
            format!("MCP server {name} contains a NUL argument"),
        ));
    }
    Ok(Some(args))
}

pub fn execute(layers: &ConfigLayers, plan: &mut Plan) {
    let result = (|| {
        for change in &mut plan.mcp {
            if change.outcome != Outcome::Planned {
                continue;
            }
            change.outcome = Outcome::Failed;
            let home = crate::native_plugins::home(layers, change.harness)?;
            let value = change.definition.as_ref().unwrap();
            let user = crate::mcp_enumeration::user_definitions(change.harness, &home)?;
            if let Some(actual) = user.get(&change.id) {
                if !same_definition(actual, value, change.harness) {
                    return Err(error(
                        change.harness,
                        "mcp-install-conflict",
                        format!("MCP server {} changed since planning", change.id),
                    ));
                }
                change.add = Step::Unchanged;
            } else {
                change.add = Step::Failed;
                if let Some(args) = &change.args {
                    run_add(change.harness, &home, args)?;
                } else {
                    write_codex(&home.directory.join("config.toml"), &change.id, value)?;
                }
                change.add = Step::Added;
                if !crate::mcp_enumeration::user_definitions(change.harness, &home)?
                    .get(&change.id)
                    .is_some_and(|v| same_definition(v, value, change.harness))
                {
                    return Err(error(
                        change.harness,
                        "install-write-failed",
                        format!(
                            "MCP server {} was added but its definition could not be verified",
                            change.id
                        ),
                    ));
                }
            }
            if change.harness != Harness::Claude {
                let result = if change.harness == Harness::Codex {
                    crate::codex_native_write::write_mcp(
                        &home.directory.join("config.toml"),
                        &change.id,
                        false,
                    )
                } else if crate::native_mcp::user_state(change.harness, &home, &change.id)?
                    == NativeState::Off
                {
                    Ok("unchanged")
                } else {
                    crate::native_mcp::write_copilot(&home, "disable", &change.id, &mut None)
                };
                match result {
                    Ok(outcome) => {
                        change.disable = if outcome == "changed" {
                            Step::Changed
                        } else {
                            Step::Unchanged
                        }
                    }
                    Err(mut d) => {
                        change.disable = Step::Failed;
                        // Native stderr may echo headers or env arguments.
                        d.message = format!(
                            "MCP server {} is installed; native disable failed",
                            change.id
                        );
                        d.hint = Some(format!(
                            "retry ayran install --mcp {} --{}",
                            change.logical,
                            change.harness.binary()
                        ));
                        return Err(d);
                    }
                }
            }
            change.outcome = if change.add == Step::Unchanged && change.disable != Step::Changed {
                Outcome::Unchanged
            } else {
                Outcome::Installed
            };
        }
        Ok::<_, Diagnostic>(())
    })();
    if let Err(d) = result {
        plan.diagnostics.push(d);
        skip_pending(plan);
    }
}
pub fn skip_pending(plan: &mut Plan) {
    for c in &mut plan.mcp {
        if c.outcome == Outcome::Planned {
            c.outcome = Outcome::Skipped;
        }
        if matches!(c.outcome, Outcome::Skipped | Outcome::Failed) {
            if c.add == Step::Planned {
                c.add = Step::Skipped;
            }
            if c.disable == Step::Planned {
                c.disable = Step::Skipped;
            }
        }
    }
}

fn run_add(h: Harness, home: &HarnessHome, args: &[String]) -> Result<(), Diagnostic> {
    fs::create_dir_all(&home.directory)
        .map_err(|_| error(h, "home-unavailable", "cannot create Harness home"))?;
    let binary = crate::program_on_path(h.binary().as_ref())
        .ok_or_else(|| error(h, "harness-not-found", "Harness not found"))?;
    let output = std::process::Command::new(binary)
        .args(args)
        .current_dir(&home.directory)
        .env(home.variable, &home.directory)
        .output()
        .map_err(|_| error(h, "install-write-failed", "cannot execute native MCP add"))?;
    if !output.status.success() {
        return Err(error(
            h,
            "install-write-failed",
            format!("native MCP add failed ({})", output.status),
        ));
    }
    Ok(())
}
fn write_codex(path: &Path, id: &str, value: &Value) -> Result<(), Diagnostic> {
    let fail = || {
        error(
            Harness::Codex,
            "install-write-failed",
            "cannot persist Codex MCP definition",
        )
    };
    let original = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return Err(fail()),
    };
    let mut doc = original
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| fail())?;
    if let Some(servers) = doc.get("mcp_servers") {
        let table = servers.as_table_like().ok_or_else(fail)?;
        if table.contains_key(id) {
            return Err(error(
                Harness::Codex,
                "mcp-install-conflict",
                "MCP definition appeared during install",
            ));
        }
    }
    let table: toml::Table = serde_json::from_value(value.clone()).map_err(|_| fail())?;
    let text = toml::to_string(&table).map_err(|_| fail())?;
    let definition = text.parse::<toml_edit::DocumentMut>().map_err(|_| fail())?;
    doc["mcp_servers"][id] = toml_edit::Item::Table(definition.as_table().clone());
    fs::create_dir_all(path.parent().unwrap()).map_err(|_| fail())?;
    let mut tmp = tempfile::NamedTempFile::new_in(path.parent().unwrap()).map_err(|_| fail())?;
    if let Ok(metadata) = fs::metadata(path) {
        tmp.as_file()
            .set_permissions(metadata.permissions())
            .map_err(|_| fail())?;
    }
    tmp.write_all(doc.to_string().as_bytes())
        .map_err(|_| fail())?;
    tmp.as_file().sync_all().map_err(|_| fail())?;
    let current = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return Err(fail()),
    };
    if current != original {
        return Err(error(
            Harness::Codex,
            "mcp-install-conflict",
            "Codex config changed during install; retry",
        ));
    }
    tmp.persist(path).map_err(|_| fail())?;
    Ok(())
}

pub fn advice(layers: &ConfigLayers, harness: Harness) -> Vec<Diagnostic> {
    let names = layers
        .mcp
        .iter()
        .filter(|(_, s)| matches!(s.value.binding(harness), Some(McpBinding::Definition(_))))
        .map(|(n, _)| n.clone())
        .collect::<Vec<_>>();
    let mut result = Plan::default();
    plan(layers, &names, Some(harness), &mut result);
    let mut diagnostics: Vec<_> = result
        .diagnostics
        .into_iter()
        .filter(|d| {
            matches!(
                d.code,
                "mcp-install-conflict"
                    | "untrusted-layer"
                    | "unsupported-binding"
                    | "trust-store-failed"
                    | "mcp-install-on"
            )
        })
        .collect();
    for d in &mut diagnostics {
        if d.severity == Severity::Error {
            d.severity = Severity::Warning;
        }
    }
    for c in result.mcp {
        if c.outcome == Outcome::Planned {
            diagnostics.push(Diagnostic {hint:Some(format!("ayran install --mcp {} --{}",c.logical,harness.binary())),..note(harness,"mcp-not-installed",format!("MCP definition {} can be installed persistently; launch can also supply it per Session",c.logical))});
        }
    }
    diagnostics
}
