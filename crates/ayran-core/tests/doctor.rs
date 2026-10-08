use ayran_core::{
    config::{ConfigLayers, Skill, Sourced},
    diagnostic::Severity,
    doctor::{DoctorState, audit},
    harness::Harness,
};
#[test]
fn installed_harness_defaults_grade_errors_and_order_by_name() {
    let mut layers = ConfigLayers::default();
    for name in ["z", "a"] {
        layers.skills.insert(
            name.into(),
            Sourced {
                value: Skill {
                    default: true,
                    ..Skill::default()
                },
                path: "fixture.toml".into(),
            },
        );
    }
    let state = DoctorState {
        installed: vec![Harness::Codex],
        ..DoctorState::default()
    };
    let diagnostics = audit(&layers, &state);
    assert_eq!(diagnostics.len(), 6);
    assert_eq!(
        diagnostics
            .iter()
            .map(|d| (
                d.harness.unwrap(),
                d.severity,
                d.capability.as_ref().unwrap().name.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            (Harness::Claude, Severity::Warning, "a"),
            (Harness::Claude, Severity::Warning, "z"),
            (Harness::Codex, Severity::Error, "a"),
            (Harness::Codex, Severity::Error, "z"),
            (Harness::Copilot, Severity::Warning, "a"),
            (Harness::Copilot, Severity::Warning, "z")
        ]
    );
}
#[test]
fn launch_missing_binding_carries_the_known_harness_and_capability() {
    let mut layers = ConfigLayers::default();
    layers.skills.insert(
        "tdd".into(),
        Sourced {
            value: Skill::default(),
            path: "fixture.toml".into(),
        },
    );
    let diagnostics = ayran_core::resolve::resolve(
        ayran_core::resolve::Request {
            harness: Some(Harness::Codex),
            skills: vec!["tdd".into()],
            ..Default::default()
        },
        &layers,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .err()
    .unwrap();
    assert_eq!(diagnostics[0].code, "missing-binding");
    assert_eq!(diagnostics[0].harness, Some(Harness::Codex));
    assert_eq!(diagnostics[0].capability.as_ref().unwrap().name, "tdd");
}

#[test]
fn selected_disabled_items_retain_their_logical_and_native_names() {
    use ayran_core::{
        config::SkillBinding,
        mcp::{McpBinding, McpServer, McpState},
        resolve::{Request, resolve},
        skills::SkillState,
    };
    let mut layers = ConfigLayers::default();
    layers.skills.insert(
        "review".into(),
        Sourced {
            path: "fixture.toml".into(),
            value: Skill {
                all: Some(SkillBinding::Native("native-review".into())),
                ..Skill::default()
            },
        },
    );
    layers.mcp.insert(
        "files".into(),
        Sourced {
            path: "fixture.toml".into(),
            value: McpServer {
                all: Some(McpBinding::Native("native-files".into())),
                ..McpServer::default()
            },
        },
    );
    let skills = SkillState {
        personal: ["native-review".into()].into(),
        disabled: ["native-review".into()].into(),
        ..SkillState::default()
    };
    let mcp = McpState {
        user: ["native-files".into()].into(),
        disabled: ["native-files".into()].into(),
        ..McpState::default()
    };
    for (harness, code, logical, item) in [
        (
            Harness::Copilot,
            "skill-disabled",
            "review",
            "native-review",
        ),
        (Harness::Claude, "mcp-disabled", "files", "native-files"),
    ] {
        let plan = resolve(
            Request {
                harness: Some(harness),
                skills: vec!["review".into()],
                mcp: vec!["files".into()],
                ..Request::default()
            },
            &layers,
            &Default::default(),
            &skills,
            &mcp,
        )
        .unwrap_or_else(|_| panic!("fixture launch failed"));
        let d = plan.diagnostics.iter().find(|d| d.code == code).unwrap();
        assert_eq!(d.harness, Some(harness));
        assert_eq!(d.capability.as_ref().unwrap().name, logical);
        assert_eq!(d.item.as_ref().unwrap().as_str(), item);
    }
}

#[test]
fn native_inventory_checks_grade_and_order_all_gaps() {
    use ayran_core::{config::SkillBinding, doctor::HarnessState, skills::SkillState};
    let mut layers = ConfigLayers::default();
    for (name, default) in [("z", false), ("a", true)] {
        layers.skills.insert(
            name.into(),
            Sourced {
                path: "fixture.toml".into(),
                value: Skill {
                    default,
                    all: Some(SkillBinding::Native(name.into())),
                    ..Default::default()
                },
            },
        );
    }
    let state = DoctorState {
        installed: vec![Harness::Codex],
        harnesses: vec![HarnessState {
            harness: Harness::Codex,
            plugins: None,
            skills: Some(SkillState::default()),
            mcp: None,
        }],
        ..Default::default()
    };
    let diagnostics = audit(&layers, &state);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d.code == "native-not-found")
            .map(|d| (
                d.code,
                d.severity,
                d.capability.as_ref().unwrap().name.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("native-not-found", Severity::Error, "a"),
            ("native-not-found", Severity::Warning, "z")
        ]
    );
}

#[test]
fn native_skills_accept_discovered_scopes_and_invocation_aliases() {
    use ayran_core::{
        config::SkillBinding,
        doctor::HarnessState,
        skills::{CodexSkill, SkillState},
    };
    for harness in [Harness::Claude, Harness::Codex, Harness::Copilot] {
        let mut layers = ConfigLayers::default();
        layers.skills.insert(
            "review".into(),
            Sourced {
                path: "fixture.toml".into(),
                value: Skill {
                    all: Some(SkillBinding::Native("shortcut".into())),
                    ..Default::default()
                },
            },
        );
        let skills = SkillState {
            aliases: [("shortcut".into(), "review".into())].into(),
            project: ["review".into()].into(),
            codex: [(
                "SKILL.md".into(),
                CodexSkill {
                    name: "review".into(),
                    personal: false,
                },
            )]
            .into(),
            ..Default::default()
        };
        let state = DoctorState {
            harnesses: vec![HarnessState {
                harness,
                plugins: None,
                skills: Some(skills),
                mcp: None,
            }],
            ..Default::default()
        };
        assert!(
            audit(&layers, &state)
                .iter()
                .all(|d| d.code == "leak" && d.severity == Severity::Note)
        );
    }
}

#[test]
fn worst_case_leaks_group_protected_items_and_survive_independent_inventory_failures() {
    use ayran_core::{
        doctor::HarnessState, enumerate::InstalledPlugins, mcp::McpState, skills::SkillState,
    };
    let state = DoctorState {
        installed: vec![Harness::Claude, Harness::Codex],
        harnesses: vec![
            HarnessState {
                harness: Harness::Claude,
                plugins: Some(InstalledPlugins {
                    user: ["one@skills-dir".into(), "two@skills-dir".into()].into(),
                    project: ["one@skills-dir".into(), "two@skills-dir".into()].into(),
                    ..Default::default()
                }),
                skills: Some(SkillState {
                    personal: ["builtin".into(), "managed".into()].into(),
                    bundled: ["builtin".into()].into(),
                    enterprise: ["managed".into()].into(),
                    ..Default::default()
                }),
                mcp: None,
            },
            HarnessState {
                harness: Harness::Codex,
                plugins: None,
                skills: None,
                mcp: Some(McpState {
                    user: ["files".into(), "search".into()].into(),
                    project: ["files".into(), "search".into()].into(),
                    ..Default::default()
                }),
            },
        ],
        ..Default::default()
    };
    let diagnostics = audit(&ConfigLayers::default(), &state);
    assert_eq!(diagnostics.len(), 6);
    assert_eq!(
        diagnostics
            .iter()
            .map(|d| (
                d.harness.unwrap(),
                d.severity,
                d.cause.as_deref().unwrap().as_str(),
                d.item.as_deref().map(String::as_str)
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                Harness::Claude,
                Severity::Warning,
                "claude-bundled-shadow",
                Some("builtin")
            ),
            (
                Harness::Claude,
                Severity::Warning,
                "claude-enterprise-shadow",
                Some("managed")
            ),
            (
                Harness::Claude,
                Severity::Warning,
                "claude-project-plugin-shadow",
                Some("one@skills-dir, two@skills-dir")
            ),
            (
                Harness::Claude,
                Severity::Note,
                "claude-synced-plugin",
                None
            ),
            (
                Harness::Codex,
                Severity::Warning,
                "mcp-project-shadow",
                Some("files, search")
            ),
            (Harness::Codex, Severity::Note, "codex-remote-plugin", None),
        ]
    );
}

#[test]
fn claude_plugin_overlap_uses_effective_names_and_endpoint_environments() {
    use ayran_core::plugin_audit::{PluginContents, PluginInventory};
    use std::collections::{BTreeMap, BTreeSet};
    let plugin = |id: &str, env: serde_json::Value| PluginContents {
        id: id.into(),
        namespace: "same".into(),
        root: id.into(),
        skills: BTreeSet::from(["same:review".into()]),
        servers: BTreeMap::from([(
            "files".into(),
            serde_json::json!({"command":"/bin/false","args":["same"],"env":env}),
        )]),
    };
    let inventory = PluginInventory {
        plugins: vec![
            plugin(
                "alpha@market",
                serde_json::json!({"B":"2","A":"1","CLAUDE_PLUGIN_ROOT":"/alpha"}),
            ),
            plugin(
                "zeta@market",
                serde_json::json!({"A":"1","B":"2","CLAUDE_PLUGIN_ROOT":"/zeta"}),
            ),
            plugin("different@market", serde_json::json!({"A":"different"})),
        ],
        ..Default::default()
    };
    let diagnostics = ayran_core::plugin_audit::audit(
        Harness::Claude,
        &inventory,
        &ConfigLayers::default(),
        &DoctorState::default(),
    );
    let clashes: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.code == "plugin-item-clash")
        .collect();
    assert_eq!(clashes.len(), 1);
    assert_eq!(
        clashes[0].item.as_deref().map(String::as_str),
        Some("same:review")
    );
    let dedup: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.code == "plugin-server-dedup")
        .collect();
    assert_eq!(dedup.len(), 1);
    assert!(dedup[0].message.contains("alpha@market"));
    assert!(dedup[0].message.contains("zeta@market"));
    assert!(!dedup[0].message.contains("different@market"));
    assert!(dedup[0].message.contains("loading order"));
}

#[test]
fn claude_remote_endpoint_signatures_unwrap_proxies_and_normalize_urls() {
    use ayran_core::plugin_audit::{PluginContents, PluginInventory};
    let inventory = PluginInventory {
        plugins: vec![PluginContents {
            id: "alpha@market".into(),
            namespace: "alpha".into(),
            root: "alpha".into(),
            skills: Default::default(),
            servers: std::collections::BTreeMap::from([
                (
                    "normalized".into(),
                    serde_json::json!({"url":"https://EXAMPLE.com:443/mcp/#fragment"}),
                ),
                (
                    "proxied".into(),
                    serde_json::json!({"url":"https://api.anthropic.com/v1/code/proxy?mcp_url=https%3A%2F%2Fexample.com%2Fmcp"}),
                ),
                (
                    "query".into(),
                    serde_json::json!({"url":"https://example.com/mcp?q=different"}),
                ),
            ]),
        }],
        manual: std::collections::BTreeMap::from([(
            "manual".into(),
            serde_json::json!({"url":"https://example.com/mcp"}),
        )]),
    };
    let diagnostics = ayran_core::plugin_audit::audit(
        Harness::Claude,
        &inventory,
        &ConfigLayers::default(),
        &DoctorState::default(),
    );
    let items: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.code == "plugin-server-dedup")
        .map(|d| d.item.as_deref().unwrap().as_str())
        .collect();
    assert_eq!(
        items,
        vec!["plugin:alpha:normalized", "plugin:alpha:proxied"]
    );
}

#[test]
fn builtin_layer_is_silent_until_reachable_and_user_redefinitions_are_audited() {
    let mut layers = ConfigLayers::builtin();
    let state = DoctorState {
        installed: vec![Harness::Codex],
        ..Default::default()
    };
    assert!(audit(&layers, &state).is_empty());
    layers.skills.get_mut("ayran").unwrap().value.default = true;
    let diagnostics = audit(&layers, &state);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "unsupported-binding");
    assert_eq!(diagnostics[0].severity, Severity::Error);
    assert_eq!(diagnostics[0].layer.as_deref(), Some("built-in"));
    assert_eq!(
        diagnostics[0].hint.as_deref(),
        Some("ayran install --skill ayran --codex")
    );
    layers.skills.get_mut("ayran").unwrap().value.default = false;
    layers.skills.get_mut("ayran").unwrap().path = "user.toml".into();
    let diagnostics = audit(&layers, &state);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].severity, Severity::Warning);
}
