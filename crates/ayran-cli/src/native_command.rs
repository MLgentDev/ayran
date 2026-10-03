use crate::native_plugins::NativeState;
use ayran_core::{
    config::{ConfigLayers, PluginBinding},
    diagnostic::{Diagnostic, Severity},
    doctor::HARNESSES,
    harness::Harness,
};
use clap::{Arg, ArgAction, ArgMatches, Command};

pub fn command() -> Command {
    let mut plugin = Command::new("plugin")
        .about("Inspect and switch persistent native Plugin state")
        .subcommand_required(true);
    for action in ["list", "enable", "disable"] {
        let mut command = Command::new(action);
        for h in HARNESSES {
            command = command.arg(
                Arg::new(h.binary())
                    .long(h.binary())
                    .action(ArgAction::SetTrue),
            );
        }
        command = command
            .arg(
                Arg::new("harness")
                    .long("harness")
                    .value_parser(["claude", "codex", "copilot"]),
            )
            .group(
                clap::ArgGroup::new("harness-choice")
                    .args(["claude", "codex", "copilot", "harness"]),
            )
            .arg(Arg::new("json").long("json").action(ArgAction::SetTrue));
        if action != "list" {
            command = command
                .arg(Arg::new("native-names").required(true).num_args(1..))
                .arg(Arg::new("native-id").long("id").action(ArgAction::SetTrue))
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .action(ArgAction::SetTrue),
                );
        }
        plugin = plugin.subcommand(command);
    }
    Command::new("native")
        .about("Manage Harness-native persistent state")
        .subcommand_required(true)
        .subcommand(plugin)
}
pub fn selected(matches: &ArgMatches) -> Option<Harness> {
    HARNESSES.into_iter().find(|h| {
        matches.get_flag(h.binary())
            || matches
                .get_one::<String>("harness")
                .is_some_and(|n| n == h.binary())
    })
}
struct Change {
    plugin: crate::native_plugins::Plugin,
    logical: Option<String>,
    after: NativeState,
    outcome: &'static str,
    written: Option<String>,
}
fn resolve(
    matches: &ArgMatches,
    layers: &ConfigLayers,
    rows: &[crate::native_plugins::Plugin],
    harnesses: &[Harness],
    after: NativeState,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Change> {
    let mut changes = Vec::new();
    for name in matches.get_many::<String>("native-names").unwrap() {
        let logical = (!matches.get_flag("native-id"))
            .then(|| layers.plugins.get(name))
            .flatten();
        if logical.is_none() && selected(matches).is_none() {
            diagnostics.push(Diagnostic::error(
                "usage",
                format!("native ID {name} requires a Harness flag (or configure a logical Plugin)"),
                None,
            ));
            continue;
        }
        for &harness in harnesses {
            let id = if let Some(plugin) = logical {
                match plugin.value.binding(harness) {
                    Some(PluginBinding::Native(id)) => id,
                    binding => {
                        let reason = match binding {
                            Some(PluginBinding::Path(_)) => "path Binding",
                            Some(PluginBinding::Absent) => "deliberately absent Binding",
                            _ => "missing Binding",
                        };
                        let mut d = Diagnostic {
                            code: "native-binding-skipped",
                            severity: Severity::Note,
                            message: format!("{name} → {}: skipped {reason}", harness.binary()),
                            ..Default::default()
                        };
                        d.harness = Some(harness);
                        diagnostics.push(d);
                        continue;
                    }
                }
            } else {
                name
            };
            let Some(row) = rows.iter().find(|p| p.harness == harness && p.id == *id) else {
                let mut d = Diagnostic::error(
                    "native-not-found",
                    format!(
                        "{name} → {} {id}: Plugin is not installed",
                        harness.binary()
                    ),
                    None,
                );
                d.harness = Some(harness);
                if harness == Harness::Copilot && after == NativeState::Off {
                    d.severity = Severity::Warning;
                }
                diagnostics.push(d);
                continue;
            };
            if changes
                .iter()
                .any(|c: &Change| c.plugin.harness == harness && c.plugin.id == *id)
            {
                continue;
            }
            changes.push(Change {
                plugin: row.clone(),
                logical: logical.map(|_| name.clone()),
                after,
                outcome: "planned",
                written: None,
            });
        }
    }
    changes
}
fn write_native_cli(
    change: &mut Change,
    home: &crate::harness_home::HarnessHome,
    action: &str,
) -> Result<(), Diagnostic> {
    if change.plugin.state == change.after {
        change.outcome = "unchanged";
        return Ok(());
    }
    change.outcome = "failed";
    let outcome = write_cli(
        change.plugin.harness,
        home,
        action,
        &change.plugin.id,
        &mut change.written,
    )?;
    change.outcome = outcome;
    Ok(())
}

/// Write a user-level native toggle through the Harness CLI.
pub fn write_cli(
    harness: Harness,
    home: &crate::harness_home::HarnessHome,
    action: &str,
    id: &str,
    written: &mut Option<String>,
) -> Result<&'static str, Diagnostic> {
    home.materialize()?;
    let mut args = vec!["plugin", action];
    if harness == Harness::Claude {
        args.extend(["--scope", "user", "--json"]);
    }
    args.push(id);
    let command = format!(
        "{} {}",
        harness.binary(),
        args.iter()
            .map(|a| crate::shell_quote(a.as_ref()))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let binary = crate::program_on_path(harness.binary().as_ref()).ok_or_else(|| {
        Diagnostic::error(
            "harness-not-found",
            format!("{} is not installed", harness.binary()),
            None,
        )
    })?;
    let output = std::process::Command::new(binary)
        .args(args)
        .current_dir(&home.directory)
        .env(home.variable, &home.directory)
        .output()
        .map_err(|e| Diagnostic::error("native-write-failed", format!("{command}: {e}"), None))?;
    *written = Some(command.clone());
    let response = serde_json::from_slice::<serde_json::Value>(&output.stdout).ok();
    if output.status.success() {
        Ok("changed")
    } else if harness == Harness::Claude
        && output.status.code() == Some(1)
        && response
            .as_ref()
            .and_then(|r| r.get("failureCode"))
            .and_then(|v| v.as_str())
            == Some("already_in_goal_state")
    {
        Ok("unchanged")
    } else {
        Err(Diagnostic {
            code: "native-write-failed",
            message: format!(
                "{command}: {}: {} {}",
                output.status,
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            harness: Some(harness),
            ..Default::default()
        })
    }
}
fn write_codex(
    change: &mut Change,
    home: &crate::harness_home::HarnessHome,
) -> Result<(), Diagnostic> {
    if change.plugin.state == change.after {
        change.outcome = "unchanged";
        return Ok(());
    }
    change.outcome = "failed";
    home.materialize()?;
    let path = home.directory.join("config.toml");
    crate::codex_plugin_write::write(&path, &change.plugin.id, change.after == NativeState::On)?;
    change.written = Some(path.display().to_string());
    change.outcome = "changed";
    Ok(())
}
pub fn run(matches: &ArgMatches) -> i32 {
    let (_, plugin) = matches.subcommand().unwrap();
    let (action, matches) = plugin.subcommand().unwrap();
    let mut diagnostics = Vec::new();
    let mut rows = Vec::new();
    let mut changes = Vec::new();
    let result = ConfigLayers::load().and_then(|layers| {
        let harnesses = crate::native_plugins::installed().into_iter().filter(|h| selected(matches).is_none_or(|s| s == *h)).collect::<Vec<_>>();
        if let Some(h) = selected(matches) && !harnesses.contains(&h) {
            return Err(Diagnostic::error("harness-not-found", format!("{} is not installed", h.binary()), None));
        }
        rows = crate::native_plugins::read(&layers, &harnesses)?;
        if action != "list" {
            changes = resolve(matches, &layers, &rows, &harnesses, if action == "enable" { NativeState::On } else { NativeState::Off }, &mut diagnostics);
            if diagnostics.iter().any(|d| d.severity == Severity::Error || d.code == "native-not-found") { changes.clear(); return Ok(()); }
            let dry_run = matches.get_flag("dry-run");
            let mut homes = Vec::new();
            // Preflight every target before the first persistent write.
            for change in &mut changes {
                let harness = change.plugin.harness;
                let home = crate::native_plugins::home(&layers, harness)?;
                let cwd = std::env::current_dir().map_err(|e| Diagnostic::error("enumeration-failed", e.to_string(), None))?;
                change.plugin.state = if harness == Harness::Claude {
                    crate::native_plugins::claude_user_state(&home.directory, &cwd, &change.plugin.id)?.0
                } else if harness == Harness::Codex {
                    crate::native_plugins::codex_user_state(&home.directory, &change.plugin.id)?
                } else {
                    change.plugin.state
                };
                if !dry_run {
                    let binary = crate::program_on_path(harness.binary().as_ref()).ok_or_else(|| Diagnostic::error("harness-not-found", format!("{} is not installed", harness.binary()), None))?;
                    crate::harness_version::check(harness.binary().as_ref(), &binary, false)?;
                }
                homes.push(home);
            }
            if !dry_run {
                for (change, home) in changes.iter_mut().zip(homes) {
                    let result = match change.plugin.harness {
                        Harness::Codex => write_codex(change, &home),
                        Harness::Claude | Harness::Copilot => write_native_cli(change, &home, action),
                    };
                    if let Err(d) = result {
                        diagnostics.push(d);
                        break;
                    }
                }
                // Keep completed outcomes visible if a later write or read fails.
                for change in &mut changes {
                    if change.outcome == "planned" { change.outcome = "failed"; }
                }
                let effective = crate::native_plugins::read(&layers, &harnesses)?;
                for change in &changes {
                    if matches!(change.outcome, "changed" | "unchanged")
                        && let Some(row) = effective.iter().find(|p| p.harness == change.plugin.harness && p.id == change.plugin.id)
                        && row.state != change.after {
                        diagnostics.push(Diagnostic {
                            code: "native-plugin-overridden",
                            severity: Severity::Warning,
                            message: if row.state == NativeState::Unknown {
                                format!("{} {}: user-level state requested {}; effective state cannot be confirmed because of {}", row.harness.binary(), row.id, change.after.as_str(), row.layer)
                            } else {
                                format!("{} {}: user-level state is {}, but effective state is {} because of {}", row.harness.binary(), row.id, change.after.as_str(), row.state.as_str(), row.layer)
                            },
                            harness: Some(row.harness),
                            layer: Some(row.layer.clone()),
                            ..Default::default()
                        });
                    }
                }
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
        if action == "list" {
            output["plugins"] = serde_json::json!(rows.iter().map(|p| serde_json::json!({"harness":p.harness, "id":p.id, "state":p.state.as_str(), "layer":p.layer, "logical":p.logical, "default":p.default, "reachable":p.reachable})).collect::<Vec<_>>());
        } else {
            output["changes"] = serde_json::json!(changes.iter().map(|c| serde_json::json!({"harness":c.plugin.harness, "id":c.plugin.id, "logical":c.logical, "before":c.plugin.state.as_str(), "after":c.after.as_str(), "written":c.written, "outcome":c.outcome})).collect::<Vec<_>>());
        }
        println!("{output}");
    } else {
        if action == "list" {
            crate::list_command::print_row(vec![
                "harness".into(),
                "id".into(),
                "state".into(),
                "layer".into(),
                "logical".into(),
                "default".into(),
                "reachable".into(),
            ]);
            for row in rows {
                crate::list_command::print_row(vec![
                    row.harness.binary().into(),
                    row.id,
                    row.state.as_str().into(),
                    row.layer,
                    row.logical.join(","),
                    row.default.to_string(),
                    row.reachable.to_string(),
                ]);
            }
        } else {
            for c in changes {
                crate::list_command::print_row(vec![format!(
                    "{}{} {}: {} → {} ({})",
                    c.logical.map(|n| format!("{n} → ")).unwrap_or_default(),
                    c.plugin.harness.binary(),
                    c.plugin.id,
                    c.plugin.state.as_str(),
                    c.after.as_str(),
                    match c.written {
                        Some(command) => format!("{}; {command}", c.outcome),
                        None if c.outcome == "unchanged" => format!("already {}", c.after.as_str()),
                        None => c.outcome.to_owned(),
                    }
                )]);
            }
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
