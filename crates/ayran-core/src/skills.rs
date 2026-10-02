//! Skill selection and projection, using filesystem facts supplied by the edge.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::cache::GeneratedSkills;
use crate::config::{ConfigLayers, SkillBinding};
use crate::diagnostic::Diagnostic;
use crate::harness::Harness;
use crate::resolve::{CapabilityOrigin, Request, profile_selections};

#[derive(Default)]
pub struct SkillState {
    /// Validated path Skills, with their frontmatter name or directory-name fallback.
    pub names: BTreeMap<PathBuf, String>,
    /// XDG cache directory with the ayran suffix, resolved by the caller.
    pub cache_root: PathBuf,
    /// Standalone personal Skills discovered in the Harness home.
    pub personal: BTreeSet<String>,
    /// Copilot Skills from settings.json skillDirectories, which cannot be hidden.
    pub custom: BTreeSet<String>,
    /// Copilot's disabledSkills setting, enumerated read-only.
    pub disabled: BTreeSet<String>,
    /// Standalone project Skills; these must never be hidden.
    pub project: BTreeSet<String>,
    pub bundled: BTreeSet<String>,
    pub enterprise: BTreeSet<String>,
    /// Native invocation aliases mapped to effective names for visibility overrides.
    pub aliases: BTreeMap<String, String>,
    /// Codex Skills keyed by canonical SKILL.md path, for path-scoped overrides.
    pub codex: BTreeMap<PathBuf, CodexSkill>,
}

impl SkillState {
    /// Resolve a standalone native Skill, including Harness invocation aliases.
    pub(crate) fn native_name<'a>(
        &'a self,
        harness: Harness,
        name: &'a String,
    ) -> Option<&'a String> {
        let discovered = |name: &String| {
            if harness == Harness::Codex {
                self.codex.values().any(|skill| &skill.name == name)
            } else {
                self.personal.contains(name)
                    || self.custom.contains(name)
                    || self.project.contains(name)
                    || self.bundled.contains(name)
                    || self.enterprise.contains(name)
            }
        };
        let effective = if discovered(name) {
            name
        } else {
            self.aliases.get(name).unwrap_or(name)
        };
        discovered(effective).then_some(effective)
    }
}

pub struct CodexSkill {
    pub name: String,
    pub personal: bool,
}

pub struct ResolvedSkills {
    pub trace: Vec<String>,
    pub generated: Option<GeneratedSkills>,
    pub overrides: BTreeMap<String, &'static str>,
    pub path_overrides: BTreeMap<PathBuf, bool>,
    pub diagnostics: Vec<Diagnostic>,
}

pub(crate) fn resolve(
    selection: SkillSelection<'_>,
    layers: &ConfigLayers,
    harness: Harness,
    state: &SkillState,
) -> Result<ResolvedSkills, Vec<Diagnostic>> {
    let mut trace = selection.trace;
    let mut diagnostics = selection.diagnostics;
    let mut native_items = BTreeSet::new();
    let mut names = BTreeMap::new();
    let mut targets = BTreeMap::new();
    let mut overrides = BTreeMap::new();
    for (logical, origin) in selection.active {
        let skill = &layers.skills[logical];
        let binding = skill
            .value
            .binding(harness)
            .expect("selected Skill has a Binding");
        let (name, item, target) = match binding {
            SkillBinding::Native(name) => {
                let Some(effective) = state.native_name(harness, name) else {
                    return Err(vec![
                        Diagnostic::error(
                            "native-not-found",
                            format!("Skill {logical} names undiscovered standalone Skill {name}"),
                            None,
                        )
                        .for_capability(
                            harness,
                            crate::diagnostic::CapabilityKind::Skill,
                            logical,
                            &skill.path,
                        )
                        .with_item(name),
                    ]);
                };
                if harness == Harness::Claude {
                    overrides.insert(effective.clone(), "on");
                }
                (effective.clone(), format!("native {name}"), None)
            }
            SkillBinding::Path(path) => {
                let name = state.names.get(path).ok_or_else(|| {
                    vec![
                        Diagnostic::error(
                            "path-not-found",
                            format!(
                                "Skill path {} must contain a readable SKILL.md",
                                path.display()
                            ),
                            None,
                        )
                        .for_capability(
                            harness,
                            crate::diagnostic::CapabilityKind::Skill,
                            logical,
                            &skill.path,
                        )
                        .with_item(path.display().to_string()),
                    ]
                })?;
                (
                    name.clone(),
                    format!("path {}", path.display()),
                    Some(path.clone()),
                )
            }
            SkillBinding::Absent => unreachable!("false Bindings are handled during selection"),
        };
        trace.push(format!(
            "Skill {logical}: {origin} → {name} ({item}, {})",
            skill.path.display()
        ));
        // Only native Bindings deduplicate across different logical names.
        if target.is_none() && !native_items.insert(name.clone()) {
            continue;
        }
        if let Some(previous) = names.insert(name.clone(), logical) {
            return Err(vec![
                Diagnostic::error(
                    "skill-name-clash",
                    format!("Skills {previous} and {logical} both resolve to {name}"),
                    None,
                )
                .for_capability(
                    harness,
                    crate::diagnostic::CapabilityKind::Skill,
                    logical,
                    &skill.path,
                )
                .with_item(name),
            ]);
        }
        if let Some(target) = target {
            targets.insert(name, target);
        }
    }

    if harness == Harness::Copilot {
        for name in native_items.intersection(&state.disabled) {
            diagnostics.push(
                Diagnostic {
                    code: "skill-disabled",
                    severity: crate::diagnostic::Severity::Note,
                    message: format!(
                        "selected native Skill {name} is disabled in Copilot's settings"
                    ),
                    hint: Some(format!("copilot skill enable {name}")),
                    layer: None,
                    ..Diagnostic::default()
                }
                .for_capability(
                    harness,
                    crate::diagnostic::CapabilityKind::Skill,
                    names[name],
                    &layers.skills[names[name]].path,
                )
                .with_item(name),
            );
        }
        for name in targets.keys().filter(|name| state.project.contains(*name)) {
            diagnostics.push(
                Diagnostic {
                    code: "skill-shadowed",
                    severity: crate::diagnostic::Severity::Note,
                    message: format!(
                        "path Skill {name} is shadowed by a project Skill of the same name"
                    ),
                    hint: None,
                    layer: None,
                    ..Diagnostic::default()
                }
                .for_capability(
                    harness,
                    crate::diagnostic::CapabilityKind::Skill,
                    names[name],
                    &layers.skills[names[name]].path,
                )
                .with_item(name),
            );
        }
        for (items, cause, scope) in [
            (&state.personal, "copilot-personal-skill", "personal"),
            (&state.custom, "copilot-custom-skill-dir", "custom-dir"),
        ] {
            let leaked: Vec<_> = items.difference(&native_items).cloned().collect();
            if !leaked.is_empty() {
                diagnostics.push(Diagnostic {
                    code: "leak",
                    severity: crate::diagnostic::Severity::Warning,
                    message: format!(
                        "{cause}: unselected {scope} Skills remain visible: {}",
                        leaked.join(", ")
                    ),
                    hint: None,
                    layer: None,
                    harness: Some(harness),
                    cause: Some(Box::new(cause.into())),
                    item: Some(Box::new(leaked.join(", "))),
                    ..Diagnostic::default()
                });
            }
        }
    }
    let mut path_overrides = BTreeMap::new();
    if harness == Harness::Codex {
        for (path, skill) in &state.codex {
            let selected = native_items.contains(&skill.name);
            if selected || skill.personal {
                path_overrides.insert(path.clone(), selected);
                if !selected {
                    trace.push(format!(
                        "Skill {}: hidden (unselected personal Skill, {})",
                        skill.name,
                        path.display()
                    ));
                }
            }
        }
    }
    for name in state.personal.iter().filter(|_| harness == Harness::Claude) {
        if native_items.contains(name) {
            continue;
        }
        let protected_scope = if state.project.contains(name) {
            Some(("claude-project-shadow", "project"))
        } else if state.enterprise.contains(name) {
            Some(("claude-enterprise-shadow", "enterprise"))
        } else if state.bundled.contains(name) {
            Some(("claude-bundled-shadow", "bundled"))
        } else {
            None
        };
        if let Some((cause, scope)) = protected_scope {
            diagnostics.push(Diagnostic {
                code: "leak",
                severity: crate::diagnostic::Severity::Warning,
                message: format!("{cause}: personal Skill {name} cannot be hidden by name without hiding the {scope} Skill"),
                hint: None,
                layer: None,
                harness: Some(harness),
                cause: Some(Box::new(cause.into())),
                item: Some(Box::new(name.clone())),
                ..Diagnostic::default()
            });
        }
        if targets.contains_key(name) {
            diagnostics.push(Diagnostic {
                code: "skill-shadowed",
                severity: crate::diagnostic::Severity::Note,
                message: format!(
                    "path Skill {name} is shadowed by an unselected personal Skill of the same name"
                ),
                hint: None,
                layer: None,
                ..Diagnostic::default()
            }.for_capability(harness, crate::diagnostic::CapabilityKind::Skill, names[name], &layers.skills[names[name]].path).with_item(name));
        }
        if protected_scope.is_none() && !targets.contains_key(name) {
            overrides.insert(name.clone(), "off");
            trace.push(format!("Skill {name}: hidden (unselected personal Skill)"));
        }
    }
    let generated = if targets.is_empty() {
        None
    } else {
        Some(GeneratedSkills::new(&state.cache_root, harness, targets))
    };
    Ok(ResolvedSkills {
        trace,
        generated,
        overrides,
        path_overrides,
        diagnostics,
    })
}

/// Logical selection shared by filesystem preparation and pure projection.
/// Only surviving, supported Bindings reach path enumeration.
pub struct SkillSelection<'a> {
    pub(crate) active: BTreeMap<&'a String, CapabilityOrigin<'a>>,
    pub trace: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

impl SkillSelection<'_> {
    pub fn names(&self) -> Vec<String> {
        self.active.keys().map(|name| (*name).clone()).collect()
    }
}

pub fn select<'a>(
    request: &'a Request,
    layers: &'a ConfigLayers,
    harness: Harness,
) -> Result<SkillSelection<'a>, Vec<Diagnostic>> {
    let (profiles, _, diagnostics) = profile_selections(request, layers)?;
    let mut selection = select_from_profiles(request, layers, harness, profiles.skills)?;
    selection.diagnostics.extend(diagnostics);
    Ok(selection)
}

pub(crate) fn select_from_profiles<'a>(
    request: &'a Request,
    layers: &'a ConfigLayers,
    harness: Harness,
    profiles: BTreeMap<&'a String, CapabilityOrigin<'a>>,
) -> Result<SkillSelection<'a>, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let alias = request.alias.as_ref().and_then(|name| {
        layers
            .aliases
            .get(name)
            .map(|definition| (name, definition))
    });
    let mut candidates = BTreeMap::new();
    for (name, skill) in &layers.skills {
        if skill.value.default {
            candidates.insert(name, CapabilityOrigin::Default);
        }
    }
    candidates.extend(profiles);
    if let Some((name, definition)) = alias {
        for skill in &definition.skills {
            candidates.insert(skill, CapabilityOrigin::Alias(name));
        }
    }
    for name in &request.skills {
        candidates.insert(name, CapabilityOrigin::Cli("--skill"));
    }
    // Unknown explicit names are errors even if a CLI Disable removes them.
    for (name, origin) in &candidates {
        if origin.is_direct_selection() && !layers.skills.contains_key(*name) {
            return Err(vec![Diagnostic::error(
                "unknown-skill",
                format!("unknown Skill {name}"),
                None,
            )]);
        }
    }
    let considered: BTreeSet<_> = candidates.keys().copied().collect();
    let mut active = BTreeMap::new();
    let mut trace = Vec::new();
    for (name, origin) in candidates {
        let reason = if request.no_skills.contains(name) {
            Some("disabled by --no-skill".to_owned())
        } else if matches!(origin, CapabilityOrigin::Default) && request.no_defaults {
            Some("disabled by --no-defaults".to_owned())
        } else if matches!(origin, CapabilityOrigin::Default)
            && alias.is_some_and(|(_, definition)| !definition.defaults)
        {
            Some(format!(
                "disabled by Alias {} (defaults = false)",
                alias.unwrap().0
            ))
        } else if matches!(origin, CapabilityOrigin::Default)
            && layers.disabled_default_skills.contains_key(name)
        {
            Some(format!(
                "disabled by layer {} (Defaults)",
                layers.disabled_default_skills[name].display()
            ))
        } else if !origin.is_direct_selection() && layers.disabled_skills.contains_key(name) {
            Some(format!(
                "disabled by layer {}",
                layers.disabled_skills[name].display()
            ))
        } else if !origin.is_direct_selection()
            && alias.is_some_and(|(_, definition)| definition.disabled_skills.contains(name))
        {
            Some(format!("disabled by Alias {}", alias.unwrap().0))
        } else {
            None
        };
        if let Some(reason) = reason {
            trace.push(format!("Skill {name}: {reason} ({origin})"));
            continue;
        }
        let skill = &layers.skills[name];
        let binding = skill.value.binding(harness).ok_or_else(|| {
            vec![
                Diagnostic::error(
                    "missing-binding",
                    format!("Skill {name} has no Binding for {}", harness.binary()),
                    None,
                )
                .for_capability(
                    harness,
                    crate::diagnostic::CapabilityKind::Skill,
                    name,
                    &skill.path,
                ),
            ]
        })?;
        match binding {
            SkillBinding::Absent if !origin.is_direct_selection() => {
                let message = format!(
                    "Skill {name}: skipped because of a false Binding for {} ({origin}, {})",
                    harness.binary(),
                    skill.path.display()
                );
                trace.push(message.clone());
                diagnostics.push(
                    Diagnostic {
                        code: "binding-skipped",
                        severity: crate::diagnostic::Severity::Note,
                        message,
                        hint: None,
                        layer: Some(skill.path.display().to_string()),
                        ..Diagnostic::default()
                    }
                    .for_capability(
                        harness,
                        crate::diagnostic::CapabilityKind::Skill,
                        name,
                        &skill.path,
                    ),
                );
                continue;
            }
            SkillBinding::Absent => {
                return Err(vec![
                    Diagnostic::error(
                        "binding-absent",
                        format!(
                            "explicitly selected Skill {name} is deliberately absent for {}",
                            harness.binary()
                        ),
                        None,
                    )
                    .for_capability(
                        harness,
                        crate::diagnostic::CapabilityKind::Skill,
                        name,
                        &skill.path,
                    ),
                ]);
            }
            SkillBinding::Path(_) if harness == Harness::Codex => {
                return Err(vec![
                    Diagnostic::error(
                        "unsupported-binding",
                        format!("Codex cannot load path Skill {name}"),
                        None,
                    )
                    .for_capability(
                        harness,
                        crate::diagnostic::CapabilityKind::Skill,
                        name,
                        &skill.path,
                    ),
                ]);
            }
            _ => {}
        }
        active.insert(name, origin);
    }
    for name in layers.skills.keys() {
        if !considered.contains(name) {
            let reason = if request.no_skills.contains(name) {
                "disabled by --no-skill"
            } else {
                "unselected"
            };
            trace.push(format!("Skill {name}: {reason}"));
        }
    }
    Ok(SkillSelection {
        active,
        trace,
        diagnostics,
    })
}
