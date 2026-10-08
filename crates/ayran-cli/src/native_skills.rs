//! Explicit standalone Skill state changes; every target is preflighted before writing.
use std::path::PathBuf;

use crate::{native_capabilities::Row, native_plugins::NativeState};
use ayran_core::{
    config::{ConfigLayers, SkillBinding},
    diagnostic::{CapabilityKind, Diagnostic, Severity},
    harness::Harness,
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
    home.directory.join(if harness == Harness::Claude {
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
        let rows = crate::native_capabilities::read(&layers, &harnesses, CapabilityKind::Skill)?;
        for name in matches.get_many::<String>("native-names").unwrap() {
            let logical = (!matches.get_flag("native-id"))
                .then(|| layers.skills.get(name))
                .flatten();
            if logical.is_none() && selected.is_none() {
                diagnostics.push(Diagnostic::error(
                    "usage",
                    format!("native Skill ID {name} requires a Harness flag"),
                    None,
                ));
                continue;
            }
            for &harness in &harnesses {
                let id = if let Some(skill) = logical {
                    match skill.value.binding(harness) {
                        Some(SkillBinding::Native(id)) => id,
                        binding => {
                            let reason = match binding {
                                Some(SkillBinding::Path(_)) => "path Binding",
                                Some(SkillBinding::Git(_)) => "git Binding",
                                Some(SkillBinding::Builtin(_)) => "built-in Binding",
                                Some(SkillBinding::Absent) => "deliberately absent Binding",
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
                if harness == Harness::Copilot {
                    diagnostics.push(Diagnostic {
                        code: "native-unsupported",
                        message: "Copilot Skill state cannot be overridden by Session selection"
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
                            "{name} → {} {id}: standalone Skill is not installed",
                            harness.binary()
                        ),
                        harness: Some(harness),
                        ..Default::default()
                    });
                }
                for row in targets {
                    if changes.iter().any(|c| {
                        c.row.harness == harness && c.row.name == row.name && c.row.path == row.path
                    }) {
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
            change.before = crate::native_skill_write::prepare(
                harness,
                &config(&home, harness),
                &change.row.name,
                change.row.path.as_deref(),
                enabled,
            )?;
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
            let minimum = if harness == Harness::Claude {
                "2.1.288"
            } else {
                "0.160.0"
            };
            if semver::Version::parse(&version).unwrap() < semver::Version::parse(minimum).unwrap()
            {
                return Err(Diagnostic::error(
                    "harness-too-old",
                    format!(
                        "{} Skill state requires {minimum} or newer; found {version}",
                        harness.binary()
                    ),
                    None,
                ));
            }
        }
        for (change, home) in changes.iter_mut().zip(homes) {
            change.outcome = "failed";
            let path = config(&home, change.row.harness);
            match crate::native_skill_write::write(
                change.row.harness,
                &path,
                &change.row.name,
                change.row.path.as_deref(),
                enabled,
            ) {
                Ok(outcome) => {
                    change.outcome = outcome;
                    if outcome == "changed" {
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
        let effective =
            crate::native_capabilities::read(&layers, &harnesses, CapabilityKind::Skill)?;
        for change in &changes {
            if !matches!(change.outcome, "changed" | "unchanged") {
                continue;
            }
            let row = effective.iter().find(|r| {
                r.harness == change.row.harness
                    && r.name == change.row.name
                    && r.path == change.row.path
            });
            if row.is_none_or(|r| r.state != after) {
                diagnostics.push(Diagnostic {
                    code: "native-skill-overridden",
                    severity: Severity::Warning,
                    message: format!(
                        "{} {}: user-level state is {}; effective state is {} because of {}",
                        change.row.harness.binary(),
                        change.row.name,
                        after.as_str(),
                        row.map_or("unknown", |r| r.state.as_str()),
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
                    "harness": c.row.harness, "id": c.row.name, "path": c.row.path,
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
    if harness == Harness::Copilot {
        return Vec::new();
    }
    crate::native_capabilities::read(layers, &[harness], CapabilityKind::Skill)
        .unwrap_or_default().into_iter()
        .filter(|r| r.source == "personal" && r.state == NativeState::On && r.logical.is_empty())
        .map(|r| Diagnostic {
            code: "native-skill-unbound", severity: Severity::Note,
            message: format!("unbound and natively on Skill {}; consider `ayran native skill disable {} --{} --id`", r.name, crate::shell_quote(r.name.as_ref()), harness.binary()),
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
    for row in crate::native_capabilities::read(layers, harnesses, CapabilityKind::Skill)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.harness != Harness::Copilot && r.state == state)
    {
        let description = format!(
            "{} {} ({})",
            row.harness.binary(),
            row.name,
            row.state.as_str()
        );
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
