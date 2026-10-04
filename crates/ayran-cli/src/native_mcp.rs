//! Explicit MCP server state changes; every target is preflighted before writing.
use std::path::PathBuf;

use crate::{native_capabilities::Row, native_plugins::NativeState};
use ayran_core::{
    config::ConfigLayers,
    diagnostic::{CapabilityKind, Diagnostic, Severity},
    harness::Harness,
    mcp::McpBinding,
};
use clap::ArgMatches;

struct Change {
    row: Row,
    logical: Option<String>,
    before: NativeState,
    outcome: &'static str,
    written: Option<String>,
}

fn config(home: &crate::harness_home::HarnessHome, harness: Harness) -> PathBuf {
    home.directory.join(if harness == Harness::Copilot {
        "settings.json"
    } else {
        "config.toml"
    })
}

pub(crate) fn run(action: &str, matches: &ArgMatches) -> i32 {
    let after = if action == "enable" {
        NativeState::On
    } else {
        NativeState::Off
    };
    let enabled = after == NativeState::On;
    let dry_run = matches.get_flag("dry-run");
    let selected = crate::native_command::selected(matches);
    let mut diagnostics = Vec::new();
    let mut changes: Vec<Change> = Vec::new();
    let result = ConfigLayers::load().and_then(|layers| {
        let harnesses: Vec<_> = crate::native_plugins::installed()
            .into_iter()
            .filter(|h| selected.is_none_or(|s| s == *h))
            .collect();
        if let Some(h) = selected
            && !harnesses.contains(&h)
        {
            return Err(Diagnostic::error(
                "harness-not-found",
                format!("{} is not installed", h.binary()),
                None,
            ));
        }
        let rows = crate::native_capabilities::read(&layers, &harnesses, CapabilityKind::Mcp)?;
        for name in matches.get_many::<String>("native-names").unwrap() {
            let logical = (!matches.get_flag("native-id"))
                .then(|| layers.mcp.get(name))
                .flatten();
            if logical.is_none() && selected.is_none() {
                diagnostics.push(Diagnostic::error(
                    "usage",
                    format!("native MCP ID {name} requires a Harness flag"),
                    None,
                ));
                continue;
            }
            for &harness in &harnesses {
                let id = if let Some(server) = logical {
                    match server.value.binding(harness) {
                        Some(McpBinding::Native(id)) => id,
                        Some(McpBinding::Connector(_)) => {
                            diagnostics.push(Diagnostic {
                                code: "native-unsupported",
                                message: format!(
                                    "{name}: Account connectors are not native-state targets"
                                ),
                                harness: Some(harness),
                                ..Default::default()
                            });
                            continue;
                        }
                        binding => {
                            let reason = match binding {
                                Some(McpBinding::Definition(_)) => "definition Binding",
                                Some(McpBinding::Absent) => "deliberately absent Binding",
                                _ => "missing Binding",
                            };
                            diagnostics.push(Diagnostic {
                                code: "native-binding-skipped",
                                severity: Severity::Note,
                                message: format!("{name} → {}: skipped {reason}", harness.binary()),
                                harness: Some(harness),
                                ..Default::default()
                            });
                            continue;
                        }
                    }
                } else {
                    name
                };
                if harness == Harness::Claude {
                    diagnostics.push(Diagnostic {
                        code: "native-unsupported",
                        message: "Claude MCP state cannot be overridden by Session selection"
                            .into(),
                        harness: Some(harness),
                        ..Default::default()
                    });
                    continue;
                }
                let targets: Vec<_> = rows
                    .iter()
                    .filter(|r| {
                        r.harness == harness
                            && (r.name == *id || logical.is_some() && r.logical.contains(name))
                    })
                    .collect();
                if targets.is_empty() {
                    diagnostics.push(Diagnostic {
                        code: "native-not-found",
                        message: format!(
                            "{name} → {} {id}: MCP server is not installed",
                            harness.binary()
                        ),
                        harness: Some(harness),
                        ..Default::default()
                    });
                }
                for row in targets {
                    if row.source == "connector" {
                        diagnostics.push(Diagnostic {
                            code: "native-unsupported",
                            message: format!(
                                "{id}: Account connectors are not native-state targets"
                            ),
                            harness: Some(harness),
                            ..Default::default()
                        });
                        continue;
                    }
                    if changes
                        .iter()
                        .any(|c| c.row.harness == harness && c.row.name == row.name)
                    {
                        continue;
                    }
                    changes.push(Change {
                        row: row.clone(),
                        logical: logical.map(|_| name.clone()),
                        before: row.state,
                        outcome: "planned",
                        written: None,
                    });
                }
            }
        }
        if diagnostics.iter().any(|d| d.severity == Severity::Error) {
            changes.clear();
            return Ok(());
        }
        let mut homes = Vec::new();
        for change in &mut changes {
            let harness = change.row.harness;
            let home = crate::native_plugins::home(&layers, harness)?;
            change.before = user_state(harness, &home, &change.row.name)?;
            homes.push(home);
        }
        if dry_run {
            return Ok(());
        }
        // Probe all binaries before materializing homes or replacing config files.
        for harness in harnesses
            .iter()
            .copied()
            .filter(|h| changes.iter().any(|c| c.row.harness == *h))
        {
            let binary = crate::program_on_path(harness.binary().as_ref()).ok_or_else(|| {
                Diagnostic::error(
                    "harness-not-found",
                    format!("{} is not installed", harness.binary()),
                    None,
                )
            })?;
            let version = crate::harness_version::check(harness.binary().as_ref(), &binary, false)?;
            let minimum = if harness == Harness::Codex {
                "0.160.0"
            } else {
                "1.0.91"
            };
            if semver::Version::parse(&version).unwrap() < semver::Version::parse(minimum).unwrap()
            {
                return Err(Diagnostic::error(
                    "harness-too-old",
                    format!(
                        "{} MCP state requires {minimum} or newer; found {version}",
                        harness.binary()
                    ),
                    None,
                ));
            }
        }
        for (change, home) in changes.iter_mut().zip(homes) {
            change.outcome = "failed";
            let path = config(&home, change.row.harness);
            let result = if change.row.harness == Harness::Codex {
                crate::codex_native_write::write_mcp(&path, &change.row.name, enabled)
            } else {
                user_state(change.row.harness, &home, &change.row.name).and_then(|state| {
                    if state == after {
                        Ok("unchanged")
                    } else {
                        write_copilot(&home, action, &change.row.name, &mut change.written)
                    }
                })
            };
            match result {
                Ok(outcome) => {
                    change.outcome = outcome;
                    if outcome == "changed" && change.row.harness == Harness::Codex {
                        change.written = Some(path.display().to_string());
                    }
                }
                Err(d) => {
                    diagnostics.push(d);
                    break;
                }
            }
        }
        for change in &mut changes {
            if change.outcome == "planned" {
                change.outcome = "failed";
            }
        }
        let effective = crate::native_capabilities::read(&layers, &harnesses, CapabilityKind::Mcp)?;
        for change in &changes {
            if !matches!(change.outcome, "changed" | "unchanged") {
                continue;
            }
            let row = effective.iter().find(|r| {
                r.harness == change.row.harness
                    && r.name == change.row.name
                    && r.path == change.row.path
            });
            let state = row
                .map(|r| effective_state(&layers, r))
                .transpose()?
                .unwrap_or(NativeState::Unknown);
            if state != after {
                diagnostics.push(Diagnostic {
                    code: "native-mcp-overridden",
                    severity: Severity::Warning,
                    message: format!(
                        "{} {}: user-level state is {}; effective state is {} because of {}",
                        change.row.harness.binary(),
                        change.row.name,
                        after.as_str(),
                        state.as_str(),
                        row.map_or("discovery changed", |r| r.layer.as_str())
                    ),
                    harness: Some(change.row.harness),
                    layer: row.map(|r| r.layer.clone()),
                    ..Default::default()
                });
            }
        }
        Ok(())
    });
    if let Err(d) = result {
        if changes.iter().all(|c| c.outcome == "planned") {
            changes.clear();
        }
        diagnostics.push(d);
    }
    if matches.get_flag("json") {
        let mut output = crate::list_command::json_envelope(&diagnostics);
        output["changes"] = serde_json::json!(
            changes
                .iter()
                .map(|c| serde_json::json!({
                    "harness": c.row.harness, "id": c.row.name,
                    "logical": c.logical, "before": c.before.as_str(), "after": after.as_str(),
                    "outcome": c.outcome, "written": c.written
                }))
                .collect::<Vec<_>>()
        );
        println!("{output}");
    } else {
        for change in changes {
            crate::list_command::print_row(vec![format!(
                "{}{} {}: {} → {} ({}{})",
                change
                    .logical
                    .map(|n| format!("{n} → "))
                    .unwrap_or_default(),
                change.row.harness.binary(),
                change.row.name,
                change.before.as_str(),
                after.as_str(),
                change.outcome,
                change.written.map(|p| format!("; {p}")).unwrap_or_default()
            )]);
        }
        for d in &diagnostics {
            crate::render(d, false);
        }
    }
    if diagnostics.iter().any(|d| d.code == "usage") {
        2
    } else if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        3
    } else {
        0
    }
}

pub(crate) fn advice(layers: &ConfigLayers, harness: Harness) -> Vec<Diagnostic> {
    if harness == Harness::Claude {
        return Vec::new();
    }
    let Ok(home) = crate::native_plugins::home(layers, harness) else {
        return Vec::new();
    };
    crate::native_capabilities::read(layers, &[harness], CapabilityKind::Mcp)
        .unwrap_or_default().into_iter()
        .filter(|r| r.source == "user" && r.logical.is_empty() && user_state(harness, &home, &r.name).ok() == Some(NativeState::On) && effective_state(layers, r).ok() == Some(NativeState::On))
        .map(|r| Diagnostic {
            code: "native-mcp-unbound", severity: Severity::Note,
            message: format!("unbound and natively on MCP server {}; consider `ayran native mcp disable {} --{} --id`", r.name, crate::shell_quote(r.name.as_ref()), harness.binary()),
            harness: Some(harness), item: Some(Box::new(r.name)), layer: Some(r.layer), ..Default::default()
        }).collect()
}

pub(crate) fn completion(
    layers: &ConfigLayers,
    harnesses: &[Harness],
    state: NativeState,
    native_id: bool,
    explicit: bool,
) -> std::collections::BTreeMap<String, String> {
    let mut names = std::collections::BTreeMap::new();
    for row in crate::native_capabilities::read(layers, harnesses, CapabilityKind::Mcp)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| {
            r.harness != Harness::Claude
                && r.source != "connector"
                && effective_state(layers, r).ok() == Some(state)
        })
    {
        let description = format!("{} {} ({})", row.harness.binary(), row.name, state.as_str());
        if explicit {
            names.insert(row.name, description.clone());
        }
        if !native_id {
            for logical in row.logical {
                names.insert(logical, description.clone());
            }
        }
    }
    names
}

pub(crate) fn user_state(
    harness: Harness,
    home: &crate::harness_home::HarnessHome,
    name: &str,
) -> Result<NativeState, Diagnostic> {
    if harness == Harness::Codex {
        crate::codex_native_write::prepare_mcp(&config(home, harness), name)
    } else {
        let mut off = std::collections::BTreeSet::new();
        crate::copilot_mcp_enumeration::read_disabled(
            &home.directory.join("settings.json"),
            &mut off,
        )?;
        Ok(NativeState::from_enabled(!off.contains(name)))
    }
}

fn effective_state(layers: &ConfigLayers, row: &Row) -> Result<NativeState, Diagnostic> {
    let home = crate::native_plugins::home(layers, row.harness)?;
    if row.harness == Harness::Copilot {
        let state = crate::copilot_mcp_enumeration::read(&home)?;
        if state.project.contains(&row.name) {
            return Ok(NativeState::Unknown);
        }
        if state.copilot_project_off.contains(&row.name) {
            return Ok(NativeState::Off);
        }
        // Harness args can change visibility without changing the user-level toggle.
        let args = layers
            .settings(row.harness)
            .args
            .as_ref()
            .map(|a| a.value.clone())
            .unwrap_or_default();
        if args.iter().enumerate().any(|(i, a)| {
            a == &format!("--enable-mcp-server={}", row.name)
                || a == "--enable-mcp-server" && args.get(i + 1) == Some(&row.name)
        }) {
            return Ok(NativeState::Unknown);
        }
        return user_state(row.harness, &home, &row.name);
    }
    if row.state == NativeState::Unknown
        && row.layer == "native state unproven"
        && row.source == "user"
        && layers.settings(row.harness).args.is_none()
    {
        return user_state(row.harness, &home, &row.name);
    }
    Ok(row.state)
}

pub(crate) fn write_copilot(
    home: &crate::harness_home::HarnessHome,
    action: &str,
    name: &str,
    written: &mut Option<String>,
) -> Result<&'static str, Diagnostic> {
    home.materialize()?;
    let command = format!("copilot mcp {action} {}", crate::shell_quote(name.as_ref()));
    let binary = crate::program_on_path("copilot".as_ref())
        .ok_or_else(|| Diagnostic::error("harness-not-found", "Copilot is not installed", None))?;
    let output = std::process::Command::new(binary)
        .args(["mcp", action, name])
        .current_dir(&home.directory)
        .env(home.variable, &home.directory)
        .output()
        .map_err(|e| Diagnostic::error("native-write-failed", format!("{command}: {e}"), None))?;
    *written = Some(command.clone());
    if output.status.success() {
        Ok("changed")
    } else {
        Err(Diagnostic {
            code: "native-write-failed",
            message: format!(
                "{command}: {}: {} {}",
                output.status,
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            harness: Some(Harness::Copilot),
            ..Default::default()
        })
    }
}
