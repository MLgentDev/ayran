use ayran_core::{config::ConfigLayers, harness::Harness};
use clap::{Arg, ArgAction, ArgMatches, Command};

pub fn command() -> Command {
    let mut command = Command::new("install")
        .about("Install declared Plugins, Skills or MCP definitions")
        .arg(Arg::new("plugins").num_args(0..))
        .arg(
            Arg::new("install-skill")
                .long("skill")
                .num_args(1..)
                .action(ArgAction::Append),
        )
        .arg(
            Arg::new("install-mcp")
                .long("mcp")
                .num_args(1..)
                .action(ArgAction::Append),
        );
    for h in ["claude", "codex", "copilot"] {
        command = command.arg(Arg::new(h).long(h).action(ArgAction::SetTrue));
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
            Arg::new("dry-run")
                .long("dry-run")
                .action(ArgAction::SetTrue),
        )
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue))
}
pub fn run(matches: &ArgMatches) -> i32 {
    let selected = [Harness::Claude, Harness::Codex, Harness::Copilot]
        .into_iter()
        .find(|h| {
            matches.get_flag(h.binary())
                || matches
                    .get_one::<String>("harness")
                    .is_some_and(|s| s == h.binary())
        });
    let names = matches
        .get_many::<String>("plugins")
        .map(|v| v.cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let plan = match ConfigLayers::load() {
        Ok(layers) => {
            let mcp = matches
                .get_many::<String>("install-mcp")
                .map(|v| v.cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            let skills = matches
                .get_many::<String>("install-skill")
                .map(|v| v.cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            let mut plan = if (mcp.is_empty() && skills.is_empty()) || !names.is_empty() {
                crate::install_plan::build(&layers, &names, selected)
            } else {
                crate::install_plan::Plan::default()
            };
            crate::install_mcp::plan(&layers, &mcp, selected, &mut plan);
            use std::io::IsTerminal;
            let fetch = if matches.get_flag("dry-run") {
                crate::install_skills::FetchMode::Preview
            } else if std::io::stdin().is_terminal() && !matches.get_flag("json") {
                crate::install_skills::FetchMode::Interactive
            } else {
                crate::install_skills::FetchMode::NonInteractive
            };
            crate::install_skills::plan(&layers, &skills, selected, &mut plan, fetch);
            if !matches.get_flag("dry-run") && !plan.has_errors() {
                crate::install_write::execute(&layers, &mut plan);
                if !plan.has_errors() {
                    crate::install_mcp::execute(&layers, &mut plan);
                } else {
                    crate::install_mcp::skip_pending(&mut plan);
                }
            }
            if !matches.get_flag("dry-run") && !plan.has_errors() {
                crate::install_skills::execute(&mut plan);
            } else if !matches.get_flag("dry-run") && plan.has_errors() {
                crate::install_skills::skip_pending(&mut plan);
            }
            plan
        }
        Err(d) => crate::install_plan::Plan {
            diagnostics: vec![d],
            ..Default::default()
        },
    };
    if matches.get_flag("json") {
        let mut output = crate::list_command::json_envelope(&plan.diagnostics);
        output["marketplaces"] = serde_json::json!(
            plan.marketplaces
                .iter()
                .map(|c| c.json())
                .collect::<Vec<_>>()
        );
        output["plugins"] =
            serde_json::json!(plan.plugins.iter().map(|c| c.json()).collect::<Vec<_>>());
        output["mcp"] = serde_json::json!(plan.mcp.iter().map(|c| c.json()).collect::<Vec<_>>());
        output["skills"] =
            serde_json::json!(plan.skills.iter().map(|c| c.json()).collect::<Vec<_>>());
        println!("{output}");
    } else {
        for c in plan.marketplaces.iter().chain(&plan.plugins) {
            crate::list_command::print_row(vec![format!(
                "{} → {} {}: {}{}",
                c.logical,
                c.harness.binary(),
                c.id,
                c.outcome.as_str(),
                c.command
                    .as_ref()
                    .map(|cmd| format!("; {cmd}"))
                    .unwrap_or_default()
            )]);
        }
        for c in &plan.mcp {
            crate::list_command::print_row(vec![format!(
                "{} → {} {}: {} (add: {}, native off: {})",
                c.logical,
                c.harness.binary(),
                c.id,
                c.outcome.as_str(),
                c.add,
                c.disable
            )]);
        }
        for c in &plan.skills {
            crate::list_command::print_row(vec![format!(
                "{} → {} {}: {} (copy: {}, native off: {})",
                c.logical,
                c.harness.binary(),
                c.id,
                c.outcome.as_str(),
                c.copy,
                c.disable
            )]);
        }
        for d in &plan.diagnostics {
            crate::render(d, false);
        }
    }
    if plan.has_errors() { 3 } else { 0 }
}
