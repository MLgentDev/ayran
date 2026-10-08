//! Named native updates, with validation before writes and restoration after each update.
use crate::{harness_home::HarnessHome, native_plugins::NativeState};
use ayran_core::{
    config::{ConfigLayers, PluginBinding},
    diagnostic::{Diagnostic, Severity},
    harness::Harness,
    marketplace::Source,
};
use clap::ArgMatches;
use serde_json::Value;

struct Target {
    harness: Harness,
    id: String,
    logical: Option<String>,
    home: HarnessHome,
    state: NativeState,
    codex_enabled: Option<bool>,
    local_source: bool,
    before: Option<String>,
    after: Option<String>,
    outcome: &'static str,
    commands: Vec<String>,
    written: Option<String>,
}

pub fn run(kind: &str, matches: &ArgMatches) -> i32 {
    let mut diagnostics = Vec::new();
    let mut targets = Vec::new();
    let dry_run = matches.get_flag("dry-run");
    let result = ConfigLayers::load().and_then(|layers| {
        targets = resolve(kind, matches, &layers, &mut diagnostics)?;
        if diagnostics.iter().any(|d| d.severity == Severity::Error) {
            targets.clear();
            return Ok(());
        }
        if !dry_run {
            // Check every selected Harness before starting persistent updates.
            for h in ayran_core::doctor::HARNESSES {
                if targets.iter().any(|t| t.harness == h && !t.local_source) {
                    let binary = crate::program_on_path(h.binary().as_ref()).ok_or_else(|| {
                        Diagnostic::error(
                            "harness-not-found",
                            format!("{} is not installed", h.binary()),
                            None,
                        )
                    })?;
                    crate::harness_version::check(h.binary().as_ref(), &binary, false)?;
                }
            }
            for target in &mut targets {
                if target.local_source {
                    target.outcome = "unchanged";
                    continue;
                }
                target.outcome = "failed";
                match execute(kind, target, &mut diagnostics) {
                    Ok(()) => {
                        target.outcome = if target.before == target.after {
                            "unchanged"
                        } else {
                            "changed"
                        }
                    }
                    Err(d) => diagnostics.push(d),
                }
            }
        }
        Ok(())
    });
    if let Err(d) = result {
        targets.clear();
        diagnostics.push(d);
    }
    if matches.get_flag("json") {
        let mut output = crate::list_command::json_envelope(&diagnostics);
        output["changes"] = serde_json::json!(
            targets
                .iter()
                .map(|t| serde_json::json!({
                    "harness": t.harness, "id": t.id, "logical": t.logical,
                    "before": t.before, "after": t.after, "outcome": t.outcome,
                    "commands": t.commands, "written": t.written,
                }))
                .collect::<Vec<_>>()
        );
        println!("{output}");
    } else {
        for t in targets {
            let result = if t.local_source {
                "unchanged (local source)".to_owned()
            } else if dry_run {
                format!("planned: {}", t.commands.join("; "))
            } else if t.outcome == "changed" {
                format!(
                    "{} → {}",
                    t.before.as_deref().unwrap_or("unknown"),
                    t.after.as_deref().unwrap_or("unknown")
                )
            } else {
                t.outcome.to_owned()
            };
            crate::list_command::print_row(vec![format!(
                "{} {}: {result}",
                t.harness.binary(),
                t.id
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

fn resolve(
    kind: &str,
    matches: &ArgMatches,
    layers: &ConfigLayers,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<Target>, Diagnostic> {
    let selected = crate::native_command::selected(matches);
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
    let plugins = if kind == "plugin" {
        crate::native_plugins::read(layers, &harnesses)?
    } else {
        Vec::new()
    };
    let mut targets: Vec<Target> = Vec::new();
    for name in matches.get_many::<String>("native-names").unwrap() {
        let logical = !matches.get_flag("native-id")
            && if kind == "plugin" {
                layers.plugins.contains_key(name)
            } else {
                layers.marketplaces.contains_key(name)
            };
        if !logical && selected.is_none() {
            diagnostics.push(Diagnostic::error(
                "usage",
                format!("native name {name} requires a Harness flag"),
                None,
            ));
            continue;
        }
        for &h in &harnesses {
            let id = if !logical {
                Some(name.clone())
            } else if kind == "plugin" {
                match layers.plugins[name].value.binding(h) {
                    Some(PluginBinding::Native(id)) => Some(id.clone()),
                    _ => None,
                }
            } else {
                layers.marketplaces[name]
                    .value
                    .definition(h)
                    .map(|d| d.name.as_deref().unwrap_or(name).to_owned())
            };
            let Some(id) = id else {
                diagnostics.push(skipped(
                    h,
                    name,
                    "missing, path or deliberately absent Binding",
                ));
                continue;
            };
            let home = crate::native_plugins::home(layers, h)?;
            let mut state = NativeState::Unknown;
            let local_source;
            if kind == "plugin" {
                let installed = installed_plugins(h, &home)?;
                if !installed.contains(&id) {
                    if plugins.iter().any(|p| p.harness == h && p.id == id) {
                        diagnostics.push(skipped(
                            h,
                            name,
                            "Plugin is installed only at project or local scope",
                        ));
                    } else {
                        diagnostics.push(not_found(h, name, &id));
                    }
                    continue;
                }
                state = match h {
                    Harness::Claude => {
                        crate::native_plugins::claude_user_state(
                            &home.directory,
                            &std::env::current_dir().map_err(|e| {
                                Diagnostic::error("enumeration-failed", e.to_string(), None)
                            })?,
                            &id,
                        )?
                        .0
                    }
                    Harness::Codex => {
                        match crate::native_plugins::codex_user_state(&home.directory, &id)? {
                            NativeState::Unknown => NativeState::On,
                            s => s,
                        }
                    }
                    Harness::Copilot => plugins
                        .iter()
                        .find(|p| p.harness == h && p.id == id)
                        .map_or(NativeState::On, |p| p.state),
                };
                local_source = false;
            } else {
                let registered = crate::marketplace_state::read_listing(h, &home.directory)?;
                let Some(registration) = registered.get(&id) else {
                    diagnostics.push(not_found(h, name, &id));
                    continue;
                };
                local_source = registration
                    .definition
                    .as_ref()
                    .is_some_and(|d| matches!(d.source, Source::Path(_)))
                    || ["file:", "path:"]
                        .iter()
                        .any(|prefix| registration.source.starts_with(prefix));
                if h == Harness::Codex
                    && !local_source
                    && !registration
                        .definition
                        .as_ref()
                        .is_some_and(|d| matches!(d.source, Source::Git(_) | Source::Github(_)))
                {
                    diagnostics.push(skipped(h, name, "Codex only upgrades Git Marketplaces"));
                    continue;
                }
            }
            if targets.iter().any(|t| t.harness == h && t.id == id) {
                continue;
            }
            let before = version(kind, h, &home.directory, &id)?;
            let mut target = Target {
                harness: h,
                logical: logical.then(|| name.clone()),
                state,
                codex_enabled: if h == Harness::Codex && kind == "plugin" {
                    crate::codex_config::read_table(&home.directory.join("config.toml"))?
                        .get("plugins")
                        .and_then(|v| v.get(&id))
                        .and_then(|v| v.get("enabled"))
                        .and_then(toml::Value::as_bool)
                } else {
                    None
                },
                id,
                home,
                local_source,
                after: before.clone(),
                before,
                outcome: "planned",
                commands: Vec::new(),
                written: None,
            };
            if !local_source {
                target.commands.push(command(h, &args(kind, &target)));
                if kind == "plugin" && h == Harness::Codex {
                    target.commands.push(format!(
                        "restore {}: plugins.{}.enabled={}",
                        target.home.directory.join("config.toml").display(),
                        target.id,
                        target.codex_enabled.map_or_else(
                            || "omitted (native default)".to_owned(),
                            |enabled| enabled.to_string()
                        )
                    ));
                }
                if kind == "plugin" && h == Harness::Claude && state == NativeState::Off {
                    target.commands.push(format!(
                        "if command_source_inactive: {}; {}; always {}",
                        toggle_command(&target, "enable"),
                        command(h, &args(kind, &target)),
                        toggle_command(&target, "disable")
                    ));
                }
            }
            targets.push(target);
        }
    }
    Ok(targets)
}
fn skipped(h: Harness, name: &str, reason: &str) -> Diagnostic {
    Diagnostic {
        code: "native-binding-skipped",
        severity: Severity::Note,
        harness: Some(h),
        message: format!("{name} → {}: skipped {reason}", h.binary()),
        ..Default::default()
    }
}
fn not_found(h: Harness, name: &str, id: &str) -> Diagnostic {
    Diagnostic {
        harness: Some(h),
        ..Diagnostic::error(
            "native-not-found",
            format!(
                "{name} → {} {id}: native target is not installed or registered",
                h.binary()
            ),
            None,
        )
    }
}
fn args(kind: &str, t: &Target) -> Vec<String> {
    let mut args: Vec<String> = if kind == "marketplace" {
        vec![
            "plugin",
            "marketplace",
            if t.harness == Harness::Codex {
                "upgrade"
            } else {
                "update"
            },
        ]
    } else {
        vec![
            "plugin",
            if t.harness == Harness::Codex {
                "add"
            } else {
                "update"
            },
        ]
    }
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.push(t.id.clone());
    if kind == "plugin" && t.harness == Harness::Claude {
        args.extend(["--scope", "user", "--json"].map(str::to_owned));
    }
    args
}
fn command(h: Harness, args: &[String]) -> String {
    format!(
        "{} {}",
        h.binary(),
        args.iter()
            .map(|a| crate::shell_quote(a.as_ref()))
            .collect::<Vec<_>>()
            .join(" ")
    )
}
fn toggle_command(t: &Target, action: &str) -> String {
    command(
        t.harness,
        &["plugin", action, "--scope", "user", "--json", &t.id].map(str::to_owned),
    )
}
fn invoke(t: &mut Target, args: &[String]) -> Result<std::process::Output, Diagnostic> {
    let cmd = command(t.harness, args);
    let binary = crate::program_on_path(t.harness.binary().as_ref())
        .ok_or_else(|| Diagnostic::error("harness-not-found", cmd.clone(), None))?;
    t.home.materialize()?;
    t.written = Some(match &t.written {
        Some(previous) => format!("{previous}; {cmd}"),
        None => cmd.clone(),
    });
    std::process::Command::new(binary)
        .args(args)
        .current_dir(&t.home.directory)
        .env(t.home.variable, &t.home.directory)
        .output()
        .map_err(|e| Diagnostic::error("native-write-failed", format!("{cmd}: {e}"), None))
}
fn failure(t: &Target, output: &std::process::Output) -> Diagnostic {
    Diagnostic {
        harness: Some(t.harness),
        ..Diagnostic::error(
            "native-write-failed",
            format!(
                "{}: {}: {} {}",
                t.written.as_deref().unwrap_or(&t.id),
                output.status,
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            None,
        )
    }
}
fn last_json(output: &std::process::Output) -> Option<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .and_then(|line| serde_json::from_str(line).ok())
}
fn execute(
    kind: &str,
    t: &mut Target,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), Diagnostic> {
    let args = args(kind, t);
    let mut result = invoke(t, &args);
    let mut restore_failure = None;
    if kind == "plugin"
        && t.harness == Harness::Claude
        && t.state == NativeState::Off
        && result.as_ref().is_ok_and(|o| {
            !o.status.success()
                && last_json(o).is_some_and(|v| v["failureCode"] == "command_source_inactive")
        })
    {
        let enable = toggle(t, "enable");
        result = match enable {
            Ok(_) => invoke(t, &args),
            Err(d) => Err(d),
        };
        // Enabling can fail after a partial write: always restore the original off state.
        if let Err(mut d) = toggle(t, "disable") {
            d.hint = Some(format!(
                "run ayran native plugin disable --claude --id {}",
                crate::shell_quote(t.id.as_ref())
            ));
            restore_failure = Some(d);
        }
    }
    if kind == "plugin" && t.harness == Harness::Codex {
        // add may change config before reporting a failure, so restoration is unconditional.
        if let Err(d) = crate::codex_native_write::restore_plugin(
            &t.home.directory.join("config.toml"),
            &t.id,
            t.codex_enabled,
        ) {
            restore_failure = Some(d);
        }
    }
    let output = match result.and_then(|o| {
        if o.status.success() {
            Ok(o)
        } else {
            Err(failure(t, &o))
        }
    }) {
        Ok(output) => output,
        Err(d) => {
            if let Some(restore) = restore_failure {
                diagnostics.push(restore);
            }
            return Err(d);
        }
    };
    t.after = version(kind, t.harness, &t.home.directory, &t.id)?;
    if t.harness == Harness::Claude && kind == "plugin" {
        if let Some(response) = last_json(&output) {
            t.before = response
                .get("oldVersion")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or(t.before.take());
            t.after = response
                .get("newVersion")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or(t.after.take());
            if response["updateOutcome"] == "unchanged" {
                t.after = t.before.clone();
            }
        }
    } else if t.harness == Harness::Copilot {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if let Some((before, after)) = line.split_once('→').or_else(|| line.split_once("->"))
            {
                t.before = before.split_whitespace().last().map(str::to_owned);
                t.after = after
                    .split_whitespace()
                    .next()
                    .map(|s| s.trim_end_matches(['.', ',', ')']).to_owned());
            }
            if line.to_ascii_lowercase().contains("already at latest") {
                t.after = t.before.clone();
            }
        }
    }
    match restore_failure {
        Some(d) => Err(d),
        None => Ok(()),
    }
}

fn toggle(t: &mut Target, action: &str) -> Result<(), Diagnostic> {
    let mut written = None;
    let result = crate::native_command::write_cli(t.harness, &t.home, action, &t.id, &mut written);
    if let Some(command) = written {
        t.written = Some(format!("{}; {command}", t.written.as_deref().unwrap_or("")));
    }
    result.map(|_| ())
}

fn version(
    kind: &str,
    h: Harness,
    home: &std::path::Path,
    id: &str,
) -> Result<Option<String>, Diagnostic> {
    if kind == "marketplace" {
        let path = if h == Harness::Codex {
            home.join("config.toml")
        } else if h == Harness::Claude {
            home.join("plugins/known_marketplaces.json")
        } else {
            home.join("settings.json")
        };
        let record = if h == Harness::Codex {
            crate::codex_config::read_table(&path)?
                .get("marketplaces")
                .and_then(|v| v.get(id))
                .map(|v| serde_json::to_value(v).unwrap())
        } else {
            crate::plugin_enumeration::copilot_json(&path)?.and_then(|v| {
                if h == Harness::Claude {
                    v.get(id).cloned()
                } else {
                    v.get("extraKnownMarketplaces")
                        .and_then(|v| v.get(id))
                        .cloned()
                }
            })
        };
        if let Some(version) = record.as_ref().and_then(|v| {
            ["version", "revision", "sha", "gitCommitSha"]
                .into_iter()
                .find_map(|key| v.get(key).and_then(Value::as_str))
        }) {
            return Ok(Some(version.to_owned()));
        }
        let checkout = record
            .as_ref()
            .and_then(|v| v.get("installLocation"))
            .and_then(Value::as_str)
            .map(std::path::PathBuf::from)
            .or_else(|| (h == Harness::Codex).then(|| home.join(".tmp/marketplaces").join(id)));
        return checkout
            .map(|p| git_revision(&p))
            .transpose()
            .map(Option::flatten);
    }
    if h == Harness::Claude {
        return Ok(crate::plugin_enumeration::copilot_json(
            &home.join("plugins/installed_plugins.json"),
        )?
        .and_then(|v| {
            v.get("plugins")?
                .get(id)?
                .as_array()?
                .iter()
                .find(|v| v["scope"] == "user")?
                .get("version")?
                .as_str()
                .map(str::to_owned)
        }));
    }
    if h == Harness::Codex {
        let Some((name, market)) = id.rsplit_once('@') else {
            return Ok(None);
        };
        return Ok(crate::plugin_enumeration::active_codex_plugin_root(
            &home.join("plugins/cache").join(market).join(name),
        )?
        .and_then(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_owned)));
    }
    Ok(
        crate::plugin_enumeration::copilot_json(&home.join("config.json"))?.and_then(|v| {
            v.get("installedPlugins")?
                .as_array()?
                .iter()
                .find(|p| {
                    format!(
                        "{}@{}",
                        p["name"].as_str().unwrap_or(""),
                        p["marketplace"].as_str().unwrap_or("")
                    ) == id
                })?
                .get("version")?
                .as_str()
                .map(str::to_owned)
        }),
    )
}

/// Completion uses the same user-level inventory as update resolution and never runs a Harness.
pub fn completion(
    kind: &str,
    layers: &ConfigLayers,
    harnesses: &[Harness],
    native_id: bool,
    explicit: bool,
) -> Vec<(String, String)> {
    let mut names = std::collections::BTreeMap::new();
    for &h in harnesses {
        let Ok(home) = crate::native_plugins::home(layers, h) else {
            return Vec::new();
        };
        let ids = if kind == "plugin" {
            let Ok(ids) = installed_plugins(h, &home) else {
                return Vec::new();
            };
            ids
        } else {
            let Ok(rows) = crate::marketplace_state::read_listing(h, &home.directory) else {
                return Vec::new();
            };
            rows.into_keys().collect()
        };
        for id in ids {
            let description = format!("{} {id}", h.binary());
            if explicit {
                names.insert(id.clone(), description.clone());
            }
            if !native_id {
                if kind == "plugin" {
                    for (logical, p) in &layers.plugins {
                        if matches!(p.value.binding(h), Some(PluginBinding::Native(binding)) if binding == &id)
                        {
                            names.insert(logical.clone(), description.clone());
                        }
                    }
                } else {
                    for (logical, m) in &layers.marketplaces {
                        if m.value
                            .definition(h)
                            .is_some_and(|d| d.name.as_deref().unwrap_or(logical) == id)
                        {
                            names.insert(logical.clone(), description.clone());
                        }
                    }
                }
            }
        }
    }
    names.into_iter().collect()
}

fn git_revision(checkout: &std::path::Path) -> Result<Option<String>, Diagnostic> {
    let read = |path: &std::path::Path| -> Result<Option<String>, Diagnostic> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Diagnostic::error(
                "enumeration-failed",
                format!("{}: {e}", path.display()),
                None,
            )),
        }
    };
    let git = checkout.join(".git");
    let git = if git.is_file() {
        let Some(text) = read(&git)? else {
            return Ok(None);
        };
        let Some(path) = text.trim().strip_prefix("gitdir: ") else {
            return Ok(None);
        };
        checkout.join(path)
    } else {
        git
    };
    let Some(head) = read(&git.join("HEAD"))? else {
        return Ok(None);
    };
    let head = head.trim();
    let valid = |sha: &str| sha.len() >= 40 && sha.chars().all(|c| c.is_ascii_hexdigit());
    let Some(reference) = head.strip_prefix("ref: ") else {
        return Ok(valid(head).then(|| head.to_owned()));
    };
    if !reference.starts_with("refs/") || reference.split('/').any(|s| matches!(s, ".." | "." | ""))
    {
        return Ok(None);
    }
    // Worktrees keep refs in the common Git directory.
    let common = read(&git.join("commondir"))?.map_or(git.clone(), |p| git.join(p.trim()));
    if let Some(sha) = read(&common.join(reference))? {
        return Ok(valid(sha.trim()).then(|| sha.trim().to_owned()));
    }
    Ok(read(&common.join("packed-refs"))?.and_then(|text| {
        text.lines().find_map(|line| {
            let (sha, name) = line.split_once(' ')?;
            (name == reference && valid(sha)).then(|| sha.to_owned())
        })
    }))
}

/// Updates require an install root; Copilot toggle entries alone may be stale.
fn installed_plugins(
    harness: Harness,
    home: &HarnessHome,
) -> Result<std::collections::BTreeSet<String>, Diagnostic> {
    if harness != Harness::Copilot {
        return crate::marketplace_state::installed_plugins(harness, home);
    }
    let inventory = crate::plugin_enumeration::read(harness, None, home)?;
    let mut ids = inventory.user;
    ids.extend(
        inventory
            .direct
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_owned)),
    );
    Ok(ids)
}
