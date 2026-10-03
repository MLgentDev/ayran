use crate::config_command;
use clap::{Arg, ArgAction, Command, builder::OsStringValueParser};

pub fn command() -> Command {
    let command = Command::new("ayran")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Launch a coding Harness with selected Capabilities")
        .subcommand(config_command::command())
        .subcommand(crate::doctor_command::command())
        .subcommand(crate::list_command::command())
        .subcommand(crate::native_command::command())
        .subcommand(crate::trust::command())
        .subcommand(crate::install_command::command())
        .subcommand(crate::update_command::command())
        .subcommand(
            Command::new("__complete").hide(true).arg(
                Arg::new("words")
                    .num_args(0..)
                    .last(true)
                    .value_parser(OsStringValueParser::new()),
            ),
        )
        .subcommand(
            Command::new("activate")
                .about("Print shell dispatchers and completion hooks")
                .arg(
                    Arg::new("shell")
                        .required(true)
                        .value_parser(["zsh", "bash", "pwsh"]),
                ),
        )
        .arg(
            Arg::new("claude")
                .help("Launch Claude Code")
                .long("claude")
                .action(ArgAction::Count),
        )
        .arg(
            Arg::new("codex")
                .help("Launch Codex")
                .long("codex")
                .action(ArgAction::Count),
        )
        .arg(
            Arg::new("copilot")
                .help("Launch GitHub Copilot CLI")
                .long("copilot")
                .action(ArgAction::Count),
        )
        .arg(
            Arg::new("harness")
                .help("Choose a Harness")
                .long("harness")
                .action(ArgAction::Append)
                .value_parser(["claude", "codex", "copilot"]),
        )
        .arg(
            Arg::new("mcp")
                .help("Select MCP servers by logical name")
                .long("mcp")
                .action(ArgAction::Append)
                .value_delimiter(','),
        )
        .arg(
            Arg::new("skill")
                .help("Select Skills by logical name")
                .long("skill")
                .action(ArgAction::Append)
                .value_delimiter(','),
        )
        .arg(
            Arg::new("plugin")
                .help("Select Plugins by logical name")
                .long("plugin")
                .action(ArgAction::Append)
                .value_delimiter(','),
        )
        .arg(
            Arg::new("profile")
                .help("Select Profiles by logical name")
                .long("profile")
                .short('p')
                .action(ArgAction::Append)
                .value_delimiter(','),
        )
        .arg(
            Arg::new("no-profile")
                .help("Disable Profiles by logical name")
                .long("no-profile")
                .action(ArgAction::Append)
                .value_delimiter(','),
        )
        .arg(
            Arg::new("no-mcp")
                .help("Disable MCP servers by logical name")
                .long("no-mcp")
                .action(ArgAction::Append)
                .value_delimiter(','),
        )
        .arg(
            Arg::new("no-skill")
                .help("Disable Skills by logical name")
                .long("no-skill")
                .action(ArgAction::Append)
                .value_delimiter(','),
        )
        .arg(
            Arg::new("no-plugin")
                .help("Disable Plugins by logical name")
                .long("no-plugin")
                .action(ArgAction::Append)
                .value_delimiter(','),
        )
        .arg(
            Arg::new("no-harness-args")
                .help("Drop configured Harness and Alias args")
                .long("no-harness-args")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("no-defaults")
                .help("Drop every Default")
                .long("no-defaults")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("model")
                .help("Choose a model")
                .short('m')
                .long("model")
                .action(ArgAction::Append),
        )
        .arg(
            Arg::new("alias")
                .long("alias")
                .hide(true)
                .action(ArgAction::Append),
        )
        .arg(
            Arg::new("effort")
                .help("Choose the reasoning Effort")
                .short('e')
                .long("effort")
                .action(ArgAction::Append)
                .value_parser(["low", "medium", "high", "xhigh", "max"]),
        )
        .arg(
            Arg::new("dry-run")
                .help("Show the launch command")
                .long("dry-run")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("json")
                .help("Print the dry-run command as JSON")
                .long("json")
                .requires("dry-run")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("quiet")
                .help("Suppress informational output")
                .short('q')
                .long("quiet")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("passthrough")
                .num_args(0..)
                .require_equals(false)
                .last(true)
                .allow_hyphen_values(true)
                .value_parser(OsStringValueParser::new()),
        );
    let resume = Command::new("resume")
        .about("Resume a recorded Session")
        .arg(
            Arg::new("fork")
                .long("fork")
                .help("Continue the conversation in a new Session")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("native")
                .long("native")
                .value_name("UUID")
                .help("Set a Codex Session's native thread ID"),
        )
        .arg(Arg::new("id").conflicts_with("last"))
        .arg(
            Arg::new("last")
                .long("last")
                .help("Resume the most recently used Session")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("all")
                .long("all")
                .help("Find the last Session across every directory")
                .requires("last")
                .conflicts_with("id")
                .action(ArgAction::SetTrue),
        )
        .group(
            clap::ArgGroup::new("session-choice")
                .args(["id", "last"])
                .required(true),
        )
        .args(command.get_arguments().cloned());
    command.subcommand(resume)
}
