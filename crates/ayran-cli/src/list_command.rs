use ayran_core::config::{ConfigLayers, PluginBinding, SkillBinding};
use ayran_core::harness::Harness;
use ayran_core::mcp::{McpBinding, McpDefinition};
use clap::{Arg, ArgMatches, Command};

const HARNESSES: [Harness; 3] = [Harness::Claude, Harness::Codex, Harness::Copilot];

pub fn command() -> Command {
    let mut command = Command::new("list")
        .about("List Plugins, Skills, MCP servers, Profiles and Aliases from the current directory's Config layers")
        .arg(Arg::new("kind").value_parser(["plugins", "skills", "mcp", "profiles", "aliases", "sessions"]));
    for harness in HARNESSES {
        command = command.arg(
            Arg::new(harness.binary())
                .long(harness.binary())
                .help(format!("Show only the {} Binding column", harness.binary()))
                .action(clap::ArgAction::SetTrue),
        );
    }
    command
        .arg(
            Arg::new("all")
                .long("all")
                .help("List Sessions in every directory")
                .action(clap::ArgAction::SetTrue),
        )
        .group(
            clap::ArgGroup::new("harness-choice").args(
                HARNESSES
                    .map(Harness::binary)
                    .into_iter()
                    .chain(["harness"]),
            ),
        )
        .arg(
            Arg::new("harness")
                .long("harness")
                .help("Show only this Harness's Binding column")
                .value_parser(clap::builder::PossibleValuesParser::new(
                    HARNESSES.map(Harness::binary),
                )),
        )
        .arg(
            Arg::new("json")
                .long("json")
                .help("Print versioned JSON")
                .action(clap::ArgAction::SetTrue),
        )
}

fn binding_json(binding: Option<&PluginBinding>) -> serde_json::Value {
    match binding {
        Some(PluginBinding::Native(id)) => serde_json::json!({"kind":"native", "id":id}),
        Some(PluginBinding::Path(path)) => {
            serde_json::json!({"kind":"path", "path":path.to_string_lossy()})
        }
        Some(PluginBinding::Absent) => serde_json::json!({"kind":"absent"}),
        None => serde_json::Value::Null,
    }
}

fn binding_label(binding: Option<&PluginBinding>) -> &str {
    match binding {
        Some(PluginBinding::Native(id)) => id,
        Some(PluginBinding::Path(_)) => "path",
        Some(PluginBinding::Absent) => "—",
        None => "✗",
    }
}

fn skill_binding_json(binding: Option<&SkillBinding>) -> serde_json::Value {
    match binding {
        Some(SkillBinding::Native(id)) => serde_json::json!({"kind":"native", "id":id}),
        Some(SkillBinding::Path(path)) => {
            serde_json::json!({"kind":"path", "path":path.to_string_lossy()})
        }
        Some(SkillBinding::Absent) => serde_json::json!({"kind":"absent"}),
        None => serde_json::Value::Null,
    }
}

fn skill_binding_label(binding: Option<&SkillBinding>) -> &str {
    match binding {
        Some(SkillBinding::Native(id)) => id,
        Some(SkillBinding::Path(_)) => "path",
        Some(SkillBinding::Absent) => "—",
        None => "✗",
    }
}

// List only the transport: arguments, environment, headers and URLs can hold secrets.
fn mcp_binding_json(binding: Option<&McpBinding>) -> serde_json::Value {
    match binding {
        Some(McpBinding::Native(id)) => serde_json::json!({"kind":"native", "id":id}),
        Some(McpBinding::Connector(id)) => serde_json::json!({"kind":"connector", "id":id}),
        Some(McpBinding::Definition(definition)) => serde_json::json!({
            "kind":"definition", "transport": match definition {
                McpDefinition::Stdio { .. } => "stdio",
                McpDefinition::Http { .. } => "http",
            }
        }),
        Some(McpBinding::Absent) => serde_json::json!({"kind":"absent"}),
        None => serde_json::Value::Null,
    }
}

fn mcp_binding_label(binding: Option<&McpBinding>) -> &str {
    match binding {
        Some(McpBinding::Native(id)) => id,
        Some(McpBinding::Connector(_)) => "connector",
        Some(McpBinding::Definition(McpDefinition::Stdio { .. })) => "definition (stdio)",
        Some(McpBinding::Definition(McpDefinition::Http { .. })) => "definition (http)",
        Some(McpBinding::Absent) => "—",
        None => "✗",
    }
}

pub(crate) fn print_row(row: Vec<String>) {
    let row: Vec<_> = row
        .into_iter()
        .map(|cell| {
            cell.replace('\\', "\\\\")
                .replace('\t', "\\t")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
        })
        .collect();
    println!("{}", row.join("\t"));
}

/// The shared versioned list envelope, including empty arrays for unselected kinds.
pub(crate) fn json_envelope(
    diagnostics: &[ayran_core::diagnostic::Diagnostic],
) -> serde_json::Value {
    use ayran_core::diagnostic::Severity;
    serde_json::json!({
        "version":1, "plugins":[], "skills":[], "mcp":[], "profiles":[], "aliases":[], "sessions":[],
        "diagnostics":diagnostics,
        "summary":{
            "errors":diagnostics.iter().filter(|d| matches!(d.severity, Severity::Error)).count(),
            "warnings":diagnostics.iter().filter(|d| matches!(d.severity, Severity::Warning)).count(),
            "notes":diagnostics.iter().filter(|d| matches!(d.severity, Severity::Note)).count()
        }
    })
}

pub fn run(matches: &ArgMatches) -> i32 {
    let kind = matches.get_one::<String>("kind").map(String::as_str);
    if kind == Some("sessions") {
        return crate::session_list::run(matches);
    }
    if matches.get_flag("all") {
        crate::render(
            &ayran_core::diagnostic::Diagnostic::error(
                "usage",
                "--all requires list sessions",
                None,
            ),
            false,
        );
        return 2;
    }
    let layers = match ConfigLayers::load() {
        Ok(layers) => layers,
        Err(diagnostic) => {
            if matches.get_flag("json") {
                println!("{}", json_envelope(&[diagnostic]));
            } else {
                crate::render(&diagnostic, false);
            }
            return 3;
        }
    };
    let selected = matches
        .get_one::<String>("harness")
        .map(String::as_str)
        .or_else(|| {
            HARNESSES
                .into_iter()
                .find(|h| matches.get_flag(h.binary()))
                .map(Harness::binary)
        });
    let harnesses: Vec<_> = HARNESSES
        .into_iter()
        .filter(|harness| selected.is_none_or(|name| name == harness.binary()))
        .collect();
    if matches.get_flag("json") {
        let plugins: Vec<_> = layers.plugins.iter().filter(|_| kind.is_none_or(|kind| kind == "plugins")).map(|(name, plugin)| {
            let bindings: serde_json::Map<_, _> = harnesses.iter().map(|h| (h.binary().to_owned(), binding_json(plugin.value.binding(*h)))).collect();
            serde_json::json!({"name":name, "default":plugin.value.default, "layer":plugin.path.to_string_lossy(), "description":plugin.value.description, "bindings":bindings})
        }).collect();
        let skills: Vec<_> = layers.skills.iter().filter(|_| kind.is_none_or(|kind| kind == "skills")).map(|(name, skill)| {
            let bindings: serde_json::Map<_, _> = harnesses.iter().map(|h| (h.binary().to_owned(), skill_binding_json(skill.value.binding(*h)))).collect();
            serde_json::json!({"name":name, "default":skill.value.default, "layer":skill.path.to_string_lossy(), "description":skill.value.description, "bindings":bindings})
        }).collect();
        let mcp: Vec<_> = layers.mcp.iter().filter(|_| kind.is_none_or(|kind| kind == "mcp")).map(|(name, server)| {
            let bindings: serde_json::Map<_, _> = harnesses.iter().map(|h| (h.binary().to_owned(), mcp_binding_json(server.value.binding(*h)))).collect();
            serde_json::json!({"name":name, "default":server.value.default, "layer":server.path.to_string_lossy(), "description":server.value.description, "bindings":bindings})
        }).collect();
        let profiles: Vec<_> = layers.profiles.iter().filter(|_| kind.is_none_or(|kind| kind == "profiles")).map(|(name, profile)| {
            serde_json::json!({"name":name, "default":profile.value.default, "layer":profile.path.to_string_lossy(), "description":profile.value.description, "plugins":profile.value.plugins, "skills":profile.value.skills, "mcp":profile.value.mcp, "profiles":profile.value.profiles})
        }).collect();
        let aliases: Vec<_> = layers
            .aliases
            .iter()
            .filter(|(_, alias)| {
                kind.is_none_or(|k| k == "aliases")
                    && selected.is_none_or(|h| h == alias.harness.binary())
            })
            .map(|(name, alias)| {
                serde_json::json!({
                    "name":name, "harness":alias.harness.binary(), "description":alias.description
                })
            })
            .collect();
        let mut output = json_envelope(&[]);
        output["plugins"] = serde_json::json!(plugins);
        output["skills"] = serde_json::json!(skills);
        output["mcp"] = serde_json::json!(mcp);
        output["profiles"] = serde_json::json!(profiles);
        output["aliases"] = serde_json::json!(aliases);
        println!("{output}");
        return 0;
    }
    if kind.is_none_or(|kind| kind == "plugins") {
        let mut header = vec!["name", "default", "layer", "description"];
        header.extend(harnesses.iter().map(|h| h.binary()));
        println!("{}", header.join("\t"));
        for (name, plugin) in layers.plugins {
            let mut row = vec![
                name,
                if plugin.value.default { "*" } else { "" }.to_owned(),
                plugin.path.display().to_string(),
                plugin.value.description.clone().unwrap_or_default(),
            ];
            row.extend(
                harnesses
                    .iter()
                    .map(|h| binding_label(plugin.value.binding(*h)).to_owned()),
            );
            print_row(row);
        }
    }
    if kind == Some("skills") || (kind.is_none() && !layers.skills.is_empty()) {
        let mut header = vec!["name", "default", "layer", "description"];
        header.extend(harnesses.iter().map(|h| h.binary()));
        println!("{}", header.join("\t"));
        for (name, skill) in layers.skills {
            let mut row = vec![
                name,
                if skill.value.default { "*" } else { "" }.to_owned(),
                skill.path.display().to_string(),
                skill.value.description.clone().unwrap_or_default(),
            ];
            row.extend(
                harnesses
                    .iter()
                    .map(|h| skill_binding_label(skill.value.binding(*h)).to_owned()),
            );
            print_row(row);
        }
    }
    if kind == Some("mcp") || (kind.is_none() && !layers.mcp.is_empty()) {
        let mut header = vec!["name", "default", "layer", "description"];
        header.extend(harnesses.iter().map(|h| h.binary()));
        println!("{}", header.join("\t"));
        for (name, server) in layers.mcp {
            let mut row = vec![
                name,
                if server.value.default { "*" } else { "" }.to_owned(),
                server.path.display().to_string(),
                server.value.description.clone().unwrap_or_default(),
            ];
            row.extend(
                harnesses
                    .iter()
                    .map(|h| mcp_binding_label(server.value.binding(*h)).to_owned()),
            );
            print_row(row);
        }
    }
    if kind == Some("profiles") || (kind.is_none() && !layers.profiles.is_empty()) {
        println!("name\tdefault\tlayer\tdescription\tmembers");
        for (name, profile) in layers.profiles {
            let mut members = Vec::new();
            if !profile.value.plugins.is_empty() {
                members.push(format!("plugins: {}", profile.value.plugins.join(", ")));
            }
            if !profile.value.skills.is_empty() {
                members.push(format!("skills: {}", profile.value.skills.join(", ")));
            }
            if !profile.value.mcp.is_empty() {
                members.push(format!("mcp: {}", profile.value.mcp.join(", ")));
            }
            if !profile.value.profiles.is_empty() {
                members.push(format!("profiles: {}", profile.value.profiles.join(", ")));
            }
            print_row(vec![
                name,
                if profile.value.default { "*" } else { "" }.to_owned(),
                profile.path.display().to_string(),
                profile.value.description.unwrap_or_default(),
                members.join(" · "),
            ]);
        }
    }
    if kind == Some("aliases") || (kind.is_none() && !layers.aliases.is_empty()) {
        println!("name\tharness\tdescription");
        for (name, alias) in layers.aliases {
            if selected.is_none_or(|h| h == alias.harness.binary()) {
                print_row(vec![
                    name,
                    alias.harness.binary().to_owned(),
                    alias.description.unwrap_or_default(),
                ]);
            }
        }
    }
    0
}
