use ayran_core::harness::Harness;
use clap::{Arg, ArgAction, ArgMatches, Command};

pub fn command() -> Command {
    let mut command = Command::new("doctor").about("Audit config and Harness state");
    for harness in ["claude", "codex", "copilot"] {
        command = command.arg(Arg::new(harness).long(harness).action(ArgAction::SetTrue));
    }
    command
        .group(
            clap::ArgGroup::new("harness-choice").args(["claude", "codex", "copilot", "harness"]),
        )
        .arg(
            Arg::new("harness")
                .long("harness")
                .value_parser(["claude", "codex", "copilot"]),
        )
        .arg(
            Arg::new("quiet")
                .long("quiet")
                .short('q')
                .action(ArgAction::SetTrue),
        )
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue))
}
pub fn run(matches: &ArgMatches) -> i32 {
    use ayran_core::{
        config::ConfigLayers,
        diagnostic::Severity,
        doctor::{DoctorState, HARNESSES, audit},
    };
    let filter = HARNESSES.into_iter().find(|h| {
        matches.get_flag(h.binary())
            || matches
                .get_one::<String>("harness")
                .is_some_and(|v| v == h.binary())
    });
    let mut state = DoctorState {
        installed: HARNESSES
            .into_iter()
            .filter(|h| crate::program_on_path(std::ffi::OsStr::new(h.binary())).is_some())
            .collect(),
        ..DoctorState::default()
    };
    let real_home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from);
    let mut plugin_inventories = Vec::new();
    let mut headings = std::collections::BTreeMap::new();
    let (layers, mut diagnostics) = ConfigLayers::load_for_doctor();
    for harness in HARNESSES
        .into_iter()
        .filter(|h| filter.is_none_or(|f| f == *h))
    {
        match crate::harness_home::HarnessHome::resolve(
            harness,
            layers.settings(harness).home_mode,
            real_home.as_deref(),
        ) {
            Ok(home) => {
                let isolated = home.mode == ayran_core::launch::HomeMode::Isolated;
                let missing = isolated
                    && matches!(std::fs::metadata(&home.directory),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound);
                let label = if missing {
                    "isolated home, not created yet: "
                } else if isolated {
                    "isolated home: "
                } else {
                    "home: "
                };
                if state.installed.contains(&harness) {
                    let mut inventory = read_harness(
                        harness,
                        &home,
                        real_home.as_deref(),
                        missing,
                        layers.mcp.values().any(|server| {
                            matches!(
                                server.value.codex,
                                Some(ayran_core::mcp::McpBinding::Connector(_))
                            )
                        }),
                        &mut diagnostics,
                    );
                    // Copilot launch loads declared native Plugins from the shared home.
                    if harness == Harness::Copilot
                        && isolated
                        && let Some(plugins) = &mut inventory.plugins
                    {
                        match crate::harness_home::HarnessHome::resolve(
                            harness,
                            ayran_core::launch::HomeMode::Shared,
                            real_home.as_deref(),
                        ) {
                            Ok(shared) => {
                                let bindings = layers.plugins.values().filter_map(|plugin| {
                                    match plugin.value.binding(harness) {
                                        Some(ayran_core::config::PluginBinding::Native(id)) => {
                                            Some(id.as_str())
                                        }
                                        _ => None,
                                    }
                                });
                                plugins.paths = crate::plugin_enumeration::copilot_native_paths(
                                    &shared.directory,
                                    bindings,
                                );
                            }
                            Err(mut d) => {
                                d.harness = Some(harness);
                                diagnostics.push(d);
                                inventory.plugins = None;
                            }
                        }
                    }
                    if let Some(plugins) = &inventory.plugins {
                        match crate::plugin_contents::read(harness, &home, plugins) {
                            Ok(contents) => {
                                if harness != Harness::Claude
                                    && let Some(mcp) = &mut inventory.mcp
                                {
                                    mcp.plugins = contents
                                        .plugins
                                        .iter()
                                        .flat_map(|p| p.servers.keys().cloned())
                                        .collect();
                                }
                                plugin_inventories.push((harness, contents));
                            }
                            Err(mut d) => {
                                d.harness = Some(harness);
                                diagnostics.push(d);
                            }
                        }
                    }
                    state.harnesses.push(inventory);
                }
                headings.insert(
                    harness.binary(),
                    format!("{} ({label}{})", harness.binary(), home.directory.display()),
                );
            }
            Err(mut d) => {
                if state.installed.contains(&harness) {
                    d.harness = Some(harness);
                    diagnostics.push(d);
                }
            }
        }
        if let Some(binary) = crate::program_on_path(std::ffi::OsStr::new(harness.binary())) {
            if let Err(mut diagnostic) =
                crate::harness_version::check_fresh(std::ffi::OsStr::new(harness.binary()), &binary)
            {
                diagnostic.harness = Some(harness);
                diagnostics.push(diagnostic);
            }
        } else {
            diagnostics.push(ayran_core::diagnostic::Diagnostic {
                code: "harness-not-found",
                severity: if filter.is_some() {
                    Severity::Error
                } else {
                    Severity::Note
                },
                message: format!("Harness {} was not found on PATH", harness.binary()),
                harness: Some(harness),
                ..Default::default()
            });
        }
    }

    if !diagnostics.iter().any(|d| d.code == "config-invalid") {
        let own_binary = std::env::current_exe()
            .ok()
            .and_then(|path| path.canonicalize().ok());
        for name in layers.aliases.keys() {
            if let Some(path) = crate::program_on_path(std::ffi::OsStr::new(name))
                && !own_binary
                    .as_ref()
                    .is_some_and(|own| path.canonicalize().ok().as_ref() == Some(own))
            {
                state.alias_commands.insert(name.clone(), path);
            }
        }
        read_paths(&layers, &mut state);
        diagnostics.extend(audit(&layers, &state));
    } else {
        diagnostics.extend(ayran_core::doctor::audit_leaks(&state));
    }
    let audit_layers = if diagnostics.iter().any(|d| d.code == "config-invalid") {
        ConfigLayers::default()
    } else {
        layers
    };
    for (harness, contents) in plugin_inventories {
        diagnostics.extend(ayran_core::plugin_audit::audit(
            harness,
            &contents,
            &audit_layers,
            &state,
        ));
    }
    ayran_core::doctor::order(&mut diagnostics);
    diagnostics.retain(|d| filter.is_none() || d.harness.is_none() || d.harness == filter);
    let errors = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let warnings = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .count();
    let notes = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Note)
        .count();
    if matches.get_flag("json") {
        println!(
            "{}",
            serde_json::json!({"version":1,"diagnostics":diagnostics,"summary":{"errors":errors,"warnings":warnings,"notes":notes}})
        );
    } else {
        for group in std::iter::once(None).chain(
            HARNESSES
                .into_iter()
                .filter(|h| filter.is_none_or(|f| f == *h))
                .map(Some),
        ) {
            println!(
                "{}",
                group
                    .and_then(|h| headings.get(h.binary()).map(String::as_str))
                    .unwrap_or_else(|| group.map_or("config", Harness::binary))
            );
            for d in diagnostics.iter().filter(|d| {
                d.harness == group && (!matches.get_flag("quiet") || d.severity == Severity::Error)
            }) {
                println!("{}[{}]: {}", d.severity, d.code, d.message);
                if let Some(hint) = &d.hint {
                    println!("  hint: {hint}");
                }
            }
        }
        if diagnostics.is_empty() {
            println!("no problems found");
        } else {
            println!(
                "{errors} {}, {warnings} {}, {notes} {}",
                plural(errors, "error", "errors"),
                plural(warnings, "warning", "warnings"),
                plural(notes, "note", "notes")
            );
        }
    }
    i32::from(errors > 0)
}
fn plural(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        singular.into()
    } else {
        plural.into()
    }
}

fn read_paths(
    layers: &ayran_core::config::ConfigLayers,
    state: &mut ayran_core::doctor::DoctorState,
) {
    use ayran_core::{
        config::{PluginBinding, SkillBinding},
        doctor::HARNESSES,
        mcp::{McpBinding, McpDefinition},
    };
    for harness in HARNESSES {
        for plugin in layers.plugins.values() {
            if let Some(PluginBinding::Path(path)) = plugin.value.binding(harness)
                && path.is_dir()
            {
                state.paths.insert(path.clone());
                if let Ok(canonical) = path.canonicalize() {
                    state.canonical_paths.insert(path.clone(), canonical);
                }
            }
        }
        for skill in layers.skills.values() {
            if let Some(SkillBinding::Path(path)) = skill.value.binding(harness)
                && path.is_dir()
                && let Ok(canonical) = path.canonicalize()
            {
                state.canonical_paths.insert(path.clone(), canonical);
            }
            if let Some(SkillBinding::Path(path)) = skill.value.binding(harness)
                && let Ok(contents) = std::fs::read_to_string(path.join("SKILL.md"))
                && let Ok(name) = crate::skill_activation::skill_name(path, &contents)
            {
                state.skill_names.insert(path.clone(), name);
            }
        }
        for server in layers.mcp.values() {
            if let Some(McpBinding::Definition(McpDefinition::Stdio { command, .. })) =
                server.value.binding(harness)
                && std::path::Path::new(command).exists()
            {
                state.paths.insert(command.into());
            }
        }
    }
}

fn read_harness(
    harness: Harness,
    home: &crate::harness_home::HarnessHome,
    real_home: Option<&std::path::Path>,
    missing: bool,
    codex_connectors_declared: bool,
    diagnostics: &mut Vec<ayran_core::diagnostic::Diagnostic>,
) -> ayran_core::doctor::HarnessState {
    fn inventory<T>(
        result: Result<T, ayran_core::diagnostic::Diagnostic>,
        harness: Harness,
        diagnostics: &mut Vec<ayran_core::diagnostic::Diagnostic>,
    ) -> Option<T> {
        match result {
            Ok(state) => Some(state),
            Err(mut d) => {
                d.harness = Some(harness);
                diagnostics.push(d);
                None
            }
        }
    }
    if missing {
        return ayran_core::doctor::HarnessState {
            harness,
            plugins: Some(Default::default()),
            skills: Some(Default::default()),
            mcp: Some(Default::default()),
        };
    }
    let plugins = inventory(
        crate::plugin_enumeration::read(harness, real_home, home),
        harness,
        diagnostics,
    );
    let mut skills = ayran_core::skills::SkillState::default();
    let result = match harness {
        Harness::Claude => crate::skill_enumeration::read_claude(&mut skills, real_home, home),
        Harness::Codex => crate::codex_skill_enumeration::read(&mut skills, real_home, home),
        Harness::Copilot => crate::copilot_skill_enumeration::read(&mut skills, real_home, home),
    };
    let skills = inventory(result.map(|()| skills), harness, diagnostics);
    let mcp = inventory(
        match harness {
            Harness::Claude => crate::mcp_enumeration::read_claude(home),
            Harness::Codex => {
                crate::mcp_enumeration::read_codex(home, real_home).and_then(|mut state| {
                    if codex_connectors_declared {
                        state.connectors = crate::mcp_enumeration::read_codex_connectors(home)?;
                    }
                    Ok(state)
                })
            }
            Harness::Copilot => crate::copilot_mcp_enumeration::read(home),
        },
        harness,
        diagnostics,
    );
    ayran_core::doctor::HarnessState {
        harness,
        plugins,
        skills,
        mcp,
    }
}
