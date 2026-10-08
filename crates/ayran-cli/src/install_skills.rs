//! Explicit Skill snapshot planning and persistent off, separate from Session loading.
use crate::{
    install_plan::{Outcome, Plan},
    skill_snapshot::{Identity, error},
};
use ayran_core::{
    config::{ConfigLayers, SkillBinding},
    diagnostic::{CapabilityKind, Diagnostic, Severity},
    harness::Harness,
};
use std::path::PathBuf;

#[derive(Clone, Copy)]
pub(crate) enum FetchMode {
    Preview,
    Interactive,
    NonInteractive,
}

pub(crate) struct Change {
    pub harness: Harness,
    pub logical: String,
    pub id: String,
    pub outcome: Outcome,
    pub copy: &'static str,
    pub disable: &'static str,
    destination: PathBuf,
    config: PathBuf,
    identity: Option<Identity>,
    previous: Option<(Identity, PathBuf)>,
    copy_source: PathBuf,
    fetched: Option<crate::skill_git::Fetched>,
    git: Option<ayran_core::config::GitSkillBinding>,
    builtin: Option<String>,
}
impl Change {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"harness":self.harness,"logical":self.logical,"id":if self.git.is_some() && self.identity.is_none() { serde_json::Value::Null } else { serde_json::json!(self.id) },"path":(!self.destination.as_os_str().is_empty()).then_some(&self.destination),"outcome":self.outcome,"copy":self.copy,"disable":self.disable,"source":self.source_label(),"ref":self.git.as_ref().and_then(|g| g.r#ref.as_ref()),"subdir":self.git.as_ref().map(|g| &g.subdir),"commit":self.identity.as_ref().and_then(|i| i.git.as_ref().map(|g| &g.commit))})
    }
    pub fn source_label(&self) -> Option<String> {
        self.builtin
            .as_ref()
            .map(|name| format!("builtin:{name}"))
            .or_else(|| self.git.as_ref().map(|g| g.source.clone()))
    }
    fn preflight_native(
        &mut self,
        home: &crate::harness_home::HarnessHome,
        rows: &[crate::native_capabilities::Row],
        exists: bool,
        plan: &mut Plan,
    ) -> Result<(), Diagnostic> {
        crate::skill_snapshot::check_personal(home, self.harness, &self.id, &self.destination)?;
        for row in rows.iter().filter(|r| r.name == self.id) {
            let same = row.path.as_ref().is_some_and(|p| {
                crate::skill_enumeration::same_root(p, &self.destination.join("SKILL.md"))
            });
            if !same && (self.harness == Harness::Codex || row.source != "personal" || !exists) {
                return Err(error(
                    "skill-install-conflict",
                    format!("{} collides with a {} Skill", self.id, row.source),
                ));
            }
        }
        self.disable = if self.harness == Harness::Copilot {
            "unsupported"
        } else {
            "planned"
        };
        if self.harness != Harness::Copilot {
            let before = crate::native_skill_write::prepare(
                self.harness,
                &self.config,
                &self.id,
                Some(&self.destination.join("SKILL.md")),
                false,
            )?;
            if before == crate::native_plugins::NativeState::Off {
                self.disable = "unchanged";
            }
        } else {
            let mut skills = ayran_core::skills::SkillState::default();
            let real = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from);
            crate::copilot_skill_enumeration::read(&mut skills, real.as_deref(), home)?;
            if skills.disabled.contains(&self.id) {
                return Err(error(
                    "skill-install-conflict",
                    "Copilot Skill is natively off and cannot be enabled by Session selection",
                ));
            }
            plan.diagnostics.push(Diagnostic {severity:Severity::Note,harness:Some(self.harness), ..error("skill-install-on", "Copilot snapshot remains natively on: unselected personal Skills Leak; no verified Session enable override")});
        }
        Ok(())
    }
}
pub(crate) fn plan(
    layers: &ConfigLayers,
    names: &[String],
    selected: Option<Harness>,
    plan: &mut Plan,
    fetch: FetchMode,
) {
    if names.is_empty() {
        return;
    }
    let installed = crate::native_plugins::installed();
    for name in names {
        let Some(skill) = layers.skills.get(name) else {
            plan.diagnostics
                .push(error("unknown-skill", format!("unknown Skill {name}")));
            continue;
        };
        for harness in ayran_core::doctor::HARNESSES
            .into_iter()
            .filter(|h| selected.is_none_or(|s| s == *h))
        {
            let result = (|| {
                if !installed.contains(&harness) {
                    return Err(Diagnostic {
                        severity: Severity::Note,
                        ..error("harness-not-found", "Harness is not installed; skipped")
                    });
                }
                let home = crate::native_plugins::home(layers, harness)?;
                let rows =
                    crate::native_capabilities::read(layers, &[harness], CapabilityKind::Skill)?;
                let Some(binding) = skill.value.binding(harness) else {
                    return Err(skipped("missing Binding"));
                };
                let mut change = Change {
                    harness,
                    logical: name.clone(),
                    id: String::new(),
                    outcome: Outcome::Unchanged,
                    copy: "unchanged",
                    disable: "skipped",
                    destination: PathBuf::new(),
                    config: home.directory.join(if harness == Harness::Claude {
                        "settings.json"
                    } else {
                        "config.toml"
                    }),
                    identity: None,
                    previous: None,
                    copy_source: PathBuf::new(),
                    fetched: None,
                    git: None,
                    builtin: None,
                };
                match binding {
                    SkillBinding::Absent => return Err(skipped("deliberately absent Binding")),
                    SkillBinding::Native(id) => {
                        let row = rows
                            .iter()
                            .find(|r| r.name == *id || r.logical.contains(name))
                            .ok_or_else(|| {
                                error(
                                    "native-not-found",
                                    format!("standalone Skill {id} is not discovered"),
                                )
                            })?;
                        change.id = row.name.clone();
                        change.destination = row.path.clone().unwrap_or_default();
                    }
                    SkillBinding::Path(_) | SkillBinding::Git(_) | SkillBinding::Builtin(_) => {
                        let user = ayran_core::config::user_config_path()?;
                        if !matches!(binding, SkillBinding::Builtin(_))
                            && skill.path.file_name().is_some_and(|p| p == "ayran.toml")
                            && !crate::skill_enumeration::same_root(&skill.path, &user)
                            && !crate::trust::Store::read()?.trusted(&skill.path)?
                        {
                            return Err(Diagnostic {
                                hint: Some(format!(
                                    "ayran trust {}",
                                    crate::shell_quote(skill.path.as_os_str())
                                )),
                                ..error(
                                    "untrusted-layer",
                                    "project Skill source requires layer Trust",
                                )
                            });
                        }
                        let binary = crate::program_on_path(harness.binary().as_ref()).unwrap();
                        let version = crate::harness_version::check(
                            harness.binary().as_ref(),
                            &binary,
                            false,
                        )?;
                        let minimum = match harness {
                            Harness::Claude => "2.1.288",
                            Harness::Codex => "0.160.0",
                            Harness::Copilot => "1.0.91",
                        };
                        if semver::Version::parse(&version).unwrap()
                            < semver::Version::parse(minimum).unwrap()
                        {
                            return Err(error(
                                "harness-too-old",
                                format!("Skill installation requires {minimum}; found {version}"),
                            ));
                        }
                        let identity = match binding {
                            SkillBinding::Builtin(name) => {
                                change.builtin = Some(name.clone());
                                change.copy_source = crate::builtin_skills::directory(name)?;
                                crate::skill_snapshot::builtin(name)?
                            }
                            SkillBinding::Path(path) => {
                                change.copy_source = path.clone();
                                crate::skill_snapshot::source(path)?
                            }
                            SkillBinding::Git(g) => {
                                change.git = Some(g.clone());
                                if matches!(fetch, FetchMode::Preview) {
                                    change.outcome = Outcome::Planned;
                                    change.copy = "fetch-planned";
                                    change.disable = if harness == Harness::Copilot {
                                        "unsupported"
                                    } else {
                                        "planned"
                                    };
                                    // The native identity and refreshed tree are unknown until fetching.
                                    let existing = crate::skill_snapshot::owned_binding(
                                        binding,
                                        name,
                                        &home.directory.join("skills"),
                                    )?;
                                    if let Some((old, path)) = existing {
                                        change.destination = path;
                                        change.id = old.name;
                                        change.preflight_native(&home, &rows, true, plan)?;
                                    } else {
                                        change.id = format!("pending fetch: {name}");
                                    }
                                    if harness == Harness::Copilot
                                        && change.destination.as_os_str().is_empty()
                                    {
                                        plan.diagnostics.push(Diagnostic { severity: Severity::Note, harness: Some(harness), ..error("skill-install-on", "Copilot snapshot remains natively on: unselected personal Skills Leak; no verified Session enable override") });
                                    }
                                    return Ok(change);
                                }
                                let fetched = crate::skill_git::fetch(
                                    g,
                                    matches!(fetch, FetchMode::Interactive),
                                )?;
                                let mut identity = crate::skill_snapshot::source(&fetched.path)?;
                                identity.source = PathBuf::from(&g.source);
                                identity.git = Some(crate::skill_snapshot::GitIdentity {
                                    binding: g.clone(),
                                    commit: fetched.commit.clone(),
                                    logical: name.clone(),
                                });
                                change.copy_source = fetched.path.clone();
                                change.fetched = Some(fetched);
                                identity
                            }
                            _ => unreachable!(),
                        };
                        // Source paths remain canonical for local snapshots.
                        if matches!(binding, SkillBinding::Path(_)) {
                            change.copy_source = identity.source.clone();
                        }
                        let destination = home.directory.join("skills").join(&identity.name);
                        let owned = crate::skill_snapshot::owned_binding(
                            binding,
                            name,
                            &home.directory.join("skills"),
                        )?;
                        let target = crate::skill_snapshot::installed(&destination)?;
                        if owned.as_ref().is_some_and(|(_, p)| *p != destination)
                            && target.is_some()
                        {
                            return Err(error(
                                "skill-install-conflict",
                                "new native name already belongs to another snapshot",
                            ));
                        }
                        if let Some(old) = &target
                            && (old.source != identity.source
                                || old.git.as_ref().is_some_and(|g| g.logical != *name))
                            && layers.skills.iter().any(|(other_name, other)| {
                                other_name != name
                                    && match other.value.binding(harness) {
                                        Some(SkillBinding::Path(path)) => {
                                            old.git.is_none()
                                                && path
                                                    .canonicalize()
                                                    .unwrap_or_else(|_| path.clone())
                                                    == old.source
                                        }
                                        Some(SkillBinding::Builtin(name)) => {
                                            old.source
                                                == std::path::Path::new(&format!("builtin:{name}"))
                                        }
                                        Some(SkillBinding::Git(_)) => old
                                            .git
                                            .as_ref()
                                            .is_some_and(|g| g.logical == *other_name),
                                        _ => false,
                                    }
                            })
                        {
                            return Err(error(
                                "skill-install-conflict",
                                "native identity still belongs to another declared Skill source",
                            ));
                        }
                        let exists = target.is_some();
                        change.previous =
                            owned.or_else(|| target.map(|old| (old, destination.clone())));
                        change.id = identity.name.clone();
                        change.destination = destination;
                        change.preflight_native(&home, &rows, exists, plan)?;
                        change.outcome = Outcome::Planned;
                        change.copy = match &change.previous {
                            Some((old, _)) if *old == identity => "unchanged",
                            Some(_) => "replace-planned",
                            None => "planned",
                        };
                        change.identity = Some(identity);
                    }
                }
                Ok(change)
            })();
            match result {
                Ok(change) => {
                    if let Some(previous) = plan
                        .skills
                        .iter()
                        .find(|c| c.harness == harness && c.id == change.id)
                    {
                        if previous.identity != change.identity {
                            plan.diagnostics.push(error(
                                "skill-install-conflict",
                                format!("selected Skill sources collide on {}", change.id),
                            ));
                        }
                    } else {
                        plan.skills.push(change);
                    }
                }
                Err(mut d) => {
                    d.harness = Some(harness);
                    plan.diagnostics.push(d);
                }
            }
        }
    }
}
fn skipped(reason: &str) -> Diagnostic {
    Diagnostic {
        severity: Severity::Note,
        ..error("install-skipped", format!("Skill skipped: {reason}"))
    }
}
pub(crate) fn execute(plan: &mut Plan) {
    for change in &mut plan.skills {
        let Some(identity) = &change.identity else {
            continue;
        };
        change.outcome = Outcome::Failed;
        let result = (|| {
            change.copy = "failed";
            if change.builtin.is_some() {
                crate::builtin_skills::materialize(&change.copy_source)?;
            }
            change.copy = crate::skill_snapshot::publish(
                &change.destination,
                identity,
                change.previous.as_ref(),
                &change.copy_source,
            )?;
            if change.harness != Harness::Copilot {
                change.disable = "failed";
                change.disable = crate::native_skill_write::write(
                    change.harness,
                    &change.config,
                    &change.id,
                    Some(
                        &change
                            .destination
                            .join("SKILL.md")
                            .canonicalize()
                            .map_err(|e| error("install-write-failed", e.to_string()))?,
                    ),
                    false,
                )?;
            }
            change.outcome = if change.copy == "unchanged"
                && matches!(change.disable, "unchanged" | "unsupported")
            {
                Outcome::Unchanged
            } else {
                Outcome::Installed
            };
            Ok::<_, Diagnostic>(())
        })();
        if let Err(mut d) = result {
            d.hint = Some(format!(
                "retry ayran install --skill {} --{}",
                crate::shell_quote(change.logical.as_ref()),
                change.harness.binary()
            ));
            plan.diagnostics.push(d);
            break;
        }
    }
    skip_pending(plan);
}

pub(crate) fn skip_pending(plan: &mut Plan) {
    for change in &mut plan.skills {
        if change.outcome == Outcome::Planned {
            change.outcome = Outcome::Skipped;
        }
        if matches!(change.copy, "planned" | "replace-planned" | "fetch-planned") {
            change.copy = "skipped";
        }
        if change.disable == "planned" {
            change.disable = "skipped";
        }
    }
}

pub(crate) fn advice(layers: &ConfigLayers, harness: Harness) -> Vec<Diagnostic> {
    let reachable = ayran_core::doctor::reachable_capabilities(
        layers,
        &crate::native_plugins::installed(),
        harness,
        CapabilityKind::Skill,
    );
    let names = layers
        .skills
        .iter()
        .filter(|(name, s)| {
            (!s.path.as_os_str().is_empty() || reachable.contains(*name))
                && matches!(
                    s.value.binding(harness),
                    Some(SkillBinding::Path(_) | SkillBinding::Git(_) | SkillBinding::Builtin(_))
                )
        })
        .map(|(n, _)| n.clone())
        .collect::<Vec<_>>();
    let mut result = Plan::default();
    plan(
        layers,
        &names,
        Some(harness),
        &mut result,
        FetchMode::Preview,
    );
    for d in &mut result.diagnostics {
        if d.severity == Severity::Error {
            d.severity = Severity::Warning;
        }
    }
    for change in result.skills.into_iter().filter(|c| {
        (c.identity.is_some()
            && (matches!(c.copy, "planned" | "replace-planned") || c.disable == "planned"))
            || (c.git.is_some() && !c.destination.as_os_str().is_empty() && c.disable == "planned")
    }) {
        result.diagnostics.push(Diagnostic {
            severity: Severity::Note,
            harness: Some(harness),
            hint: Some(format!(
                "ayran install --skill {} --{}",
                crate::shell_quote(change.logical.as_ref()),
                harness.binary()
            )),
            ..error(
                "skill-not-installed",
                format!(
                    "Skill {} can be installed or refreshed as a snapshot",
                    change.logical
                ),
            )
        });
    }
    result.diagnostics
}
