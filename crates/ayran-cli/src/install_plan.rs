//! Resolve install targets against user-level native state before any persistent writes.
use ayran_core::{
    config::{ConfigLayers, PluginBinding},
    diagnostic::{Diagnostic, Severity},
    doctor::HARNESSES,
    harness::Harness,
    marketplace::{Definition, Source},
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Planned,
    Added,
    Installed,
    Unchanged,
    Skipped,
    Failed,
}
impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Added => "added",
            Self::Installed => "installed",
            Self::Unchanged => "unchanged",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }
}

pub struct Change {
    pub harness: Harness,
    pub id: String,
    pub logical: String,
    pub outcome: Outcome,
    pub command: Option<String>,
}
impl Change {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"harness":self.harness, "id":self.id, "logical":self.logical, "outcome":self.outcome, "command":self.command})
    }
}
#[derive(Default)]
pub struct Plan {
    pub marketplaces: Vec<Change>,
    pub plugins: Vec<Change>,
    pub mcp: Vec<crate::install_mcp::Change>,
    pub skills: Vec<crate::install_skills::Change>,
    pub diagnostics: Vec<Diagnostic>,
}
impl Plan {
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
}
pub fn build(layers: &ConfigLayers, names: &[String], selected: Option<Harness>) -> Plan {
    let mut plan = Plan::default();
    for name in names {
        if !layers.plugins.contains_key(name) {
            plan.diagnostics.push(Diagnostic::error(
                "unknown-plugin",
                format!("unknown logical Plugin {name}"),
                None,
            ));
        }
    }
    let targets: BTreeSet<&str> = if names.is_empty() {
        layers.plugins.keys().map(String::as_str).collect()
    } else {
        names
            .iter()
            .filter(|name| layers.plugins.contains_key(*name))
            .map(String::as_str)
            .collect()
    };
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
            crate::harness_version::check(harness.binary().as_ref(), &binary, false)?;
            let home = crate::native_plugins::home(layers, harness)?;
            let registered = crate::marketplace_state::read(harness, &home.directory)?;
            let installed = crate::marketplace_state::installed_plugins(harness, &home)?;
            let mut markets: BTreeSet<&str> = if names.is_empty() {
                layers.marketplaces.keys().map(String::as_str).collect()
            } else {
                BTreeSet::new()
            };
            for name in &targets {
                let plugin = &layers.plugins[*name];
                let Some(PluginBinding::Native(id)) = plugin.value.binding(harness) else {
                    plan.plugins.push(Change {
                        harness,
                        id: String::new(),
                        logical: (*name).into(),
                        outcome: Outcome::Skipped,
                        command: None,
                    });
                    plan.diagnostics.push(note(
                        harness,
                        "install-skipped",
                        format!(
                            "Plugin {name}: missing, path or deliberately absent Binding; skipped"
                        ),
                    ));
                    continue;
                };
                let declared = layers.marketplace_for_plugin(harness, id);
                let native_market = id.split_once('@').map(|(_, market)| market);
                if let Some((logical, _)) = declared {
                    markets.insert(logical);
                }
                let outcome = if installed.contains(id) {
                    Outcome::Unchanged
                } else if declared.is_some()
                    || native_market.is_some_and(|m| registered.contains_key(m))
                {
                    Outcome::Planned
                } else {
                    plan.diagnostics.push(note(harness, "marketplace-undeclared", format!("Plugin {name} ({id}): Marketplace is neither declared nor registered; skipped")));
                    Outcome::Skipped
                };
                plan.plugins.push(Change {
                    harness,
                    id: id.clone(),
                    logical: (*name).into(),
                    outcome,
                    command: (outcome == Outcome::Planned).then(|| plugin_command(harness, id)),
                });
            }
            for logical in markets {
                let marketplace = &layers.marketplaces[logical];
                let Some(d) = marketplace.value.definition(harness) else {
                    plan.marketplaces.push(Change {
                        harness,
                        id: logical.into(),
                        logical: logical.into(),
                        outcome: Outcome::Skipped,
                        command: None,
                    });
                    plan.diagnostics.push(note(
                        harness,
                        "install-skipped",
                        format!(
                            "Marketplace {logical}: missing or deliberately absent Binding; skipped"
                        ),
                    ));
                    continue;
                };
                let id = d.name.as_deref().unwrap_or(logical);
                if layers.marketplaces.iter().any(|(other, m)| {
                    m.value.definition(harness).is_some_and(|other_definition| {
                        other_definition.name.as_deref().unwrap_or(other) == id
                            && !crate::marketplace_state::same_source(other_definition, d)
                    })
                }) {
                    plan.diagnostics.push(Diagnostic {
                        harness: Some(harness), layer: Some(marketplace.path.display().to_string()),
                        ..Diagnostic::error("marketplace-conflict", format!("multiple declarations request different sources or refs for native Marketplace {id}"), None)
                    });
                }
                if let Some(native) = registered.get(id)
                    && !crate::marketplace_state::same_source(native, d)
                {
                    plan.diagnostics.push(Diagnostic {
                        harness: Some(harness), layer: Some(marketplace.path.display().to_string()),
                        ..Diagnostic::error("marketplace-conflict", format!("Marketplace {id} is registered from {} ref {:?}; declaration requests {} ref {:?}", native.source.label(), native.reference, d.source.label(), d.reference), None)
                    });
                }
                if harness != Harness::Codex
                    && d.reference.as_deref().is_some_and(|r| {
                        (7..=40).contains(&r.len()) && r.bytes().all(|b| b.is_ascii_hexdigit())
                    })
                {
                    plan.diagnostics.push(Diagnostic {
                        harness: Some(harness),
                        layer: Some(marketplace.path.display().to_string()),
                        ..Diagnostic::error(
                            "unsupported-ref",
                            format!(
                                "{} cannot install Marketplace {id} at a SHA ref",
                                harness.binary()
                            ),
                            Some("use a branch or tag, or select Codex"),
                        )
                    });
                }
                let outcome = if registered
                    .get(id)
                    .is_some_and(|native| crate::marketplace_state::same_source(native, d))
                {
                    Outcome::Unchanged
                } else {
                    Outcome::Planned
                };
                if !registered.contains_key(id)
                    && marketplace.value.project
                    && !crate::trust::Store::read()?.trusted(&marketplace.path)?
                {
                    plan.diagnostics.push(Diagnostic {
                        harness: Some(harness),
                        layer: Some(marketplace.path.display().to_string()),
                        ..Diagnostic::error(
                            "untrusted-layer",
                            format!(
                                "{}: Marketplace {logical} source {} ref {:?} is untrusted",
                                marketplace.path.display(),
                                d.source.label(),
                                d.reference
                            ),
                            Some(&format!(
                                "ayran trust {}",
                                crate::shell_quote(marketplace.path.as_os_str())
                            )),
                        )
                    });
                }
                plan.marketplaces.push(Change {
                    harness,
                    id: id.into(),
                    logical: logical.into(),
                    outcome,
                    command: (outcome == Outcome::Planned).then(|| marketplace_command(harness, d)),
                });
            }
            Ok::<_, Diagnostic>(())
        })();
        if let Err(mut d) = result {
            d.harness = Some(harness);
            plan.diagnostics.push(d);
        }
    }
    plan
}
fn note(harness: Harness, code: &'static str, message: impl std::fmt::Display) -> Diagnostic {
    Diagnostic {
        code,
        severity: Severity::Note,
        harness: Some(harness),
        message: format!("{}: {message}", harness.binary()),
        ..Default::default()
    }
}
fn render_command(harness: Harness, args: &[String]) -> String {
    format!(
        "{} {}",
        harness.binary(),
        args.iter()
            .map(|a| crate::shell_quote(a.as_ref()))
            .collect::<Vec<_>>()
            .join(" ")
    )
}
fn plugin_command(harness: Harness, id: &str) -> String {
    render_command(harness, &plugin_args(harness, id))
}
pub fn plugin_args(harness: Harness, id: &str) -> Vec<String> {
    let mut args = vec![
        "plugin".into(),
        if harness == Harness::Codex {
            "add"
        } else {
            "install"
        }
        .into(),
        id.into(),
    ];
    if harness == Harness::Claude {
        args.extend(["--scope".into(), "user".into(), "--json".into()]);
    }
    if harness == Harness::Codex {
        args.push("--json".into());
    }
    args
}
fn marketplace_command(harness: Harness, d: &Definition) -> String {
    render_command(harness, &marketplace_args(harness, d))
}
pub fn marketplace_args(harness: Harness, d: &Definition) -> Vec<String> {
    let mut source = match &d.source {
        Source::Github(repo) => repo.clone(),
        Source::Git(url) => url.clone(),
        Source::Path(path) => path.display().to_string(),
    };
    if harness != Harness::Codex
        && let Some(reference) = &d.reference
    {
        source.push('#');
        source.push_str(reference);
    }
    let mut args = vec!["plugin".into(), "marketplace".into(), "add".into(), source];
    if harness == Harness::Codex {
        if let Some(reference) = &d.reference {
            args.extend(["--ref".into(), reference.clone()]);
        }
        args.push("--json".into());
    }
    if harness == Harness::Claude {
        args.extend(["--scope".into(), "user".into(), "--json".into()]);
    }
    args
}

pub fn audit(layers: &ConfigLayers, harness: Harness) -> Vec<Diagnostic> {
    let plan = build(layers, &[], Some(harness));
    let mut diagnostics: Vec<_> = plan
        .diagnostics
        .into_iter()
        .filter(|d| {
            matches!(
                d.code,
                "untrusted-layer"
                    | "marketplace-conflict"
                    | "unsupported-ref"
                    | "trust-store-failed"
            )
        })
        .collect();
    for (logical, marketplace) in &layers.marketplaces {
        if let Some(d) = marketplace.value.definition(harness)
            && d.reference.is_none()
        {
            diagnostics.push(Diagnostic {
                layer: Some(marketplace.path.display().to_string()),
                ..note(
                    harness,
                    "marketplace-unpinned",
                    format!(
                        "Marketplace {logical} source {} has no ref pin",
                        d.source.label()
                    ),
                )
            });
        }
    }
    diagnostics
}
