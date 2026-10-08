//! Complete words using the same command tree as the CLI.

use ayran_core::config::{ConfigLayers, PluginBinding, SkillBinding};
use ayran_core::harness::Harness;
use ayran_core::mcp::McpBinding;
use std::ffi::OsString;
use std::io::{self, Write};

/// Wire protocol: `ayran __complete -- <command> <preceding words> <cursor word>`.
/// The final word may be empty. Bash receives one plain candidate per line.
/// With `--descriptions` before `--`, each line is a candidate and description
/// separated by a tab. Candidate backslashes and tabs are escaped as `\\` and
/// `\t`. Descriptions contain no protocol delimiters.
struct Candidate {
    value: String,
    description: String,
}

impl Candidate {
    fn new(value: impl Into<String>, description: impl ToString) -> Self {
        Self {
            value: value.into(),
            description: description.to_string().replace(['\t', '\n', '\r'], " "),
        }
    }
}
pub fn run(args: &[OsString]) {
    let (descriptions, args) = if args.first().is_some_and(|arg| arg == "--descriptions") {
        (true, &args[1..])
    } else {
        (false, args)
    };
    if args.first().is_none_or(|arg| arg != "--") {
        return;
    }
    let Some(words) = args[1..]
        .iter()
        .map(|word| word.to_str())
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };
    let Ok(layers) = ConfigLayers::load_for_completion() else {
        return;
    };
    let output = complete_described(
        crate::cli::with_presets(&layers[0].presets),
        &words,
        &layers,
    )
    .into_iter()
    .map(|candidate| {
        if descriptions {
            let value = candidate.value.replace('\\', "\\\\").replace('\t', "\\t");
            format!("{value}\t{}", candidate.description)
        } else {
            candidate.value
        }
    })
    .collect::<Vec<_>>()
    .join("\n");
    if !output.is_empty() {
        let _ = writeln!(io::stdout().lock(), "{output}");
    }
}

#[cfg(test)]
pub fn candidates(words: &[&str]) -> Vec<String> {
    complete(crate::cli::command(), words)
}

#[cfg(test)]
fn complete(grammar: clap::Command, words: &[&str]) -> Vec<String> {
    complete_described(grammar, words, &[])
        .into_iter()
        .map(|candidate| candidate.value)
        .collect()
}

fn complete_described(
    mut grammar: clap::Command,
    words: &[&str],
    layers: &[ConfigLayers],
) -> Vec<Candidate> {
    let Some((prefix, preceding)) = words.split_last() else {
        return Vec::new();
    };
    if preceding.first() != Some(&"ayran") {
        return Vec::new();
    }
    grammar.build();
    let mut command = &grammar;
    let mut used = Vec::new();
    let mut pending: Option<&clap::Arg> = None;
    let mut positional_index = 1;
    let mut split_equals = false;
    let mut alias_selected = false;
    let mut context = CompletionContext::new(layers);
    for word in &preceding[1..] {
        if let Some(arg) = pending.take() {
            // Bash splits long options at '=' using COMP_WORDBREAKS.
            if *word == "=" && !split_equals {
                pending = Some(arg);
                split_equals = true;
                continue;
            }
            let value = if split_equals {
                word
            } else {
                word.strip_prefix('=').unwrap_or(word)
            };
            if !accepts_value(arg, value) {
                return Vec::new();
            }
            context.observe(arg, value);
            alias_selected |= arg.get_id() == "alias";
        } else if let Some(subcommand) = command.find_subcommand(word) {
            if alias_selected {
                return Vec::new();
            }
            if matches!(
                command.get_name(),
                "plugin" | "skill" | "mcp" | "marketplace"
            ) && matches!(*word, "enable" | "disable" | "update")
            {
                context.native_update = (*word == "update").then_some(command.get_name());
                context.native_skill = command.get_name() == "skill";
                context.native_mcp = command.get_name() == "mcp";
                context.native_state = Some(if *word == "disable" {
                    crate::native_plugins::NativeState::On
                } else {
                    crate::native_plugins::NativeState::Off
                });
            }
            command = subcommand;
            positional_index = 1;
            used.clear();
        } else if word.starts_with('-') {
            let (flag, inline) = word
                .split_once('=')
                .map_or((*word, None), |(flag, value)| (flag, Some(value)));
            let Some(arg) = flag_arg(command, flag) else {
                return Vec::new();
            };
            if !available(command, arg, &used) {
                return Vec::new();
            }
            used.push(arg);
            if let Some(value) = inline {
                if !arg.get_action().takes_values() || !accepts_value(arg, value) {
                    return Vec::new();
                }
                context.observe(arg, value);
                alias_selected |= arg.get_id() == "alias";
            } else if arg.get_action().takes_values() {
                pending = Some(arg);
                split_equals = false;
            } else {
                context.observe(arg, "");
            }
        } else if let Some(arg) = positional(command, positional_index) {
            if !accepts_value(arg, word) {
                return Vec::new();
            }
            used.push(arg);
            positional_index += 1;
        } else {
            return Vec::new();
        }
    }
    if let Some(arg) = pending {
        return value_candidates(
            arg,
            if split_equals {
                prefix
            } else {
                prefix.strip_prefix('=').unwrap_or(prefix)
            },
            &context,
        );
    }
    if prefix.starts_with("--")
        && let Some((flag, value)) = prefix.split_once('=')
    {
        let Some(arg) = flag_arg(command, flag)
            .filter(|arg| arg.get_action().takes_values() && available(command, arg, &used))
        else {
            return Vec::new();
        };
        return value_candidates(arg, value, &context)
            .into_iter()
            .map(|mut candidate| {
                candidate.value = format!("{flag}={}", candidate.value);
                candidate
            })
            .collect();
    }
    let mut candidates = positional(command, positional_index)
        .filter(|arg| available(command, arg, &used))
        .map(|arg| value_candidates(arg, prefix, &context))
        .unwrap_or_default();
    candidates.extend(
        command
            .get_subcommands()
            .filter(|subcommand| !alias_selected && !subcommand.is_hide_set())
            .map(|subcommand| {
                Candidate::new(
                    subcommand.get_name(),
                    subcommand
                        .get_about()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| subcommand.get_name().to_owned()),
                )
            }),
    );
    for arg in command
        .get_arguments()
        .filter(|arg| !arg.is_hide_set() && available(command, arg, &used))
        .filter(|arg| !alias_selected || !is_harness_arg(arg) && arg.get_id() != "alias")
    {
        if let Some(name) = arg.get_id().as_str().strip_prefix("preset:")
            && !context
                .preset_candidates("")
                .iter()
                .any(|candidate| candidate.value == name)
        {
            continue;
        }
        if arg.get_id() == "preset" && context.preset.is_some() {
            continue;
        }
        if let Some(long) = arg.get_long() {
            candidates.push(Candidate::new(format!("--{long}"), arg_description(arg)));
        }
        if let Some(short) = arg.get_short() {
            candidates.push(Candidate::new(format!("-{short}"), arg_description(arg)));
        }
    }
    candidates.retain(|candidate| {
        candidate.value.starts_with(prefix)
            || (command.get_name() == "resume"
                && !candidate.value.starts_with('-')
                && ayran_core::session::matches_prefix(&candidate.value, prefix))
    });
    candidates.sort_by(|a, b| a.value.cmp(&b.value));
    candidates
}

#[derive(Clone, Copy)]
enum MemberKind {
    Skill,
    Mcp,
}

/// Selections and Defaults used to complete values for the current command.
struct CompletionContext<'a> {
    layers: &'a [ConfigLayers],
    explicit: Option<Harness>,
    alias: Option<&'a ayran_core::config::Alias>,
    preset: Option<&'a ayran_core::config::Preset>,
    invalid: bool,
    selected_mcp: Vec<String>,
    disabled_mcp: Vec<String>,
    selected_skills: Vec<String>,
    disabled_skills: Vec<String>,
    selected_plugins: Vec<String>,
    disabled_plugins: Vec<String>,
    selected_profiles: Vec<String>,
    disabled_profiles: Vec<String>,
    no_defaults: bool,
    native_state: Option<crate::native_plugins::NativeState>,
    native_update: Option<&'a str>,
    native_id: bool,
    native_skill: bool,
    native_mcp: bool,
}

impl<'a> CompletionContext<'a> {
    fn new(layers: &'a [ConfigLayers]) -> Self {
        Self {
            layers,
            explicit: None,
            alias: None,
            preset: None,
            invalid: false,
            selected_mcp: Vec::new(),
            disabled_mcp: Vec::new(),
            selected_skills: Vec::new(),
            disabled_skills: Vec::new(),
            selected_plugins: Vec::new(),
            disabled_plugins: Vec::new(),
            selected_profiles: Vec::new(),
            disabled_profiles: Vec::new(),
            no_defaults: false,
            native_state: None,
            native_update: None,
            native_id: false,
            native_skill: false,
            native_mcp: false,
        }
    }

    fn observe(&mut self, arg: &clap::Arg, value: &str) {
        let name = match arg.get_id().as_str() {
            "mcp" | "install-mcp" => {
                self.selected_mcp
                    .extend(value.split(',').map(str::to_owned));
                return;
            }
            "no-mcp" => {
                self.disabled_mcp
                    .extend(value.split(',').map(str::to_owned));
                return;
            }
            "skill" | "install-skill" => {
                self.selected_skills
                    .extend(value.split(',').map(str::to_owned));
                return;
            }
            "no-skill" => {
                self.disabled_skills
                    .extend(value.split(',').map(str::to_owned));
                return;
            }
            "profile" => {
                self.selected_profiles
                    .extend(value.split(',').map(str::to_owned));
                return;
            }
            "no-profile" => {
                self.disabled_profiles
                    .extend(value.split(',').map(str::to_owned));
                return;
            }
            "no-plugin" => {
                self.disabled_plugins
                    .extend(value.split(',').map(str::to_owned));
                return;
            }
            "no-defaults" => {
                self.no_defaults = true;
                return;
            }
            "native-id" => {
                self.native_id = true;
                return;
            }
            "plugin" => {
                self.selected_plugins
                    .extend(value.split(',').map(str::to_owned));
                return;
            }
            "preset" => {
                self.preset = self
                    .layers
                    .first()
                    .and_then(|layer| layer.presets.get(value))
                    .map(|p| &p.value);
                self.invalid |= self.preset.is_none();
                return;
            }
            id if id.starts_with("preset:") => {
                self.preset = self
                    .layers
                    .first()
                    .and_then(|layer| layer.presets.get(&id[7..]))
                    .map(|p| &p.value);
                return;
            }
            "harness" => value,
            "claude" | "codex" | "copilot" => arg.get_id().as_str(),
            "alias" => {
                self.alias = self
                    .layers
                    .first()
                    .and_then(|layer| layer.aliases.get(value));
                self.invalid |= self.alias.is_none();
                return;
            }
            _ => return,
        };
        self.explicit = match name {
            "claude" => Some(Harness::Claude),
            "codex" => Some(Harness::Codex),
            "copilot" => Some(Harness::Copilot),
            _ => None,
        };
    }

    fn configured_harness(&self) -> Option<Harness> {
        self.preset.map(|preset| preset.harness).or_else(|| {
            self.alias
                .and_then(|alias| alias.configured_harness(self.layers.first()?))
        })
    }

    fn harness_conflict(&self) -> bool {
        self.explicit
            .zip(self.configured_harness())
            .is_some_and(|(a, b)| a != b)
    }

    fn preset_candidates(&self, prefix: &str) -> Vec<Candidate> {
        if self.invalid || self.preset.is_some() {
            return Vec::new();
        }
        self.layers
            .first()
            .into_iter()
            .flat_map(|layer| &layer.presets)
            .filter(|(name, p)| {
                name.starts_with(prefix) && self.explicit.is_none_or(|h| p.value.harness == h)
            })
            .map(|(name, p)| {
                Candidate::new(name, p.value.description.as_deref().unwrap_or("Preset"))
            })
            .collect()
    }

    fn harness(&self) -> Option<Harness> {
        if self.invalid || self.harness_conflict() {
            return None;
        }
        self.explicit
            .or_else(|| self.configured_harness())
            .or_else(|| {
                self.layers
                    .iter()
                    .rev()
                    .find_map(|layer| layer.default_harness.as_ref().map(|source| source.value))
            })
    }

    fn plugin_candidates(&self, prefix: &str, completed: &str, disable: bool) -> Vec<Candidate> {
        if self.invalid || self.harness_conflict() {
            return Vec::new();
        }
        let harness = self.harness();
        let mut plugins = std::collections::BTreeMap::new();
        let mut reset = None;
        for (index, layer) in self.layers.iter().enumerate() {
            if layer.disable_defaults.is_some() {
                reset = Some(index);
            }
            for (name, plugin) in &layer.plugins {
                plugins.insert(name, (index, plugin));
            }
        }
        plugins
            .into_iter()
            .filter(|(name, (index, plugin))| {
                let alias_selected = self.alias.is_some_and(|alias| alias.plugins.contains(name));
                let eligible = if disable {
                    let default = plugin.value.default
                        && !self.no_defaults
                        && self.alias.is_none_or(|alias| {
                            alias.defaults && !alias.disabled_plugins.contains(name)
                        })
                        && reset.is_none_or(|reset| *index >= reset)
                        && !self
                            .layers
                            .iter()
                            .any(|layer| layer.disabled_plugins.contains_key(*name));
                    (alias_selected || default) && !self.disabled_plugins.contains(name)
                } else {
                    !alias_selected && !self.selected_plugins.contains(name)
                };
                eligible
                    && !completed
                        .split(',')
                        .any(|selected| selected == name.as_str())
                    && name.starts_with(prefix)
                    && !name.contains(['\n', '\r'])
                    && harness.is_none_or(|h| match plugin.value.binding(h) {
                        Some(PluginBinding::Native(_)) => true,
                        Some(PluginBinding::Path(_)) => h != Harness::Codex,
                        Some(PluginBinding::Absent) | None => false,
                    })
            })
            .map(|(name, (_, plugin))| {
                Candidate::new(
                    name,
                    plugin.value.description.as_deref().unwrap_or("Plugin"),
                )
            })
            .collect()
    }

    fn member_candidates(
        &self,
        prefix: &str,
        completed: &str,
        disable: bool,
        kind: MemberKind,
    ) -> Vec<Candidate> {
        if self.invalid || self.harness_conflict() {
            return Vec::new();
        }
        let harness = self.harness();
        let mut definitions = std::collections::BTreeMap::new();
        let mut reset = None;
        for (index, layer) in self.layers.iter().enumerate() {
            if layer.disable_defaults.is_some() {
                reset = Some(index);
            }
            match kind {
                MemberKind::Skill => {
                    for (name, skill) in &layer.skills {
                        let usable = harness.is_none_or(|h| match skill.value.binding(h) {
                            Some(SkillBinding::Native(_) | SkillBinding::Git(_)) => true,
                            Some(SkillBinding::Path(_) | SkillBinding::Builtin(_)) => {
                                h != Harness::Codex
                            }
                            Some(SkillBinding::Absent) | None => false,
                        });
                        definitions.insert(
                            name,
                            (
                                index,
                                skill.value.default,
                                skill.value.description.as_deref(),
                                usable,
                            ),
                        );
                    }
                }
                MemberKind::Mcp => {
                    for (name, server) in &layer.mcp {
                        let usable = harness.is_none_or(|h| match server.value.binding(h) {
                            Some(McpBinding::Native(_) | McpBinding::Definition(_)) => true,
                            Some(McpBinding::Connector(_)) => {
                                h == Harness::Claude && server.value.claude.is_some()
                            }
                            Some(McpBinding::Absent) | None => false,
                        });
                        definitions.insert(
                            name,
                            (
                                index,
                                server.value.default,
                                server.value.description.as_deref(),
                                usable,
                            ),
                        );
                    }
                }
            }
        }
        let (selected, disabled, alias_selected, alias_disabled) = match kind {
            MemberKind::Skill => (
                &self.selected_skills,
                &self.disabled_skills,
                self.alias.map(|a| &a.skills),
                self.alias.map(|a| &a.disabled_skills),
            ),
            MemberKind::Mcp => (
                &self.selected_mcp,
                &self.disabled_mcp,
                self.alias.map(|a| &a.mcp),
                self.alias.map(|a| &a.disabled_mcp),
            ),
        };
        let config_disabled = |name: &String| {
            self.layers.iter().any(|layer| match kind {
                MemberKind::Skill => layer.disabled_skills.contains_key(name),
                MemberKind::Mcp => layer.disabled_mcp.contains_key(name),
            })
        };
        let profile_members: std::collections::BTreeSet<_> = self
            .active_profiles("")
            .values()
            .flat_map(|profile| match kind {
                MemberKind::Skill => profile.value.skills.iter(),
                MemberKind::Mcp => profile.value.mcp.iter(),
            })
            .collect();
        definitions
            .into_iter()
            .filter(|(name, (index, default, _, usable))| {
                let alias_selected = alias_selected.is_some_and(|names| names.contains(name));
                let eligible = if disable {
                    let default = *default
                        && !self.no_defaults
                        && self.alias.is_none_or(|alias| {
                            alias.defaults
                                && alias_disabled.is_none_or(|names| !names.contains(name))
                        })
                        && reset.is_none_or(|reset| *index >= reset)
                        && !config_disabled(name);
                    let explicit = alias_selected || selected.contains(name);
                    let member = profile_members.contains(name)
                        && !config_disabled(name)
                        && alias_disabled.is_none_or(|names| !names.contains(name));
                    (explicit || default || member) && !disabled.contains(name)
                } else {
                    !alias_selected && !selected.contains(name)
                };
                eligible
                    && !completed
                        .split(',')
                        .any(|selected| selected == name.as_str())
                    && name.starts_with(prefix)
                    && !name.contains(['\n', '\r'])
                    && *usable
            })
            .map(|(name, (_, _, description, _))| {
                Candidate::new(
                    name,
                    description.unwrap_or(match kind {
                        MemberKind::Skill => "Skill",
                        MemberKind::Mcp => "MCP server",
                    }),
                )
            })
            .collect()
    }

    fn active_profiles(
        &self,
        completed: &str,
    ) -> std::collections::BTreeMap<
        &String,
        &ayran_core::config::Sourced<ayran_core::config::Profile>,
    > {
        let mut profiles = std::collections::BTreeMap::new();
        let mut reset = None;
        for (index, layer) in self.layers.iter().enumerate() {
            if layer.disable_defaults.is_some() {
                reset = Some(index);
            }
            for (name, profile) in &layer.profiles {
                profiles.insert(name, (index, profile));
            }
        }
        let mut selected: std::collections::BTreeSet<_> = self.selected_profiles.iter().collect();
        if let Some(alias) = self.alias {
            selected.extend(&alias.profiles);
        }
        let mut expanded = std::collections::BTreeSet::new();
        let mut pending: Vec<_> = selected.iter().copied().collect();
        if !self.no_defaults && self.alias.is_none_or(|alias| alias.defaults) {
            pending.extend(profiles.iter().filter_map(|(name, (index, profile))| {
                (profile.value.default && reset.is_none_or(|reset| *index >= reset))
                    .then_some(*name)
            }));
        }
        while let Some(name) = pending.pop() {
            let Some((_, profile)) = profiles.get(name) else {
                continue;
            };
            let disabled = self.disabled_profiles.contains(name)
                || completed.split(',').any(|disabled| disabled == name)
                || !selected.contains(name)
                    && (self
                        .layers
                        .iter()
                        .any(|layer| layer.disabled_profiles.contains_key(name))
                        || self
                            .alias
                            .is_some_and(|alias| alias.disabled_profiles.contains(name)));
            if disabled || !expanded.insert(name) {
                continue;
            }
            pending.extend(&profile.value.profiles);
        }
        profiles
            .into_iter()
            .filter(|(name, _)| expanded.contains(name))
            .map(|(name, (_, profile))| (name, profile))
            .collect()
    }

    fn profile_candidates(&self, prefix: &str, completed: &str, disable: bool) -> Vec<Candidate> {
        if self.invalid || self.harness_conflict() {
            return Vec::new();
        }
        let profiles: std::collections::BTreeMap<_, _> = if disable {
            self.active_profiles(completed)
        } else {
            self.layers
                .iter()
                .flat_map(|layer| layer.profiles.iter())
                .collect()
        };
        profiles
            .into_iter()
            .filter(|(name, _)| {
                (disable
                    || !self.selected_profiles.contains(name)
                        && self
                            .alias
                            .is_none_or(|alias| !alias.profiles.contains(name)))
                    && !completed
                        .split(',')
                        .any(|selected| selected == name.as_str())
                    && name.starts_with(prefix)
                    && !name.contains(['\n', '\r'])
            })
            .map(|(name, profile)| {
                Candidate::new(
                    name,
                    profile.value.description.as_deref().unwrap_or("Profile"),
                )
            })
            .collect()
    }

    fn model_candidates(&self, prefix: &str) -> Vec<Candidate> {
        let Some(harness) = self.harness() else {
            return Vec::new();
        };
        let mut models = std::collections::BTreeMap::new();
        for layer in self.layers {
            if let Some(model) = &layer.settings(harness).model {
                models.insert(model.value.clone(), model.path.display().to_string());
            }
            for (name, preset) in &layer.presets {
                if preset.value.harness == harness
                    && let Some(model) = &preset.value.model
                {
                    models
                        .entry(model.clone())
                        .or_insert_with(|| format!("Preset {name}"));
                }
            }
            for (name, alias) in &layer.aliases {
                if alias.effective_harness(layer) == Some(harness)
                    && let Some(model) = &alias.model
                {
                    models
                        .entry(model.clone())
                        .or_insert_with(|| format!("Alias {name}"));
                }
            }
        }
        if harness == Harness::Codex {
            let home_mode = self
                .layers
                .first()
                .map(|layer| layer.codex.home_mode)
                .unwrap_or_default();
            for (slug, description) in codex_models(home_mode) {
                if let Some(description) = description {
                    models.insert(slug, description);
                } else {
                    models
                        .entry(slug)
                        .or_insert_with(|| "Codex model cache".to_owned());
                }
            }
        }
        models
            .into_iter()
            .filter(|(model, _)| model.starts_with(prefix) && !model.contains(['\n', '\r']))
            .map(|(model, description)| Candidate::new(model, description))
            .collect()
    }
}

fn codex_models(home_mode: ayran_core::launch::HomeMode) -> Vec<(String, Option<String>)> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from);
    let directory = crate::harness_home::directory(Harness::Codex, home_mode, home.as_deref());
    let Some(directory) = directory else {
        return Vec::new();
    };
    let Ok(contents) = std::fs::read(directory.join("models_cache.json")) else {
        return Vec::new();
    };
    let Ok(cache) = serde_json::from_slice::<serde_json::Value>(&contents) else {
        return Vec::new();
    };
    let Some(models) = cache.get("models").and_then(|models| models.as_array()) else {
        return Vec::new();
    };
    if models.iter().any(|model| {
        model.get("slug").and_then(|slug| slug.as_str()).is_none()
            || model
                .get("visibility")
                .and_then(|visibility| visibility.as_str())
                .is_none()
            || model
                .get("description")
                .is_some_and(|description| !description.is_null() && !description.is_string())
    }) {
        return Vec::new();
    }
    models
        .iter()
        .filter_map(|model| {
            if model.get("visibility")?.as_str()? != "list" {
                return None;
            }
            Some((
                model.get("slug")?.as_str()?.to_owned(),
                model
                    .get("description")
                    .and_then(|description| description.as_str())
                    .map(str::to_owned),
            ))
        })
        .collect()
}
fn flag_arg<'a>(command: &'a clap::Command, word: &str) -> Option<&'a clap::Arg> {
    command.get_arguments().find(|arg| {
        arg.get_long()
            .is_some_and(|long| word == format!("--{long}"))
            || arg
                .get_all_aliases()
                .is_some_and(|aliases| aliases.iter().any(|alias| word == format!("--{alias}")))
            || arg
                .get_short()
                .is_some_and(|short| word == format!("-{short}"))
            || arg
                .get_all_short_aliases()
                .is_some_and(|aliases| aliases.iter().any(|alias| word == format!("-{alias}")))
    })
}

fn is_harness_arg(arg: &clap::Arg) -> bool {
    matches!(
        arg.get_id().as_str(),
        "harness" | "claude" | "codex" | "copilot"
    )
}

fn available(command: &clap::Command, arg: &clap::Arg, used: &[&clap::Arg]) -> bool {
    used.iter().all(|previous| {
        let same = previous.get_id() == arg.get_id();
        // Launch scalars use Append/Count to diagnose duplicates in main.
        // Comma-delimited Append arguments represent repeatable Capability lists.
        let repeatable = matches!(arg.get_action(), clap::ArgAction::Append)
            && (arg.get_value_delimiter().is_some() || arg.get_id() == "native-names");
        !(same && !repeatable
            || is_harness_arg(previous) && is_harness_arg(arg)
            || command
                .get_arg_conflicts_with(previous)
                .iter()
                .any(|conflict| conflict.get_id() == arg.get_id())
            || command
                .get_arg_conflicts_with(arg)
                .iter()
                .any(|conflict| conflict.get_id() == previous.get_id())
            || command.get_groups().any(|group| {
                let mut group = group.clone();
                !group.is_multiple()
                    && group.get_args().any(|id| id == previous.get_id())
                    && group.get_args().any(|id| id == arg.get_id())
            }))
    })
}

fn positional(command: &clap::Command, index: usize) -> Option<&clap::Arg> {
    command.get_positionals().find(|arg| {
        !arg.is_last_set()
            && (arg.get_index() == Some(index)
                || arg.get_id() == "native-names" && index >= arg.get_index().unwrap_or(1))
    })
}

fn accepts_value(arg: &clap::Arg, word: &str) -> bool {
    let accepts = |word: &str| {
        !word.starts_with('-')
            && arg
                .get_value_parser()
                .possible_values()
                .is_none_or(|mut values| values.any(|value| value.get_name() == word))
    };
    if let Some(delimiter) = arg.get_value_delimiter() {
        word.split(delimiter).all(accepts)
    } else {
        accepts(word)
    }
}

fn arg_description(arg: &clap::Arg) -> String {
    arg.get_help()
        .map(ToString::to_string)
        .unwrap_or_else(|| arg.get_id().to_string())
}

fn value_candidates(
    arg: &clap::Arg,
    prefix: &str,
    context: &CompletionContext<'_>,
) -> Vec<Candidate> {
    if arg.get_id() == "preset" {
        return context.preset_candidates(prefix);
    }
    if arg.get_id() == "install-skill" {
        let mut skills = std::collections::BTreeMap::new();
        for layer in context.layers {
            skills.extend(layer.skills.iter());
        }
        return skills
            .into_iter()
            .filter(|(name, skill)| {
                name.starts_with(prefix)
                    && !context.selected_skills.contains(name)
                    && ayran_core::doctor::HARNESSES
                        .into_iter()
                        .filter(|h| context.explicit.is_none_or(|s| s == *h))
                        .any(|h| {
                            matches!(
                                skill.value.binding(h),
                                Some(
                                    ayran_core::config::SkillBinding::Native(_)
                                        | ayran_core::config::SkillBinding::Git(_)
                                        | ayran_core::config::SkillBinding::Path(_)
                                        | ayran_core::config::SkillBinding::Builtin(_)
                                )
                            )
                        })
            })
            .map(|(name, s)| {
                Candidate::new(
                    name,
                    s.value.description.as_deref().unwrap_or("Skill snapshot"),
                )
            })
            .collect();
    }
    if arg.get_id() == "install-mcp" {
        let mut servers = std::collections::BTreeMap::new();
        for layer in context.layers {
            servers.extend(layer.mcp.iter());
        }
        return servers
            .into_iter()
            .filter(|(name, server)| {
                name.starts_with(prefix)
                    && !context.selected_mcp.contains(name)
                    && ayran_core::doctor::HARNESSES
                        .into_iter()
                        .filter(|h| context.explicit.is_none_or(|s| s == *h))
                        .any(|h| {
                            matches!(
                                server.value.binding(h),
                                Some(McpBinding::Native(_) | McpBinding::Definition(_))
                            )
                        })
            })
            .map(|(name, s)| {
                Candidate::new(name, s.value.description.as_deref().unwrap_or("MCP server"))
            })
            .collect();
    }
    if arg.get_id() == "native-names" {
        let Some(state) = context.native_state else {
            return Vec::new();
        };
        let Ok(layers) = ConfigLayers::load() else {
            return Vec::new();
        };
        let harnesses = crate::native_plugins::installed()
            .into_iter()
            .filter(|h| context.explicit.is_none_or(|selected| selected == *h))
            .collect::<Vec<_>>();
        if let Some(kind) = context.native_update {
            return crate::native_update::completion(
                kind,
                &layers,
                &harnesses,
                context.native_id,
                context.explicit.is_some(),
            )
            .into_iter()
            .filter(|(n, _)| n.starts_with(prefix))
            .map(|(n, d)| Candidate::new(n, d))
            .collect();
        }
        if context.native_skill {
            return crate::native_skills::completion(
                &layers,
                &harnesses,
                state,
                context.native_id,
                context.explicit.is_some(),
            )
            .into_iter()
            .filter(|(n, _)| n.starts_with(prefix))
            .map(|(n, d)| Candidate::new(n, d))
            .collect();
        }
        if context.native_mcp {
            return crate::native_mcp::completion(
                &layers,
                &harnesses,
                state,
                context.native_id,
                context.explicit.is_some(),
            )
            .into_iter()
            .filter(|(n, _)| n.starts_with(prefix))
            .map(|(n, d)| Candidate::new(n, d))
            .collect();
        }
        let Ok(plugins) = crate::native_plugins::read(&layers, &harnesses) else {
            return Vec::new();
        };
        let mut names = std::collections::BTreeMap::new();
        for plugin in plugins.into_iter().filter(|p| p.state == state) {
            let description = format!(
                "{} {} ({})",
                plugin.harness.binary(),
                plugin.id,
                plugin.state.as_str()
            );
            // A native ID needs a Harness flag; an unflagged logical name targets every installed Harness.
            if context.explicit.is_some() {
                names.insert(plugin.id.clone(), description.clone());
            }
            if !context.native_id {
                for logical in plugin.logical {
                    names.insert(logical, description.clone());
                }
            }
        }
        return names
            .into_iter()
            .filter(|(n, _)| n.starts_with(prefix))
            .map(|(n, d)| Candidate::new(n, d))
            .collect();
    }
    if arg.get_id() == "id" {
        return crate::session_store::records()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|record| {
                let id = ayran_core::session::short_id(&record.id);
                (prefix.is_empty() || ayran_core::session::matches_prefix(&id, prefix)).then(|| {
                    Candidate::new(
                        id,
                        format!(
                            "{} {} {}",
                            record.harness.binary(),
                            record.last_used_at,
                            crate::session_list::summary(&record.request)
                        ),
                    )
                })
            })
            .collect();
    }
    if arg.get_id() == "model" {
        return context.model_candidates(prefix);
    }
    let (completed, tail) = arg
        .get_value_delimiter()
        .and_then(|delimiter| prefix.rsplit_once(delimiter))
        .unwrap_or(("", prefix));
    let leading = &prefix[..prefix.len() - tail.len()];
    if !leading.is_empty() && !accepts_value(arg, completed) {
        return Vec::new();
    }
    let prefix = tail;
    if matches!(
        arg.get_id().as_str(),
        "plugin" | "no-plugin" | "skill" | "no-skill" | "mcp" | "no-mcp" | "profile" | "no-profile"
    ) {
        let candidates = if matches!(arg.get_id().as_str(), "profile" | "no-profile") {
            context.profile_candidates(prefix, completed, arg.get_id() == "no-profile")
        } else if matches!(arg.get_id().as_str(), "skill" | "no-skill") {
            context.member_candidates(
                prefix,
                completed,
                arg.get_id() == "no-skill",
                MemberKind::Skill,
            )
        } else if matches!(arg.get_id().as_str(), "mcp" | "no-mcp") {
            context.member_candidates(prefix, completed, arg.get_id() == "no-mcp", MemberKind::Mcp)
        } else {
            context.plugin_candidates(prefix, completed, arg.get_id() == "no-plugin")
        };
        return candidates
            .into_iter()
            .map(|mut candidate| {
                candidate.value = format!("{leading}{}", candidate.value);
                candidate
            })
            .collect();
    }
    if matches!(arg.get_value_hint(), clap::ValueHint::FilePath) {
        return file_candidates(prefix);
    }
    let mut candidates: Vec<_> = arg
        .get_value_parser()
        .possible_values()
        .into_iter()
        .flatten()
        .filter(|value| !value.is_hide_set() && value.get_name().starts_with(prefix))
        .map(|value| {
            Candidate::new(
                format!("{leading}{}", value.get_name()),
                value
                    .get_help()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("{}: {}", arg_description(arg), value.get_name())),
            )
        })
        .collect();
    candidates.sort_by(|a, b| a.value.cmp(&b.value));
    candidates
}

fn file_candidates(prefix: &str) -> Vec<Candidate> {
    let (directory, name_prefix) = prefix
        .rfind(std::path::is_separator)
        .map_or(("", prefix), |index| prefix.split_at(index + 1));
    let path = if directory.is_empty() { "." } else { directory };
    let Ok(entries) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            return Vec::new();
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(name_prefix) || name.contains(['\n', '\r']) {
            continue;
        }
        let suffix = if entry.path().is_dir() { "/" } else { "" };
        candidates.push(Candidate::new(
            format!("{directory}{name}{suffix}"),
            if suffix.is_empty() {
                "File"
            } else {
                "Directory"
            },
        ));
    }
    candidates.sort_by(|a, b| a.value.cmp(&b.value));
    candidates
}

#[cfg(test)]
mod tests {
    use super::candidates;

    #[test]
    fn list_values_complete_after_the_last_comma_and_flags_repeat() {
        let grammar = || {
            clap::Command::new("ayran").arg(
                clap::Arg::new("fixed-list")
                    .long("skill")
                    .action(clap::ArgAction::Append)
                    .value_delimiter(',')
                    .value_parser(["alpha", "beta", "build"]),
            )
        };
        assert_eq!(
            super::complete(grammar(), &["ayran", "--skill", "alpha,b"]),
            ["alpha,beta", "alpha,build"]
        );
        assert_eq!(
            super::complete(grammar(), &["ayran", "--skill=alpha,beta,b"]),
            ["--skill=alpha,beta,beta", "--skill=alpha,beta,build"]
        );
        assert_eq!(
            super::complete(grammar(), &["ayran", "--skill", "alpha,beta", "--s"]),
            ["--skill"]
        );
        assert_eq!(
            super::complete(grammar(), &["ayran", "--skill=alpha,beta", "--s"]),
            ["--skill"]
        );
        assert!(super::complete(grammar(), &["ayran", "--skill", "unknown,b"]).is_empty());
    }

    #[test]
    fn equals_values_complete_and_attached_short_values_do_not() {
        assert_eq!(
            candidates(&["ayran", "--harness=co"]),
            ["--harness=codex", "--harness=copilot"]
        );
        assert_eq!(candidates(&["ayran", "--effort=hi"]), ["--effort=high"]);
        assert_eq!(candidates(&["ayran", "--effort=high", "con"]), ["config"]);
        assert!(candidates(&["ayran", "--model=custom"]).is_empty());
        assert!(candidates(&["ayran", "-ehi"]).is_empty());
        assert!(candidates(&["ayran", "--harness=invalid", ""]).is_empty());
        assert!(candidates(&["ayran", "--quiet=true", ""]).is_empty());
    }

    #[test]
    fn used_scopes_and_scalar_flags_are_not_offered_again() {
        for scope in ["-u", "--global", "-p", "-l"] {
            assert_eq!(
                candidates(&["ayran", "config", "edit", scope, ""]),
                ["--help", "-h"]
            );
        }
        assert_eq!(
            candidates(&["ayran", "config", "edit", "--file", "custom.toml", ""]),
            ["--help", "-h"]
        );
        assert!(candidates(&["ayran", "--effort", "high", "--e"]).is_empty());
        assert!(candidates(&["ayran", "-m", "custom", "-m"]).is_empty());
        assert!(candidates(&["ayran", "--codex", "--harness"]).is_empty());
    }

    #[test]
    fn completes_subcommands_by_prefix_in_the_current_command() {
        assert_eq!(candidates(&["ayran", "con"]), ["config"]);
        assert_eq!(candidates(&["ayran", "config", "ed"]), ["edit"]);
        let config = candidates(&["ayran", "config", ""]);
        for name in ["edit", "path", "list"] {
            assert!(config.iter().any(|candidate| candidate == name));
        }
        assert!(!config.iter().any(|candidate| candidate == "--codex"));
    }
    #[test]
    fn completes_fixed_values_and_returns_to_flags_after_a_value() {
        assert_eq!(
            candidates(&["ayran", "--harness", "co"]),
            ["codex", "copilot"]
        );
        assert_eq!(candidates(&["ayran", "-e", "hi"]), ["high"]);
        assert_eq!(candidates(&["ayran", "activate", "b"]), ["bash"]);
        assert!(candidates(&["ayran", "--model", ""]).is_empty());
        assert_eq!(
            candidates(&["ayran", "--model", "custom", "con"]),
            ["config"]
        );
        assert!(
            candidates(&["ayran", "activate", "bash", ""])
                .iter()
                .all(|word| word.starts_with('-'))
        );
    }

    #[test]
    fn positional_values_do_not_hide_the_commands_own_flags() {
        assert_eq!(candidates(&["ayran", "activate", "--h"]), ["--help"]);
        assert_eq!(candidates(&["ayran", "activate", "-h"]), ["-h"]);
    }

    #[test]
    fn hidden_spellings_are_accepted_but_never_offered() {
        let root = candidates(&["ayran", ""]);
        assert!(
            !root
                .iter()
                .any(|word| word == "--alias" || word == "__complete")
        );
        let config = candidates(&["ayran", "config", ""]);
        assert!(!config.iter().any(|word| word == "ls"));
        let edit = candidates(&["ayran", "config", "edit", ""]);
        assert!(!edit.iter().any(|word| word == "-g" || word == "--global"));
        assert_eq!(candidates(&["ayran", "config", "ls", "--j"]), ["--json"]);
        assert_eq!(
            candidates(&["ayran", "config", "edit", "--global", "--h"]),
            ["--help"]
        );
        assert_eq!(
            candidates(&["ayran", "config", "edit", "-g", "--h"]),
            ["--help"]
        );
        assert_eq!(candidates(&["ayran", "--alias", "cr", "--e"]), ["--effort"]);
    }

    #[test]
    fn unrecognised_words_and_passthrough_stop_completion() {
        for words in [
            vec!["ayran", "confgi"],
            vec!["ayran", "unknown", ""],
            vec!["ayran", "config", "unknown", ""],
            vec!["ayran", "config", "--codex", ""],
            vec!["ayran", "--unknown", ""],
            vec!["ayran", "--harness", "invalid", ""],
            vec!["ayran", "activate", "fish", ""],
            vec!["ayran", "--", ""],
            vec!["ayran", "--model", "--", ""],
            vec!["ayran", "config", "edit", "--", "con"],
            vec!["ayran", "--", "--harness", "co"],
            vec!["ayran"],
            vec![],
        ] {
            assert!(candidates(&words).is_empty(), "{words:?}");
        }
    }
}
