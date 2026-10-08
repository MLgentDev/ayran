//! Selection-independent config audit. Filesystem facts are supplied by the caller.
use crate::config::{Alias, ConfigLayers, PluginBinding, SkillBinding};
use crate::diagnostic::{CapabilityKind, Diagnostic, Severity};
use crate::harness::Harness;
use crate::mcp::{McpBinding, McpDefinition};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub const HARNESSES: [Harness; 3] = [Harness::Claude, Harness::Codex, Harness::Copilot];
#[derive(Default)]
pub struct DoctorState {
    pub installed: Vec<Harness>,
    pub harnesses: Vec<HarnessState>,
    pub paths: BTreeSet<PathBuf>,
    pub skill_names: BTreeMap<PathBuf, String>,
    /// Directory identities for path Bindings, canonicalized by the edge.
    pub canonical_paths: BTreeMap<PathBuf, PathBuf>,
    /// Executables shadowed by Aliases on the current PATH, excluding ayran itself.
    pub alias_commands: BTreeMap<String, PathBuf>,
}

/// Successfully enumerated inventories. A failed inventory stays absent, avoiding false gaps.
pub struct HarnessState {
    pub harness: Harness,
    pub plugins: Option<crate::enumerate::InstalledPlugins>,
    pub skills: Option<crate::skills::SkillState>,
    pub mcp: Option<crate::mcp::McpState>,
}

#[derive(Default)]
struct Reachable {
    members: BTreeSet<(CapabilityKind, String)>,
}
impl Reachable {
    fn insert(&mut self, kind: CapabilityKind, name: &str) {
        self.members.insert((kind, name.into()));
    }
    fn contains(&self, kind: CapabilityKind, name: &str) -> bool {
        self.members.contains(&(kind, name.into()))
    }
    fn profile(&mut self, name: &str, layers: &ConfigLayers, alias: Option<&Alias>, direct: bool) {
        if !direct
            && (layers.disabled_profiles.contains_key(name)
                || alias.is_some_and(|a| a.disabled_profiles.iter().any(|n| n == name)))
        {
            return;
        }
        if self.contains(CapabilityKind::Profile, name) {
            return;
        }
        self.insert(CapabilityKind::Profile, name);
        if let Some(profile) = layers.profiles.get(name) {
            for (kind, names) in [
                (CapabilityKind::Plugin, &profile.value.plugins),
                (CapabilityKind::Skill, &profile.value.skills),
                (CapabilityKind::Mcp, &profile.value.mcp),
            ] {
                for name in names {
                    if !disabled(layers, alias, kind, name) {
                        self.insert(kind, name);
                    }
                }
            }
            for member in &profile.value.profiles {
                self.profile(member, layers, alias, false);
            }
        }
    }
}
fn disabled(
    layers: &ConfigLayers,
    alias: Option<&Alias>,
    kind: CapabilityKind,
    name: &str,
) -> bool {
    match kind {
        CapabilityKind::Plugin => {
            layers.disabled_plugins.contains_key(name)
                || alias.is_some_and(|a| a.disabled_plugins.iter().any(|n| n == name))
        }
        CapabilityKind::Skill => {
            layers.disabled_skills.contains_key(name)
                || alias.is_some_and(|a| a.disabled_skills.iter().any(|n| n == name))
        }
        CapabilityKind::Mcp => {
            layers.disabled_mcp.contains_key(name)
                || alias.is_some_and(|a| a.disabled_mcp.iter().any(|n| n == name))
        }
        _ => false,
    }
}
fn defaults(layers: &ConfigLayers, alias: Option<&Alias>, result: &mut Reachable) {
    for (kind, names) in [
        (
            CapabilityKind::Plugin,
            layers
                .plugins
                .iter()
                .filter(|(n, s)| s.value.default && !layers.disabled_defaults.contains_key(*n))
                .map(|(n, _)| n)
                .collect::<Vec<_>>(),
        ),
        (
            CapabilityKind::Skill,
            layers
                .skills
                .iter()
                .filter(|(n, s)| {
                    s.value.default && !layers.disabled_default_skills.contains_key(*n)
                })
                .map(|(n, _)| n)
                .collect(),
        ),
        (
            CapabilityKind::Mcp,
            layers
                .mcp
                .iter()
                .filter(|(n, s)| s.value.default && !layers.disabled_default_mcp.contains_key(*n))
                .map(|(n, _)| n)
                .collect(),
        ),
    ] {
        for name in names {
            if !disabled(layers, alias, kind, name) {
                result.insert(kind, name);
            }
        }
    }
    for (name, profile) in &layers.profiles {
        if profile.value.default && !layers.disabled_default_profiles.contains_key(name) {
            result.profile(name, layers, alias, false);
        }
    }
}
fn session_selections(
    layers: &ConfigLayers,
    state: &DoctorState,
    harness: Harness,
) -> Vec<Reachable> {
    let mut sessions = Vec::new();
    let mut result = Reachable::default();
    if state.installed.contains(&harness) {
        defaults(layers, None, &mut result);
        sessions.push(result);
    }
    for alias in layers
        .aliases
        .values()
        .filter(|a| a.effective_harness(layers) == Some(harness))
    {
        let mut selected = Reachable::default();
        if alias.defaults {
            defaults(layers, Some(alias), &mut selected);
        }
        for (kind, names) in [
            (CapabilityKind::Plugin, &alias.plugins),
            (CapabilityKind::Skill, &alias.skills),
            (CapabilityKind::Mcp, &alias.mcp),
        ] {
            for name in names {
                selected.insert(kind, name);
            }
        }
        for name in &alias.profiles {
            selected.profile(name, layers, Some(alias), true);
        }
        sessions.push(selected);
    }
    sessions
}
fn reachable(layers: &ConfigLayers, state: &DoctorState, harness: Harness) -> Reachable {
    Reachable {
        members: session_selections(layers, state, harness)
            .into_iter()
            .flat_map(|s| s.members)
            .collect(),
    }
}
/// Logical Plugins reachable through Defaults, Profiles or Aliases on this Harness.
pub fn reachable_plugins(
    layers: &ConfigLayers,
    installed: &[Harness],
    harness: Harness,
) -> BTreeSet<String> {
    reachable_capabilities(layers, installed, harness, CapabilityKind::Plugin)
}

/// Logical Capabilities reachable through Defaults, Profiles or Aliases.
pub fn reachable_capabilities(
    layers: &ConfigLayers,
    installed: &[Harness],
    harness: Harness,
    capability: CapabilityKind,
) -> BTreeSet<String> {
    let state = DoctorState {
        installed: installed.to_vec(),
        ..Default::default()
    };
    reachable(layers, &state, harness)
        .members
        .into_iter()
        .filter_map(|(kind, name)| (kind == capability).then_some(name))
        .collect()
}
fn gap(
    code: &'static str,
    message: String,
    kind: CapabilityKind,
    name: &str,
    path: &std::path::Path,
    harness: Harness,
    reachable: &Reachable,
) -> Diagnostic {
    let mut diagnostic =
        Diagnostic::error(code, message, None).for_capability(harness, kind, name, path);
    if !reachable.contains(kind, name) {
        diagnostic.severity = Severity::Warning;
        if code == "missing-binding" {
            diagnostic.hint = Some(format!(
                "write `{} = false` if this gap is deliberate",
                harness.binary()
            ));
        }
    }
    diagnostic
}
pub fn order(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by(|a, b| {
        let key = |d: &Diagnostic| {
            (
                d.harness
                    .map_or(0, |h| HARNESSES.iter().position(|x| *x == h).unwrap() + 1),
                d.severity,
                d.code,
                d.capability.as_ref().map(|c| c.name.clone()),
                d.item.clone(),
            )
        };
        key(a).cmp(&key(b))
    });
}
pub fn audit(layers: &ConfigLayers, state: &DoctorState) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    diagnostics.extend(
        layers
            .aliases
            .keys()
            .filter_map(|name| crate::activate::check_alias_name(name)),
    );
    for name in layers.aliases.keys() {
        if let Some(path) = state.alias_commands.get(name) {
            diagnostics.push(Diagnostic {
                code: "alias-shadows-command",
                severity: Severity::Warning,
                message: format!("Alias {name} shadows PATH executable {}", path.display()),
                item: Some(Box::new(path.to_string_lossy().into_owned())),
                ..Default::default()
            });
        }
    }
    for name in layers.aliases.keys() {
        if let Err(errors) = (crate::resolve::Request {
            alias: Some(name.clone()),
            ..Default::default()
        })
        .resolve_harness(layers)
        {
            diagnostics.extend(errors.into_iter().filter(|d| d.code != "no-harness"));
        }
    }
    check_harness_args(layers, &mut diagnostics);
    check_profiles(layers, state, &mut diagnostics);
    for harness in HARNESSES {
        let reachable = reachable(layers, state, harness);
        check_bindings(layers, state, harness, &reachable, &mut diagnostics);
        check_duplicates(layers, state, harness, &mut diagnostics);
        if let Some(inventory) = state.harnesses.iter().find(|s| s.harness == harness) {
            check_native_bindings(layers, inventory, &reachable, &mut diagnostics);
            check_connectors(layers, inventory, &mut diagnostics);
        }
        for (kind, entries) in [
            (
                CapabilityKind::Plugin,
                layers
                    .plugins
                    .iter()
                    .map(|(n, s)| (n, &s.path, s.value.binding(harness).is_none()))
                    .collect::<Vec<_>>(),
            ),
            (
                CapabilityKind::Skill,
                layers
                    .skills
                    .iter()
                    .map(|(n, s)| (n, &s.path, s.value.binding(harness).is_none()))
                    .collect(),
            ),
            (
                CapabilityKind::Mcp,
                layers
                    .mcp
                    .iter()
                    .map(|(n, s)| (n, &s.path, s.value.binding(harness).is_none()))
                    .collect(),
            ),
        ] {
            for (name, path, missing) in entries {
                if missing {
                    diagnostics.push(gap(
                        "missing-binding",
                        format!("{kind} {name} has no Binding for {}", harness.binary()),
                        kind,
                        name,
                        path,
                        harness,
                        &reachable,
                    ));
                }
            }
        }
    }
    diagnostics.extend(audit_leaks(state));
    order(&mut diagnostics);
    diagnostics
}

/// Harness Leak checks remain available when invalid config prevents a config audit.
pub fn audit_leaks(state: &DoctorState) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for inventory in &state.harnesses {
        check_leaks(inventory, &mut diagnostics);
    }
    order(&mut diagnostics);
    diagnostics
}

fn check_leaks(state: &HarnessState, diagnostics: &mut Vec<Diagnostic>) {
    // Empty config removes Defaults, Profiles and Aliases too. Use the launch resolver
    // so the rules for protecting project, bundled and enterprise items stay shared.
    let plan = crate::resolve::resolve(
        crate::resolve::Request {
            harness: Some(state.harness),
            ..Default::default()
        },
        &ConfigLayers::default(),
        state.plugins.as_ref().unwrap_or(&Default::default()),
        state.skills.as_ref().unwrap_or(&Default::default()),
        state.mcp.as_ref().unwrap_or(&Default::default()),
    )
    .unwrap_or_else(|_| unreachable!("an empty selection has no Binding or config errors"));
    let mut causes: BTreeMap<String, (Diagnostic, BTreeSet<String>)> = BTreeMap::new();
    for mut diagnostic in plan.diagnostics.into_iter().filter(|d| d.code == "leak") {
        let cause = diagnostic.cause.as_deref().unwrap().to_string();
        if matches!(
            cause.as_str(),
            "claude-synced-plugin" | "codex-remote-plugin"
        ) {
            diagnostic.severity = Severity::Note;
        }
        let items = diagnostic.item.as_deref().map(|item| item.to_string());
        let entry = causes
            .entry(cause)
            .or_insert_with(|| (diagnostic, BTreeSet::new()));
        if let Some(items) = items {
            entry.1.extend(items.split(", ").map(String::from));
        }
    }
    for (cause, (mut diagnostic, items)) in causes {
        if !items.is_empty() {
            let items = items.into_iter().collect::<Vec<_>>().join(", ");
            if diagnostic.item.as_deref().map(String::as_str) != Some(items.as_str()) {
                diagnostic.message = format!("{cause}: items remain visible: {items}");
            }
            diagnostic.item = Some(Box::new(items));
        }
        diagnostics.push(diagnostic);
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum BindingIdentity {
    Native(String),
    Connector(String),
    Directory(PathBuf),
}

fn check_duplicates(
    layers: &ConfigLayers,
    state: &DoctorState,
    harness: Harness,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let directory = |path: &PathBuf| {
        state
            .canonical_paths
            .get(path)
            .cloned()
            .map(BindingIdentity::Directory)
    };
    let mut bindings = Vec::new();
    for (name, plugin) in &layers.plugins {
        let identity = match plugin.value.binding(harness) {
            Some(PluginBinding::Native(id)) => Some(BindingIdentity::Native(id.clone())),
            Some(PluginBinding::Path(path)) => directory(path),
            _ => None,
        };
        bindings.push((CapabilityKind::Plugin, name, &plugin.path, identity));
    }
    for (name, skill) in &layers.skills {
        let identity = match skill.value.binding(harness) {
            Some(SkillBinding::Native(id)) => Some(BindingIdentity::Native(id.clone())),
            Some(SkillBinding::Path(path)) => directory(path),
            _ => None,
        };
        bindings.push((CapabilityKind::Skill, name, &skill.path, identity));
    }
    for (name, server) in &layers.mcp {
        let identity = match server.value.binding(harness) {
            Some(McpBinding::Native(id)) => Some(BindingIdentity::Native(id.clone())),
            Some(McpBinding::Connector(id)) => Some(BindingIdentity::Connector(id.clone())),
            _ => None,
        };
        bindings.push((CapabilityKind::Mcp, name, &server.path, identity));
    }
    let mut seen: BTreeMap<_, Vec<&String>> = BTreeMap::new();
    for (kind, name, layer, identity) in bindings {
        let Some(identity) = identity else { continue };
        let item = match &identity {
            BindingIdentity::Native(id) | BindingIdentity::Connector(id) => id.clone(),
            BindingIdentity::Directory(path) => path.display().to_string(),
        };
        let previous = seen.entry((kind, identity)).or_default();
        for other in previous.iter() {
            diagnostics.push(
                Diagnostic {
                    code: "duplicate-binding",
                    severity: Severity::Note,
                    message: format!("{kind} {other} and {name} bind the same item: {item}"),
                    ..Default::default()
                }
                .for_capability(harness, kind, name, layer)
                .with_item(&item),
            );
        }
        previous.push(name);
    }
}

fn check_profiles(layers: &ConfigLayers, state: &DoctorState, diagnostics: &mut Vec<Diagnostic>) {
    fn visit<'a>(
        name: &'a str,
        layers: &'a ConfigLayers,
        active: &mut Vec<&'a str>,
        done: &mut BTreeSet<&'a str>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if let Some(start) = active.iter().position(|n| *n == name) {
            let mut cycle = active[start..].to_vec();
            cycle.push(name);
            let mut d = Diagnostic::error(
                "profile-cycle",
                format!("Profile cycle: {}", cycle.join(" → ")),
                None,
            );
            d.capability = Some(Box::new(crate::diagnostic::Capability {
                kind: crate::diagnostic::CapabilityKind::Profile,
                name: name.into(),
            }));
            d.layer = layers
                .profiles
                .get(name)
                .map(|p| p.path.display().to_string());
            diagnostics.push(d);
            return;
        }
        if done.contains(name) {
            return;
        }
        if let Some(profile) = layers.profiles.get(name) {
            active.push(name);
            for member in &profile.value.profiles {
                visit(member, layers, active, done, diagnostics);
            }
            active.pop();
        }
        done.insert(name);
    }
    let mut done = BTreeSet::new();
    for name in layers.profiles.keys() {
        visit(name, layers, &mut Vec::new(), &mut done, diagnostics);
    }
    let reachables = HARNESSES.map(|h| reachable(layers, state, h));
    for (name, profile) in &layers.profiles {
        for (kind, names) in [
            (CapabilityKind::Plugin, &profile.value.plugins),
            (CapabilityKind::Skill, &profile.value.skills),
            (CapabilityKind::Mcp, &profile.value.mcp),
            (CapabilityKind::Profile, &profile.value.profiles),
        ] {
            for member in names {
                let exists = match kind {
                    CapabilityKind::Plugin => layers.plugins.contains_key(member),
                    CapabilityKind::Skill => layers.skills.contains_key(member),
                    CapabilityKind::Mcp => layers.mcp.contains_key(member),
                    _ => layers.profiles.contains_key(member),
                };
                if !exists {
                    let mut d = Diagnostic::error(
                        "dangling-ref",
                        format!("Profile {name} references undefined {kind} {member}"),
                        None,
                    );
                    d.capability = Some(Box::new(crate::diagnostic::Capability {
                        kind: crate::diagnostic::CapabilityKind::Profile,
                        name: name.clone(),
                    }));
                    d.layer = Some(profile.path.display().to_string());
                    d.item = Some(Box::new(member.clone()));
                    if !reachables
                        .iter()
                        .any(|r| r.contains(CapabilityKind::Profile, name))
                    {
                        d.severity = Severity::Warning;
                    }
                    diagnostics.push(d);
                }
            }
        }
    }
}

fn check_bindings(
    layers: &ConfigLayers,
    state: &DoctorState,
    harness: Harness,
    reachable: &Reachable,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (name, plugin) in &layers.plugins {
        if let Some(PluginBinding::Path(path)) = plugin.value.binding(harness) {
            diagnostics.extend(check_path(
                CapabilityKind::Plugin,
                name,
                &plugin.path,
                path,
                harness,
                reachable,
                state.paths.contains(path),
            ));
        } else if let Some(PluginBinding::Native(id)) = plugin.value.binding(harness)
            && harness == Harness::Copilot
            && !crate::resolve::valid_copilot_native_id(id)
        {
            let mut d = gap(
                "unsupported-binding",
                format!("Copilot native Plugin Binding {id} must use name@marketplace"),
                CapabilityKind::Plugin,
                name,
                &plugin.path,
                harness,
                reachable,
            );
            d.item = Some(Box::new(id.clone()));
            diagnostics.push(d);
        }
    }
    let sessions = session_selections(layers, state, harness);
    let mut names: BTreeMap<String, Vec<(&str, bool)>> = BTreeMap::new();
    for (logical, skill) in &layers.skills {
        if skill.path.as_os_str().is_empty() && !reachable.contains(CapabilityKind::Skill, logical)
        {
            continue;
        }
        let resolved = match skill.value.binding(harness) {
            Some(SkillBinding::Path(path)) => {
                diagnostics.extend(check_path(
                    CapabilityKind::Skill,
                    logical,
                    &skill.path,
                    path,
                    harness,
                    reachable,
                    state.skill_names.contains_key(path),
                ));
                if harness == Harness::Codex {
                    None
                } else {
                    state.skill_names.get(path).map(|n| (n.clone(), false))
                }
            }
            Some(SkillBinding::Builtin(name)) => {
                if harness == Harness::Codex {
                    let mut d = gap(
                        "unsupported-binding",
                        format!(
                            "Codex cannot load built-in Skill {logical} without an installed snapshot"
                        ),
                        CapabilityKind::Skill,
                        logical,
                        &skill.path,
                        harness,
                        reachable,
                    );
                    d.hint = Some(format!("ayran install --skill {logical} --codex"));
                    diagnostics.push(d);
                    None
                } else {
                    Some((name.clone(), false))
                }
            }
            Some(SkillBinding::Git(binding)) => {
                let mut d = gap(
                    "skill-not-installed",
                    format!("Skill {logical} has no installed owned snapshot"),
                    CapabilityKind::Skill,
                    logical,
                    &skill.path,
                    harness,
                    reachable,
                );
                d.hint = Some(format!("ayran install --skill {logical}"));
                diagnostics.push(d);
                if binding.r#ref.is_none() {
                    diagnostics.push(Diagnostic {
                        severity: Severity::Note,
                        ..Diagnostic::error(
                            "skill-unpinned",
                            format!("Skill {logical} follows the default branch"),
                            None,
                        )
                        .for_capability(
                            harness,
                            CapabilityKind::Skill,
                            logical,
                            &skill.path,
                        )
                    });
                }
                None
            }
            Some(SkillBinding::Native(name)) => Some((name.clone(), true)),
            _ => None,
        };
        if let Some((name, native)) = resolved {
            let previous = names.entry(name.clone()).or_default();
            for (other, other_native) in previous.iter() {
                if native && *other_native {
                    continue;
                }
                let mut d = gap(
                    "skill-name-clash",
                    format!("Skills {other} and {logical} both resolve to {name}"),
                    CapabilityKind::Skill,
                    logical,
                    &skill.path,
                    harness,
                    reachable,
                );
                if !sessions.iter().any(|s| {
                    s.contains(CapabilityKind::Skill, other)
                        && s.contains(CapabilityKind::Skill, logical)
                }) {
                    d.severity = Severity::Warning;
                }
                d.item = Some(Box::new(name.clone()));
                diagnostics.push(d);
            }
            previous.push((logical, native));
        }
    }
    for (name, server) in &layers.mcp {
        if let Some(McpBinding::Connector(id)) = server.value.binding(harness)
            && (harness == Harness::Copilot
                || match harness {
                    Harness::Claude => server.value.claude.is_none(),
                    Harness::Codex => server.value.codex.is_none(),
                    Harness::Copilot => true,
                })
        {
            diagnostics.push(
                gap(
                    "unsupported-binding",
                    format!(
                        "MCP server {name}: connector Binding requires a claude or codex Binding"
                    ),
                    CapabilityKind::Mcp,
                    name,
                    &server.path,
                    harness,
                    reachable,
                )
                .with_item(id),
            );
        }
        if let Some(McpBinding::Definition(McpDefinition::Stdio { command, .. })) =
            server.value.binding(harness)
            && std::path::Path::new(command).is_absolute()
            && !state.paths.contains(&PathBuf::from(command))
        {
            let mut d = gap(
                "path-not-found",
                format!("MCP server {name} command {command} does not exist"),
                CapabilityKind::Mcp,
                name,
                &server.path,
                harness,
                reachable,
            );
            d.item = Some(Box::new(command.clone()));
            diagnostics.push(d);
        }
    }
}
fn check_path(
    kind: CapabilityKind,
    name: &str,
    layer: &std::path::Path,
    path: &std::path::Path,
    harness: Harness,
    reachable: &Reachable,
    exists: bool,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if harness == Harness::Codex {
        let mut d = gap(
            "unsupported-binding",
            format!("{kind} {name} has a path Binding, which codex cannot load"),
            kind,
            name,
            layer,
            harness,
            reachable,
        );
        d.item = Some(Box::new(path.display().to_string()));
        diagnostics.push(d);
    }
    if !exists {
        let mut d = gap(
            "path-not-found",
            format!(
                "{kind} {name} path {} does not exist or cannot be read",
                path.display()
            ),
            kind,
            name,
            layer,
            harness,
            reachable,
        );
        d.item = Some(Box::new(path.display().to_string()));
        diagnostics.push(d);
    }
    diagnostics
}

fn check_connectors(
    layers: &ConfigLayers,
    state: &HarnessState,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let harness = state.harness;
    if !matches!(harness, Harness::Claude | Harness::Codex) {
        return;
    }
    let mut declared = false;
    for (name, server) in &layers.mcp {
        let explicit = match harness {
            Harness::Claude => server.value.claude.as_ref(),
            Harness::Codex => server.value.codex.as_ref(),
            Harness::Copilot => None,
        };
        if let Some(McpBinding::Connector(id)) = explicit {
            declared = true;
            if let Some(servers) = &state.mcp
                && !servers.connectors.contains(id)
            {
                diagnostics.push(
                    Diagnostic {
                        code: "connector-not-seen",
                        severity: Severity::Note,
                        message: format!("Account connector {id} has not been seen in the local cache; it may never have connected or may be misspelt"),
                        ..Default::default()
                    }
                    .for_capability(harness, CapabilityKind::Mcp, name, &server.path)
                    .with_item(id),
                );
            }
        }
    }
    if declared && harness == Harness::Claude {
        diagnostics.push(crate::mcp::claude_connector_leak(Severity::Note));
    }
}

fn check_native_bindings(
    layers: &ConfigLayers,
    state: &HarnessState,
    reachable: &Reachable,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let harness = state.harness;
    let mut missing = Vec::new();
    if let Some(installed) = &state.plugins {
        for (name, plugin) in &layers.plugins {
            if let Some(PluginBinding::Native(id)) = plugin.value.binding(harness) {
                let found = if harness == Harness::Copilot {
                    installed.paths.contains_key(id)
                } else {
                    installed.user.contains(id) || installed.project.contains(id)
                };
                if !found {
                    missing.push((CapabilityKind::Plugin, name, &plugin.path, id));
                }
            }
        }
    }
    if let Some(skills) = &state.skills {
        for (name, skill) in &layers.skills {
            if let Some(SkillBinding::Native(id)) = skill.value.binding(harness)
                && skills.native_name(harness, id).is_none()
            {
                missing.push((CapabilityKind::Skill, name, &skill.path, id));
            }
        }
    }
    if let Some(servers) = &state.mcp {
        for (name, server) in &layers.mcp {
            if let Some(McpBinding::Native(id)) = server.value.binding(harness)
                && !(harness == Harness::Codex && id == "codex_apps")
                && !servers.user.contains(id)
                && !servers.project.contains(id)
            {
                missing.push((CapabilityKind::Mcp, name, &server.path, id));
            }
            if harness == Harness::Codex
                && let Some(McpBinding::Definition(d)) = server.value.binding(harness)
                && (servers.project.contains(name)
                    || servers.user.contains(name)
                        && !servers.user_definitions.get(name).is_some_and(|actual| {
                            crate::mcp::same_definition(actual, &d.native_value(harness), harness)
                        }))
            {
                let mut d = gap(
                    "mcp-definition-collision",
                    format!("MCP definition {name} would merge with a configured Codex MCP server"),
                    CapabilityKind::Mcp,
                    name,
                    &server.path,
                    harness,
                    reachable,
                )
                .with_item(name);
                d.hint = Some(format!(
                    "use codex = \"{name}\" to select the configured server"
                ));
                diagnostics.push(d);
            }
        }
    }
    for (kind, name, layer, id) in missing {
        let installable =
            kind == CapabilityKind::Plugin && layers.marketplace_for_plugin(harness, id).is_some();
        let mut diagnostic = gap(
            if installable {
                "not-installed"
            } else {
                "native-not-found"
            },
            format!(
                "{kind} {name} binds {id}, which was not found on {}",
                harness.binary()
            ),
            kind,
            name,
            layer,
            harness,
            reachable,
        )
        .with_item(id);
        if installable {
            diagnostic.hint = Some(format!("ayran install {name}"));
        }
        diagnostics.push(diagnostic);
    }
}

fn check_harness_args(layers: &ConfigLayers, diagnostics: &mut Vec<Diagnostic>) {
    for harness in HARNESSES {
        let mut sources = Vec::new();
        if let Some(args) = &layers.settings(harness).args {
            sources.push((
                &args.value,
                args.path.display().to_string(),
                Some(args.path.display().to_string()),
            ));
        }
        for (name, alias) in &layers.aliases {
            if alias.effective_harness(layers) == Some(harness)
                && let Some(args) = &alias.args
            {
                sources.push((args, format!("Alias {name}"), None));
            }
        }
        for (name, preset) in &layers.presets {
            if preset.value.harness == harness {
                sources.push((
                    &preset.value.args,
                    format!("Preset {name}"),
                    Some(preset.path.display().to_string()),
                ));
            }
        }
        for (args, source, layer) in sources {
            if harness.args_overlap_model_or_effort(args) {
                diagnostics.push(Diagnostic {
                    code: "harness-args-overlap",
                    severity: Severity::Warning,
                    harness: Some(harness),
                    layer,
                    message: format!(
                        "{source}: Harness args contain a model or Effort flag managed by ayran"
                    ),
                    ..Default::default()
                });
            }
        }
    }
}
