use ayran_core::{config::ConfigLayers, harness::Harness};
use clap::{Arg, ArgAction, ArgMatches, Command};

pub fn command() -> Command {
    let mut command = Command::new("install")
        .about("Add declared Marketplaces and install declared Plugins")
        .arg(Arg::new("plugins").num_args(0..));
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
            let mut plan = crate::install_plan::build(&layers, &names, selected);
            if !matches.get_flag("dry-run") && !plan.has_errors() {
                crate::install_write::execute(&layers, &mut plan);
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
        for d in &plan.diagnostics {
            crate::render(d, false);
        }
    }
    if plan.has_errors() { 3 } else { 0 }
}
