//! Read-only native Skill and MCP listings using the launch discovery and proofs.
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::PathBuf,
};

use crate::native_plugins::NativeState;
use ayran_core::{
    config::{ConfigLayers, SkillBinding},
    diagnostic::{CapabilityKind, Diagnostic, Severity},
    doctor::reachable_capabilities,
    enumerate::InstalledPlugins,
    harness::Harness,
    mcp::{McpBinding, McpState},
    skills::SkillState,
};
use clap::ArgMatches;

#[derive(Clone)]
pub(crate) struct Row {
    pub(crate) harness: Harness,
    pub(crate) name: String,
    pub(crate) path: Option<PathBuf>,
    pub(crate) source: &'static str,
    pub(crate) state: NativeState,
    pub(crate) layer: String,
    pub(crate) logical: Vec<String>,
    pub(crate) default: bool,
    pub(crate) reachable: bool,
}

pub(crate) fn read(
    layers: &ConfigLayers,
    harnesses: &[Harness],
    kind: CapabilityKind,
) -> Result<Vec<Row>, Diagnostic> {
    let real_home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let installed = crate::native_plugins::installed();
    let mut rows = Vec::new();
    for &harness in harnesses {
        let home = crate::native_plugins::home(layers, harness)?;
        let args: Vec<OsString> = layers
            .settings(harness)
            .args
            .as_ref()
            .map(|a| a.value.iter().map(OsString::from).collect())
            .unwrap_or_default();
        let mut skills = SkillState::default();
        let mut mcp = McpState::default();
        if kind == CapabilityKind::Skill {
            match harness {
                Harness::Claude => {
                    crate::skill_enumeration::read_claude(&mut skills, real_home.as_deref(), &home)?
                }
                Harness::Codex => {
                    crate::codex_skill_enumeration::read(&mut skills, real_home.as_deref(), &home)?
                }
                Harness::Copilot => {
                    crate::copilot_skill_enumeration::read(
                        &mut skills,
                        real_home.as_deref(),
                        &home,
                    )?;
                    // Launch drops this inherited variable; listing also shows what a bare Harness sees.
                    if let Ok(directories) = std::env::var("COPILOT_SKILLS_DIRS") {
                        for directory in directories
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            crate::skill_enumeration::read_root(
                                std::path::Path::new(directory),
                                &mut skills.custom,
                                &mut skills.aliases,
                                harness,
                            )?;
                        }
                    }
                }
            }
        }
        if kind == CapabilityKind::Mcp {
            mcp = match harness {
                Harness::Claude => crate::mcp_enumeration::read_claude(&home)?,
                Harness::Codex => crate::mcp_enumeration::read_codex(&home, real_home.as_deref())?,
                Harness::Copilot => crate::copilot_mcp_enumeration::read(&home)?,
            };
            if harness == Harness::Codex {
                mcp.connectors = crate::mcp_enumeration::read_codex_connectors(&home)?;
            }
        }
        let codex_sources: BTreeMap<_, _> = skills
            .codex
            .iter()
            .map(|(path, skill)| {
                let source = if path.starts_with(home.directory.join("skills/.system")) {
                    "bundled"
                } else if path.starts_with("/etc/codex/skills") {
                    "enterprise"
                } else if skill.personal {
                    "personal"
                } else {
                    "project"
                };
                (path.clone(), source)
            })
            .collect();
        let mut plugins = InstalledPlugins::default();
        match harness {
            Harness::Claude => {
                crate::claude_config::apply(&home, &args, &mut plugins, &mut skills, &mut mcp)?
            }
            Harness::Codex => crate::codex_config::apply(
                &home,
                real_home.as_deref(),
                &crate::codex_config::profiles(&args),
                &mut plugins,
                &mut skills,
                &mut mcp,
            )?,
            Harness::Copilot => {}
        }
        let reachable = reachable_capabilities(layers, &installed, harness, kind);
        if kind == CapabilityKind::Skill {
            if harness == Harness::Codex {
                for (path, skill) in &skills.codex {
                    if crate::skill_enumeration::belongs_to_plugin(path)? {
                        continue;
                    }
                    let source = codex_sources.get(path).copied().unwrap_or("personal");
                    let state = if !skills.codex_skip_redundant {
                        NativeState::Unknown
                    } else {
                        NativeState::from_enabled(!skills.codex_off.contains(path))
                    };
                    rows.push(
                        Row::new(
                            harness,
                            &skill.name,
                            source,
                            state,
                            skills
                                .codex_layers
                                .get(path)
                                .cloned()
                                .unwrap_or_else(|| "native default".into()),
                        )
                        .with_path(path.clone())
                        .bind_skills(layers, &skills, &reachable),
                    );
                }
            } else {
                for (names, source) in [
                    (&skills.personal, "personal"),
                    (&skills.project, "project"),
                    (&skills.enterprise, "enterprise"),
                    (&skills.bundled, "bundled"),
                    (&skills.custom, "custom"),
                ] {
                    for name in names {
                        if name.starts_with("anthropic-skills:") {
                            continue;
                        }
                        let state = if harness == Harness::Copilot {
                            NativeState::On
                        } else {
                            match skills.claude_overrides.as_ref().map(|m| m.get(name)) {
                                Some(Some(value)) if value.as_str() == Some("off") => {
                                    NativeState::Off
                                }
                                Some(None) => NativeState::On,
                                Some(Some(value)) if value.as_str() == Some("on") => {
                                    NativeState::On
                                }
                                _ => NativeState::Unknown,
                            }
                        };
                        let layer = if state == NativeState::Unknown {
                            "effective settings uncertain".into()
                        } else {
                            skills
                                .native_layers
                                .get(name)
                                .cloned()
                                .unwrap_or_else(|| "native default".into())
                        };
                        rows.push(
                            Row::new(harness, name, source, state, layer)
                                .bind_skills(layers, &skills, &reachable),
                        );
                    }
                }
            }
        } else {
            let mut sources = mcp.sources.clone();
            for name in &mcp.user {
                if !sources.iter().any(|(id, _)| id == name) {
                    sources.insert((name.clone(), "user"));
                }
            }
            sources.extend(mcp.project.iter().map(|name| (name.clone(), "project")));
            for (name, source) in &sources {
                let state = match harness {
                    Harness::Claude if mcp.claude_off.contains(name) => NativeState::Off,
                    Harness::Codex => mcp
                        .codex_enabled
                        .get(name)
                        .map_or(NativeState::Unknown, |on| NativeState::from_enabled(*on)),
                    Harness::Copilot
                        if mcp.copilot_user_off.contains(name)
                            && !mcp.project.contains(name)
                            && !copilot_enables(&args, name) =>
                    {
                        NativeState::Off
                    }
                    _ => NativeState::Unknown,
                };
                let layer = mcp
                    .native_layers
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| "native state unproven".into());
                rows.push(
                    Row::new(harness, name, source, state, layer).bind_mcp(layers, &reachable),
                );
            }
            for name in &mcp.connectors {
                rows.push(
                    Row::new(
                        harness,
                        name,
                        "connector",
                        NativeState::Unknown,
                        "local account cache (incomplete)".into(),
                    )
                    .bind_mcp(layers, &reachable),
                );
            }
        }
    }
    rows.sort_by(|a, b| {
        (a.harness.binary(), &a.name, a.source, &a.layer).cmp(&(
            b.harness.binary(),
            &b.name,
            b.source,
            &b.layer,
        ))
    });
    Ok(rows)
}

impl Row {
    fn new(
        harness: Harness,
        name: &str,
        source: &'static str,
        state: NativeState,
        layer: String,
    ) -> Self {
        Self {
            harness,
            name: name.into(),
            path: None,
            source,
            state,
            layer,
            logical: Vec::new(),
            default: false,
            reachable: false,
        }
    }
    fn with_path(mut self, path: PathBuf) -> Self {
        self.path = Some(path);
        self
    }
    fn bind_skills(
        mut self,
        layers: &ConfigLayers,
        skills: &SkillState,
        reachable: &BTreeSet<String>,
    ) -> Self {
        self.logical = layers.skills.iter().filter(|(_, s)| matches!(s.value.binding(self.harness), Some(SkillBinding::Native(id)) if *id == self.name || skills.aliases.get(id).is_some_and(|alias| *alias == self.name))).map(|(n,_)| n.clone()).collect();
        self.default = self.logical.iter().any(|n| layers.skills[n].value.default);
        self.reachable = self.logical.iter().any(|n| reachable.contains(n));
        self
    }
    fn bind_mcp(mut self, layers: &ConfigLayers, reachable: &BTreeSet<String>) -> Self {
        self.logical = layers
            .mcp
            .iter()
            .filter(|(_, s)| match s.value.binding(self.harness) {
                Some(McpBinding::Connector(id)) if self.source == "connector" => *id == self.name,
                Some(McpBinding::Native(id)) if self.source != "connector" => *id == self.name,
                _ => false,
            })
            .map(|(n, _)| n.clone())
            .collect();
        self.default = self.logical.iter().any(|n| layers.mcp[n].value.default);
        self.reachable = self.logical.iter().any(|n| reachable.contains(n));
        self
    }
}

fn copilot_enables(args: &[OsString], name: &str) -> bool {
    args.iter().enumerate().any(|(index, arg)| {
        arg.to_str() == Some(format!("--enable-mcp-server={name}").as_str())
            || (arg == "--enable-mcp-server"
                && args.get(index + 1).and_then(|arg| arg.to_str()) == Some(name))
    })
}

pub fn run(kind: CapabilityKind, matches: &ArgMatches) -> i32 {
    let mut diagnostics = Vec::new();
    let mut rows = Vec::new();
    let result = ConfigLayers::load().and_then(|layers| {
        let selected = crate::native_command::selected(matches);
        let harnesses: Vec<_> = crate::native_plugins::installed()
            .into_iter()
            .filter(|h| selected.is_none_or(|s| s == *h))
            .collect();
        if let Some(h) = selected
            && !harnesses.contains(&h)
        {
            return Err(Diagnostic::error(
                "harness-not-found",
                format!("{} is not installed", h.binary()),
                None,
            ));
        }
        rows = read(&layers, &harnesses, kind)?;
        Ok(())
    });
    if let Err(d) = result {
        diagnostics.push(d);
    }
    if matches.get_flag("json") {
        let mut output = crate::list_command::json_envelope(&diagnostics);
        output[if kind == CapabilityKind::Skill { "skills" } else { "mcp" }] = serde_json::json!(rows.iter().map(|r| serde_json::json!({"harness": r.harness, "name": r.name, "source": r.source, "state": r.state.as_str(), "layer": r.layer, "logical": r.logical, "default": r.default, "reachable": r.reachable})).collect::<Vec<_>>());
        println!("{output}");
    } else {
        crate::list_command::print_row(
            [
                "harness",
                "name",
                "source",
                "state",
                "layer",
                "logical",
                "default",
                "reachable",
            ]
            .map(str::to_owned)
            .to_vec(),
        );
        for row in rows {
            crate::list_command::print_row(vec![
                row.harness.binary().into(),
                row.name,
                row.source.into(),
                row.state.as_str().into(),
                row.layer,
                row.logical.join(","),
                row.default.to_string(),
                row.reachable.to_string(),
            ]);
        }
        for d in &diagnostics {
            crate::render(d, false);
        }
    }
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        3
    } else {
        0
    }
}
