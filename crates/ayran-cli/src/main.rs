mod claude_config;
mod cli;
mod codex_config;
mod codex_link;
mod codex_native_write;
mod codex_skill_enumeration;
mod completion;
mod config_command;
mod copilot_mcp_enumeration;
mod copilot_skill_enumeration;
mod doctor_command;
mod generated_cache;
mod harness_home;
mod harness_version;
mod install_command;
mod install_mcp;
mod install_plan;
mod install_skills;
mod install_write;
mod list_command;
mod marketplace_list;
mod marketplace_state;
mod mcp_activation;
mod mcp_enumeration;
mod native_capabilities;
mod native_command;
mod native_marketplaces;
mod native_mcp;
mod native_plugins;
mod native_skill_write;
mod native_skills;
mod plugin_contents;
mod plugin_enumeration;
mod session_command;
mod session_list;
mod session_store;
mod skill_activation;
mod skill_enumeration;
mod skill_git;
mod skill_snapshot;
mod trust;
mod update_command;

use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{self, Command as ProcessCommand};

use ayran_core::activate::{Shell, render_activation};
use ayran_core::config::ConfigLayers;
use ayran_core::diagnostic::{Diagnostic, Severity};
use ayran_core::harness::{Effort, Harness};
use ayran_core::launch::LaunchPlan;
use ayran_core::resolve::{Request, resolve};
use clap::error::ErrorKind;

fn usage(message: impl Into<String>) -> Diagnostic {
    Diagnostic::error("usage", message, None)
}

fn run() -> i32 {
    let raw: Vec<OsString> = env::args_os().collect();
    // Handle completion before parsing so even malformed requests stay silent.
    if raw.get(1).is_some_and(|word| word == "__complete") {
        completion::run(&raw[2..]);
        return 0;
    }
    let matches = match cli::command().try_get_matches_from(&raw) {
        Ok(matches) => matches,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            print!("{error}");
            return 0;
        }
        Err(error) => {
            let clap_message = error.to_string();
            let positional = error.kind() == ErrorKind::UnknownArgument
                && raw
                    .iter()
                    .skip(1)
                    .take_while(|arg| *arg != "--")
                    .any(|arg| {
                        let word = arg.to_string_lossy();
                        !word.starts_with('-')
                            && clap_message.contains(&format!("unexpected argument '{word}' found"))
                    });
            let message = if positional {
                "unknown subcommand; Harness args go after --".to_owned()
            } else {
                clap_message.trim().to_owned()
            };
            render(&usage(message), false);
            return 2;
        }
    };

    if let Some(("config", config)) = matches.subcommand() {
        return config_command::run(config);
    }
    if let Some(("update", update)) = matches.subcommand() {
        return update_command::run(update);
    }
    if let Some(("install", install)) = matches.subcommand() {
        return install_command::run(install);
    }
    if let Some(("trust", trust)) = matches.subcommand() {
        return trust::run(trust);
    }
    if let Some(("native", native)) = matches.subcommand() {
        std::process::exit(native_command::run(native));
    }
    if let Some(("doctor", doctor)) = matches.subcommand() {
        return doctor_command::run(doctor);
    }
    if let Some(("list", list)) = matches.subcommand() {
        return list_command::run(list);
    }
    let resume_matches = matches.subcommand_matches("resume");
    let matches = resume_matches.unwrap_or(&matches);
    let quiet = matches.get_flag("quiet");
    if let Some(("activate", activation)) = matches.subcommand() {
        let shell = match activation.get_one::<String>("shell").map(String::as_str) {
            Some("zsh") => Shell::Zsh,
            Some("bash") => Shell::Bash,
            Some("pwsh") => Shell::Pwsh,
            _ => unreachable!("clap validates shell values"),
        };
        let layers = match ConfigLayers::load_user() {
            Ok(layers) => layers,
            Err(diagnostic) => {
                render(&diagnostic, quiet);
                return 3;
            }
        };
        match render_activation(shell, &layers.aliases) {
            Ok(output) => {
                print!("{output}");
                return 0;
            }
            Err(diagnostic) => {
                render(&diagnostic, quiet);
                return 3;
            }
        }
    }
    let mut chosen = Vec::new();
    for (name, harness) in [
        ("claude", Harness::Claude),
        ("codex", Harness::Codex),
        ("copilot", Harness::Copilot),
    ] {
        for _ in 0..matches.get_count(name) {
            chosen.push(harness);
        }
    }
    if let Some(values) = matches.get_many::<String>("harness") {
        for value in values {
            chosen.push(match value.as_str() {
                "claude" => Harness::Claude,
                "codex" => Harness::Codex,
                "copilot" => Harness::Copilot,
                _ => unreachable!("clap validates Harness values"),
            });
        }
    }
    if chosen.len() > 1 {
        let message = if chosen.iter().all(|harness| *harness == chosen[0]) {
            "Harness flags may appear at most once"
        } else {
            "conflicting Harness flags"
        };
        render(&usage(message), quiet);
        return 2;
    }
    for name in ["model", "effort", "alias"] {
        if matches
            .get_many::<String>(name)
            .is_some_and(|values| values.len() > 1)
        {
            render(&usage(format!("--{name} may appear at most once")), quiet);
            return 2;
        }
    }

    let effort = matches
        .get_one::<String>("effort")
        .map(|value| match value.as_str() {
            "low" => Effort::Low,
            "medium" => Effort::Medium,
            "high" => Effort::High,
            "xhigh" => Effort::Xhigh,
            "max" => Effort::Max,
            _ => unreachable!("clap validates Effort values"),
        });

    let mut resumed = if let Some(resume) = resume_matches {
        match session_command::prepare_resume(resume, chosen.first().copied()) {
            Ok(record) => Some(record),
            Err(diagnostic) => {
                let status = if diagnostic.code == "usage" { 2 } else { 3 };
                render(&diagnostic, quiet);
                return status;
            }
        }
    } else {
        None
    };
    let mut layers = match ConfigLayers::load() {
        Ok(layers) => layers,
        Err(diagnostic) => {
            render(&diagnostic, quiet);
            return 3;
        }
    };
    let mut request = Request {
        alias: matches.get_one::<String>("alias").cloned(),
        harness: chosen.into_iter().next(),
        model: matches.get_one::<String>("model").cloned(),
        effort,
        mcp: matches
            .get_many::<String>("mcp")
            .map(|values| values.cloned().collect())
            .unwrap_or_default(),
        skills: matches
            .get_many::<String>("skill")
            .map(|values| values.cloned().collect())
            .unwrap_or_default(),
        plugins: matches
            .get_many::<String>("plugin")
            .map(|values| values.cloned().collect())
            .unwrap_or_default(),
        profiles: matches
            .get_many::<String>("profile")
            .map(|values| values.cloned().collect())
            .unwrap_or_default(),
        no_profiles: matches
            .get_many::<String>("no-profile")
            .map(|values| values.cloned().collect())
            .unwrap_or_default(),
        no_mcp: matches
            .get_many::<String>("no-mcp")
            .map(|values| values.cloned().collect())
            .unwrap_or_default(),
        no_skills: matches
            .get_many::<String>("no-skill")
            .map(|values| values.cloned().collect())
            .unwrap_or_default(),
        no_plugins: matches
            .get_many::<String>("no-plugin")
            .map(|values| values.cloned().collect())
            .unwrap_or_default(),
        no_defaults: matches.get_flag("no-defaults"),
        no_harness_args: matches.get_flag("no-harness-args"),
        passthrough: matches
            .get_many::<OsString>("passthrough")
            .map(|values| values.cloned().collect())
            .unwrap_or_default(),
    };
    if let Some(record) = &resumed {
        request = match session_command::replay(record, request, &layers) {
            Ok(request) => request,
            Err(diagnostic) => {
                render(&diagnostic, quiet);
                return 3;
            }
        };
    }
    let real_home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let mut resolved_home = None;
    let resolution = request.resolve_harness(&layers).and_then(|(harness, _)| {
        let home = harness_home::HarnessHome::resolve(
            harness,
            resumed
                .as_ref()
                .map(|record| record.home)
                .unwrap_or(layers.settings(harness).home_mode),
            real_home.as_deref(),
        )
        .map_err(|diagnostic| vec![diagnostic])?;
        resolved_home = Some(home);
        let home = resolved_home.as_ref().unwrap();
        let mut installed = plugin_enumeration::read(harness, real_home.as_deref(), home)
            .map_err(|diagnostic| vec![diagnostic])?;
        if harness == Harness::Copilot && home.mode == ayran_core::launch::HomeMode::Isolated {
            let shared = harness_home::HarnessHome::resolve(
                harness,
                ayran_core::launch::HomeMode::Shared,
                real_home.as_deref(),
            )
            .map_err(|diagnostic| vec![diagnostic])?;
            let bindings = layers.plugins.values().filter_map(|plugin| {
                match plugin.value.binding(harness)? {
                    ayran_core::config::PluginBinding::Native(id) => Some(id.as_str()),
                    _ => None,
                }
            });
            installed.paths = plugin_enumeration::copilot_native_paths(&shared.directory, bindings)
                .map_err(|diagnostic| vec![diagnostic])?;
        }
        if !installed.direct.is_empty() {
            let selected_paths = layers.plugins.values().filter_map(|plugin| {
                match plugin.value.binding(harness)? {
                    ayran_core::config::PluginBinding::Path(path) => Some(path),
                    _ => None,
                }
            });
            for path in installed.direct.iter().chain(selected_paths) {
                if let Ok(canonical) = path.canonicalize() {
                    installed.canonical_paths.insert(path.clone(), canonical);
                }
            }
        }
        let snapshot_diagnostics = skill_snapshot::apply(&mut layers, harness, &home.directory);
        let selected = ayran_core::skills::select(&request, &layers, harness)?;
        let names = selected.names();
        let mut skills = skill_activation::read(&names, &layers, harness)
            .map_err(|diagnostic| vec![diagnostic])?;
        if harness == Harness::Claude {
            skill_enumeration::read_claude(&mut skills, real_home.as_deref(), home)
                .map_err(|diagnostic| vec![diagnostic])?;
        } else if harness == Harness::Codex {
            codex_skill_enumeration::read(&mut skills, real_home.as_deref(), home)
                .map_err(|diagnostic| vec![diagnostic])?;
        } else if harness == Harness::Copilot {
            copilot_skill_enumeration::read(&mut skills, real_home.as_deref(), home)
                .map_err(|diagnostic| vec![diagnostic])?;
        }
        let mut mcp = if harness == Harness::Claude {
            mcp_enumeration::read_claude(home).map_err(|diagnostic| vec![diagnostic])?
        } else if harness == Harness::Codex {
            mcp_enumeration::read_codex(home, real_home.as_deref())
                .map_err(|diagnostic| vec![diagnostic])?
        } else {
            copilot_mcp_enumeration::read(home).map_err(|diagnostic| vec![diagnostic])?
        };
        if harness == Harness::Claude {
            claude_config::apply(
                home,
                &request.trailing_args(&layers, harness),
                &mut installed,
                &mut skills,
                &mut mcp,
            )
            .map_err(|diagnostic| vec![diagnostic])?;
        } else if harness == Harness::Codex {
            let profiles = codex_config::profiles(&request.trailing_args(&layers, harness));
            codex_config::apply(
                home,
                real_home.as_deref(),
                &profiles,
                &mut installed,
                &mut skills,
                &mut mcp,
            )
            .map_err(|diagnostic| vec![diagnostic])?;
        }
        let plan = resolve(request.clone(), &layers, &installed, &skills, &mcp)?;
        let plan = if harness == Harness::Codex && !plan.native_plugins.is_empty() {
            // Resolve selections before reading payloads: unselected Plugins are irrelevant.
            mcp_enumeration::read_codex_plugins(home, &plan.native_plugins, &mut mcp)
                .map_err(|diagnostic| vec![diagnostic])?;
            resolve(request.clone(), &layers, &installed, &skills, &mcp)
        } else if harness == Harness::Copilot {
            let native_paths = plan
                .native_plugins
                .iter()
                .filter_map(|id| installed.paths.get(id));
            copilot_mcp_enumeration::read_plugins(
                native_paths.chain(plan.plugin_paths.iter()),
                &mut mcp,
            )
            .map_err(|diagnostic| vec![diagnostic])?;
            resolve(request.clone(), &layers, &installed, &skills, &mcp)
        } else {
            Ok(plan)
        };
        plan.map(|mut plan| {
            plan.diagnostics
                .extend(snapshot_diagnostics.into_iter().filter(|d| {
                    d.capability
                        .as_ref()
                        .is_some_and(|c| names.contains(&c.name))
                }));
            (harness, plan)
        })
    });
    let (harness, mut plan) = match resolution {
        Ok(plan) => plan,
        Err(diagnostics) => {
            let status = if diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "usage")
            {
                2
            } else {
                3
            };
            for mut diagnostic in diagnostics {
                if let Some(record) = &resumed {
                    session_command::recovery_hint(record, &layers, &mut diagnostic);
                }
                render(&diagnostic, quiet);
            }
            return status;
        }
    };

    let home = resolved_home.unwrap();
    let mut session_record = match session_command::prepare_record(
        resumed.take(),
        resume_matches.is_some_and(|matches| matches.get_flag("fork")),
        &request,
        &layers,
        harness,
        &mut plan,
    ) {
        Ok(record) => record,
        Err(diagnostic) => {
            render(&diagnostic, quiet);
            return 3;
        }
    };
    if plan.home_mode == ayran_core::launch::HomeMode::Isolated {
        plan.env_set.push((
            home.variable.into(),
            home.directory.clone().into_os_string(),
        ));
    }

    for path in &plan.plugin_paths {
        if !path.exists() {
            render(
                &Diagnostic::error(
                    "path-not-found",
                    format!(
                        "Plugin path {} does not exist or cannot be accessed",
                        path.display()
                    ),
                    None,
                ),
                quiet,
            );
            return 3;
        }
    }

    let mcp_cache = match mcp_activation::prepare(&mut plan) {
        Ok(cache) => cache,
        Err(diagnostic) => {
            render(&diagnostic, quiet);
            return 3;
        }
    };
    let Some(binary) = program_on_path(&plan.program) else {
        render(
            &Diagnostic::error(
                "harness-not-found",
                format!(
                    "Harness {} not found on PATH",
                    plan.program.to_string_lossy()
                ),
                Some("install the Harness or add it to PATH"),
            ),
            quiet,
        );
        return 3;
    };
    let version = match harness_version::check(&plan.program, &binary, !matches.get_flag("dry-run"))
    {
        Ok(version) => version,
        Err(diagnostic) => {
            render(&diagnostic, quiet);
            return 3;
        }
    };
    for diagnostic in &plan.diagnostics {
        render(diagnostic, quiet);
    }
    if matches.get_flag("dry-run") {
        let cwd = env::current_dir().unwrap();
        if matches.get_flag("json") {
            let diagnostics = &plan.diagnostics;
            let argv: Vec<_> = std::iter::once(&plan.program)
                .chain(plan.args.iter())
                .map(|arg| arg.to_string_lossy())
                .collect();
            let environment: std::collections::BTreeMap<_, _> = plan
                .env_set
                .iter()
                .map(|(name, value)| (name.to_string_lossy(), value.to_string_lossy()))
                .collect();
            println!(
                "{}",
                serde_json::json!({ "version": 1, "argv": argv, "env": environment,
                "env_remove": plan.env_remove.iter().map(|name| name.to_string_lossy()).collect::<Vec<_>>(),
                "cwd": cwd, "session_id": session_record.as_ref().map(|record| &record.id), "diagnostics": diagnostics })
            );
            return 0;
        }
        let line = dry_run_line(&plan);
        if resume_matches.is_some() {
            println!("cd {} && {line}", shell_quote(cwd.as_os_str()));
        } else {
            println!("{line}");
        }
        if quiet {
            return 0;
        }
        if let Some(record) = &session_record {
            if resume_matches.is_some() {
                eprintln!("Session ID: {}", record.id);
            } else {
                eprintln!("Session ID: {} (not recorded)", record.id);
            }
        }
        eprintln!("Harness: {}", plan.trace.harness);
        eprintln!("Harness version: {version}");
        eprintln!("Model: {}", plan.trace.model);
        eprintln!("Effort: {}", plan.trace.effort);
        if plan.home_mode == ayran_core::launch::HomeMode::Isolated {
            let status = if home.directory.is_dir() {
                "exists"
            } else {
                "not created yet"
            };
            eprintln!("Isolated home: {} ({status})", home.directory.display());
        }
        for entry in plan
            .trace
            .profiles
            .iter()
            .chain(&plan.trace.plugins)
            .chain(&plan.trace.skills)
            .chain(&plan.mcp.trace)
            .chain(&plan.trace.harness_args)
        {
            eprintln!("{entry}");
        }
        if let Some(cache) = &mcp_cache {
            let status = if cache.directory.is_dir() {
                "exists"
            } else {
                "not created yet"
            };
            eprintln!("MCP cache: {} ({status})", cache.directory.display());
        }
        if let Some(config) = &plan.mcp.config {
            eprintln!("MCP config: {config}");
        }
        return 0;
    }
    if let Some(cache) = &mcp_cache
        && let Err(diagnostic) = mcp_activation::materialize(cache)
    {
        render(&diagnostic, quiet);
        return 3;
    }
    if let Some(cache) = &plan.generated_skills
        && let Err(diagnostic) = skill_activation::materialize(cache)
    {
        render(&diagnostic, quiet);
        return 3;
    }
    let current_caches: Vec<_> = plan
        .generated_skills
        .as_ref()
        .map(|cache| cache.directory.as_path())
        .into_iter()
        .chain(mcp_cache.as_ref().map(|cache| cache.directory.as_path()))
        .collect();
    generated_cache::prune(&current_caches);
    if let Err(diagnostic) = home.materialize() {
        render(&diagnostic, quiet);
        return 3;
    }
    plan.program = binary.into_os_string();
    let session_write = match session_command::persist(
        session_record.as_mut(),
        resume_matches.is_some_and(|matches| matches.get_flag("fork")),
    ) {
        Ok(write) => write,
        Err(diagnostic) => {
            render(&diagnostic, quiet);
            return 3;
        }
    };
    // A Fork touches its parent before pruning, so an old parent stays resumable.
    session_store::prune(session_record.as_ref().map(|record| record.id.as_str()));
    launch(plan, session_write)
}

fn render(diagnostic: &Diagnostic, quiet: bool) {
    if quiet && !matches!(diagnostic.severity, Severity::Error) {
        return;
    }
    eprintln!(
        "ayran: {}[{}]: {}",
        diagnostic.severity, diagnostic.code, diagnostic.message
    );
    if let Some(hint) = &diagnostic.hint {
        eprintln!("  hint: {hint}");
    }
}

fn shell_quote(value: &OsStr) -> String {
    let value = value.to_string_lossy();
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_./:-".contains(character))
    {
        value.into_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

fn dry_run_line(plan: &LaunchPlan) -> String {
    let mut words = Vec::new();
    if !plan.env_remove.is_empty() {
        words.push("env".to_owned());
        for variable in &plan.env_remove {
            words.push("-u".to_owned());
            words.push(shell_quote(variable));
        }
    }
    for (name, value) in &plan.env_set {
        words.push(format!("{}={}", shell_quote(name), shell_quote(value)));
    }
    words.push(shell_quote(&plan.program));
    words.extend(plan.args.iter().map(|arg| shell_quote(arg)));
    words.join(" ")
}

fn program_on_path(program: &OsStr) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    let binary =
        env::split_paths(&path).find_map(|directory| executable_path(&directory.join(program)))?;
    if binary.is_absolute() {
        Some(binary)
    } else {
        Some(env::current_dir().ok()?.join(binary))
    }
}

#[cfg(unix)]
fn executable_path(path: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .ok()
        .filter(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .map(|_| path.to_path_buf())
}

#[cfg(windows)]
fn executable_path(path: &Path) -> Option<PathBuf> {
    let extensions = env::var_os("PATHEXT").unwrap_or_else(|| ".EXE;.BAT;.CMD;.COM".into());
    extensions
        .to_string_lossy()
        .split(';')
        .find_map(|extension| {
            let mut candidate = path.as_os_str().to_os_string();
            candidate.push(extension.to_ascii_lowercase());
            let candidate = PathBuf::from(candidate);
            candidate.is_file().then_some(candidate)
        })
}

#[cfg(unix)]
fn launch(plan: LaunchPlan, session_writes: Vec<session_store::SessionWrite>) -> i32 {
    use std::os::unix::process::CommandExt;
    let error = prepare_command(plan).exec();
    for write in session_writes.iter().rev() {
        if let Err(diagnostic) = write.rollback() {
            render(&diagnostic, false);
        }
    }
    render(
        &Diagnostic::error("harness-not-found", error.to_string(), None),
        false,
    );
    3
}

#[cfg(windows)]
fn launch(plan: LaunchPlan, session_writes: Vec<session_store::SessionWrite>) -> i32 {
    match prepare_command(plan).status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            for write in session_writes.iter().rev() {
                if let Err(diagnostic) = write.rollback() {
                    render(&diagnostic, false);
                }
            }
            render(
                &Diagnostic::error("harness-not-found", error.to_string(), None),
                false,
            );
            3
        }
    }
}

fn prepare_command(plan: LaunchPlan) -> ProcessCommand {
    let mut command = ProcessCommand::new(plan.program);
    command.args(plan.args);
    for (name, value) in plan.env_set {
        command.env(name, value);
    }
    for name in plan.env_remove {
        command.env_remove(name);
    }
    command
}

fn main() {
    process::exit(run());
}
