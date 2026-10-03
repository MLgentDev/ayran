use ayran_core::config::{ConfigLayers, Plugin, PluginBinding, Sourced};
use ayran_core::enumerate::InstalledPlugins;
use ayran_core::harness::Harness;
use ayran_core::resolve::{Request, resolve};

fn request() -> Request {
    Request {
        alias: None,
        harness: Some(Harness::Claude),
        model: None,
        effort: None,
        plugins: vec![],
        skills: vec![],
        mcp: vec![],
        profiles: vec![],
        no_plugins: vec![],
        no_skills: vec![],
        no_mcp: vec![],
        no_profiles: vec![],
        no_defaults: false,
        no_harness_args: false,
        passthrough: vec![],
    }
}

fn layers() -> ConfigLayers {
    let mut layers = ConfigLayers::default();
    layers.plugins.insert(
        "review".into(),
        Sourced {
            value: Plugin {
                all: Some(PluginBinding::Native("review@acme".into())),
                default: true,
                ..Plugin::default()
            },
            path: "/user/ayran.toml".into(),
        },
    );
    layers
}

fn installed() -> InstalledPlugins {
    InstalledPlugins {
        user: ["review@acme".into()].into(),
        ..InstalledPlugins::default()
    }
}

#[test]
fn explicit_selection_has_one_trace_even_when_defaults_are_disabled() {
    let mut request = request();
    request.plugins.push("review".into());
    request.no_defaults = true;
    let plan = resolve(
        request,
        &layers(),
        &installed(),
        &Default::default(),
        &Default::default(),
    )
    .unwrap_or_else(|_| panic!("resolution failed"));
    let trace: Vec<_> = plan
        .trace
        .plugins
        .iter()
        .filter(|line| line.starts_with("Plugin review:"))
        .map(String::as_str)
        .collect();
    assert_eq!(
        trace,
        ["Plugin review: explicit (--plugin) → review@acme (/user/ayran.toml)"]
    );
}

#[test]
fn plugin_precedence_combines_defaults_explicit_origins_and_disables() {
    #[derive(Debug, Default)]
    struct Case {
        cli_selects: bool,
        alias_selects: bool,
        config_disable: bool,
        alias_disable: bool,
        cli_disable: bool,
        no_defaults: bool,
        alias_no_defaults: bool,
    }
    for (case, enabled) in [
        (Case::default(), true),
        (
            Case {
                config_disable: true,
                ..Case::default()
            },
            false,
        ),
        (
            Case {
                alias_disable: true,
                ..Case::default()
            },
            false,
        ),
        (
            Case {
                cli_selects: true,
                config_disable: true,
                alias_disable: true,
                ..Case::default()
            },
            true,
        ),
        (
            Case {
                alias_selects: true,
                config_disable: true,
                alias_disable: true,
                ..Case::default()
            },
            true,
        ),
        (
            Case {
                cli_selects: true,
                alias_selects: true,
                config_disable: true,
                alias_disable: true,
                cli_disable: true,
                ..Case::default()
            },
            false,
        ),
        (
            Case {
                no_defaults: true,
                ..Case::default()
            },
            false,
        ),
        (
            Case {
                cli_selects: true,
                no_defaults: true,
                ..Case::default()
            },
            true,
        ),
        (
            Case {
                alias_no_defaults: true,
                ..Case::default()
            },
            false,
        ),
        (
            Case {
                alias_selects: true,
                config_disable: true,
                alias_disable: true,
                no_defaults: true,
                alias_no_defaults: true,
                ..Case::default()
            },
            true,
        ),
    ] {
        let mut layers = layers();
        let names = |selected| {
            if selected {
                vec!["review".into()]
            } else {
                vec![]
            }
        };
        if case.config_disable {
            layers
                .disabled_plugins
                .insert("review".into(), "/project/ayran.toml".into());
        }
        layers.aliases.insert(
            "work".into(),
            ayran_core::config::Alias {
                args: vec![],
                harness_args: true,
                harness: Harness::Claude,
                model: None,
                effort: None,
                description: None,
                plugins: names(case.alias_selects),
                skills: vec![],
                profiles: vec![],
                defaults: !case.alias_no_defaults,
                disabled_profiles: vec![],
                disabled_plugins: names(case.alias_disable),
                mcp: vec![],
                disabled_mcp: vec![],
                disabled_skills: vec![],
            },
        );
        let mut request = request();
        request.alias = Some("work".into());
        request.plugins = names(case.cli_selects);
        request.no_plugins = names(case.cli_disable);
        request.no_defaults = case.no_defaults;
        let plan = resolve(
            request,
            &layers,
            &installed(),
            &Default::default(),
            &Default::default(),
        )
        .unwrap_or_else(|_| panic!("resolution failed"));
        let settings: serde_json::Value =
            serde_json::from_str(plan.args[1].to_str().unwrap()).unwrap();
        assert_eq!(
            settings["enabledPlugins"]["review@acme"], enabled,
            "{case:?}"
        );
    }
}

#[test]
fn disabling_one_logical_plugin_keeps_a_shared_native_item_selected_by_another() {
    let mut layers = layers();
    layers.plugins.insert(
        "other".into(),
        Sourced {
            value: Plugin {
                all: Some(PluginBinding::Native("review@acme".into())),
                ..Plugin::default()
            },
            path: "/project/ayran.toml".into(),
        },
    );
    let mut request = request();
    request.plugins = vec!["other".into(), "other".into()];
    request.no_plugins = vec!["review".into()];
    let plan = resolve(
        request,
        &layers,
        &installed(),
        &Default::default(),
        &Default::default(),
    )
    .unwrap_or_else(|_| panic!("resolution failed"));
    assert_eq!(
        plan.args,
        [
            "--settings",
            r#"{"disableClaudeAiConnectors":true,"enabledPlugins":{"review@acme":true},"syncClaudeAiSkills":false}"#
        ]
    );
    assert_eq!(
        plan.trace
            .plugins
            .iter()
            .filter(|line| line.starts_with("Plugin other:"))
            .count(),
        1
    );
}

#[test]
fn profile_members_obey_plugin_disables_and_explicit_origins() {
    for (explicit, cli_disable, config_disable, alias_disable, enabled) in [
        (false, false, false, false, true),
        (false, false, true, false, false),
        (false, false, false, true, false),
        (true, false, true, true, true),
        (true, true, false, false, false),
        (false, true, false, false, false),
    ] {
        let mut layers = layers();
        layers.profiles.insert(
            "kit".into(),
            Sourced {
                value: ayran_core::config::Profile {
                    plugins: vec!["review".into()],
                    skills: vec![],
                    ..Default::default()
                },
                path: "/user/ayran.toml".into(),
            },
        );
        if config_disable {
            layers
                .disabled_plugins
                .insert("review".into(), "/project/ayran.toml".into());
        }
        layers.aliases.insert(
            "work".into(),
            ayran_core::config::Alias {
                args: vec![],
                harness_args: true,
                harness: Harness::Claude,
                model: None,
                effort: None,
                description: None,
                plugins: vec![],
                skills: vec![],
                profiles: vec![],
                defaults: false,
                disabled_profiles: vec![],
                mcp: vec![],
                disabled_mcp: vec![],
                disabled_skills: vec![],
                disabled_plugins: if alias_disable {
                    vec!["review".into()]
                } else {
                    vec![]
                },
            },
        );
        let mut request = request();
        request.alias = Some("work".into());
        request.profiles = vec!["kit".into()];
        request.no_defaults = true;
        if explicit {
            request.plugins.push("review".into());
        }
        if cli_disable {
            request.no_plugins.push("review".into());
        }
        let plan = resolve(
            request,
            &layers,
            &installed(),
            &Default::default(),
            &Default::default(),
        )
        .unwrap_or_else(|_| panic!("resolution failed"));
        let settings: serde_json::Value =
            serde_json::from_str(plan.args[1].to_str().unwrap()).unwrap();
        assert_eq!(
            settings["enabledPlugins"]["review@acme"], enabled,
            "explicit={explicit}, CLI Disable={cli_disable}, config Disable={config_disable}, Alias Disable={alias_disable}"
        );
        if enabled {
            let origin = if explicit {
                "explicit (--plugin)"
            } else {
                "via Profile kit"
            };
            assert!(plan.trace.plugins.iter().any(|line| line.contains(origin)));
        }
    }
}

#[test]
fn default_profile_expands_nested_members_with_default_origin() {
    let mut layers = layers();
    layers.plugins.get_mut("review").unwrap().value.default = false;
    for (name, plugins, profiles, default) in [
        ("base", vec!["review".into()], vec![], false),
        ("team", vec![], vec!["base".into()], true),
    ] {
        layers.profiles.insert(
            name.into(),
            Sourced {
                value: ayran_core::config::Profile {
                    plugins,
                    profiles,
                    default,
                    ..Default::default()
                },
                path: "/project/ayran.toml".into(),
            },
        );
    }
    let plan = resolve(
        request(),
        &layers,
        &installed(),
        &Default::default(),
        &Default::default(),
    )
    .unwrap_or_else(|_| panic!("resolution failed"));
    assert_eq!(
        plan.args,
        [
            "--settings",
            r#"{"disableClaudeAiConnectors":true,"enabledPlugins":{"review@acme":true},"syncClaudeAiSkills":false}"#
        ]
    );
    assert!(
        plan.trace.plugins.iter().any(|line| line
            == "Plugin review: Default (via Profile team) → review@acme (/user/ayran.toml)")
    );
}

#[test]
fn default_profile_routes_obey_defaults_disables_and_stronger_origins() {
    // no Defaults, Alias Defaults, config Disable, Alias Disable, CLI Disable,
    // explicit Profile, explicit Plugin, expected activation and origin.
    for (
        no_defaults,
        alias_defaults,
        config_disable,
        alias_disable,
        cli_disable,
        explicit_profile,
        explicit_plugin,
        enabled,
        origin,
    ) in [
        (
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            true,
            "Default (via Profile team)",
        ),
        (true, true, false, false, false, false, false, false, ""),
        (false, false, false, false, false, false, false, false, ""),
        (false, true, true, false, false, false, false, false, ""),
        (false, true, false, true, false, false, false, false, ""),
        (false, true, false, false, true, false, false, false, ""),
        (
            true,
            false,
            false,
            false,
            false,
            true,
            false,
            true,
            "via Profile base",
        ),
        (
            false,
            true,
            false,
            false,
            false,
            true,
            false,
            true,
            "via Profile base",
        ),
        (
            false,
            true,
            true,
            true,
            false,
            false,
            true,
            true,
            "explicit (--plugin)",
        ),
        (
            true,
            false,
            true,
            true,
            false,
            false,
            true,
            true,
            "explicit (--plugin)",
        ),
        (false, true, false, false, true, true, true, false, ""),
    ] {
        let mut layers = layers();
        layers.plugins.get_mut("review").unwrap().value.default = false;
        // A Plugin's own suppressed Default must not suppress a live Profile route.
        layers
            .disabled_defaults
            .insert("review".into(), "/project/ayran.toml".into());
        for (name, plugins, profiles, default) in [
            ("base", vec!["review".into()], vec![], false),
            ("team", vec![], vec!["base".into()], true),
        ] {
            layers.profiles.insert(
                name.into(),
                Sourced {
                    value: ayran_core::config::Profile {
                        plugins,
                        profiles,
                        default,
                        ..Default::default()
                    },
                    path: "/project/ayran.toml".into(),
                },
            );
        }
        if config_disable {
            layers
                .disabled_plugins
                .insert("review".into(), "/project/ayran.toml".into());
        }
        layers.aliases.insert(
            "work".into(),
            ayran_core::config::Alias {
                args: vec![],
                harness_args: true,
                harness: Harness::Claude,
                model: None,
                effort: None,
                description: None,
                plugins: vec![],
                skills: vec![],
                profiles: vec![],
                defaults: alias_defaults,
                disabled_profiles: vec![],
                mcp: vec![],
                disabled_mcp: vec![],
                disabled_skills: vec![],
                disabled_plugins: if alias_disable {
                    vec!["review".into()]
                } else {
                    vec![]
                },
            },
        );
        let mut request = request();
        request.alias = Some("work".into());
        request.no_defaults = no_defaults;
        if explicit_profile {
            request.profiles.push("team".into());
        }
        if explicit_plugin {
            request.plugins.push("review".into());
        }
        if cli_disable {
            request.no_plugins.push("review".into());
        }
        let plan = resolve(
            request,
            &layers,
            &installed(),
            &Default::default(),
            &Default::default(),
        )
        .unwrap_or_else(|_| panic!("resolution failed"));
        let settings: serde_json::Value =
            serde_json::from_str(plan.args[1].to_str().unwrap()).unwrap();
        assert_eq!(
            settings["enabledPlugins"]["review@acme"], enabled,
            "no_defaults={no_defaults}, alias_defaults={alias_defaults}, config_disable={config_disable}, alias_disable={alias_disable}, cli_disable={cli_disable}, explicit_profile={explicit_profile}, explicit_plugin={explicit_plugin}"
        );
        if enabled {
            assert!(
                plan.trace.plugins.iter().any(|line| line
                    == &format!("Plugin review: {origin} → review@acme (/user/ayran.toml)")),
                "{:?}",
                plan.trace.plugins
            );
        }
    }
}

#[test]
fn alias_profiles_are_explicit_but_their_plugins_keep_profile_origin() {
    // Alias selection beats Profile Disables, while its members still obey
    // Plugin Disables. Direct Plugin selection is a separate explicit route.
    for (disable, direct_plugin, enabled) in [
        ("none", false, true),
        ("layer-profile", false, true),
        ("alias-profile", false, true),
        ("cli-profile", false, false),
        ("layer-plugin", false, false),
        ("alias-plugin", false, false),
        ("layer-plugin", true, true),
        ("cli-profile", true, true),
    ] {
        let mut layers = layers();
        layers.profiles.insert(
            "team".into(),
            Sourced {
                value: ayran_core::config::Profile {
                    plugins: vec!["review".into()],
                    skills: vec![],
                    default: true,
                    ..Default::default()
                },
                path: "/user/ayran.toml".into(),
            },
        );
        layers.aliases.insert(
            "work".into(),
            ayran_core::config::Alias {
                args: vec![],
                harness_args: true,
                harness: Harness::Claude,
                model: None,
                effort: None,
                description: None,
                skills: vec![],
                plugins: if direct_plugin {
                    vec!["review".into()]
                } else {
                    vec![]
                },
                profiles: vec!["team".into()],
                defaults: false,
                mcp: vec![],
                disabled_mcp: vec![],
                disabled_skills: vec![],
                disabled_plugins: if disable == "alias-plugin" {
                    vec!["review".into()]
                } else {
                    vec![]
                },
                disabled_profiles: if disable == "alias-profile" {
                    vec!["team".into()]
                } else {
                    vec![]
                },
            },
        );
        let mut request = request();
        request.alias = Some("work".into());
        match disable {
            "layer-profile" => {
                layers
                    .disabled_profiles
                    .insert("team".into(), "/project/ayran.toml".into());
            }
            "layer-plugin" => {
                layers
                    .disabled_plugins
                    .insert("review".into(), "/project/ayran.toml".into());
            }
            "cli-profile" => request.no_profiles.push("team".into()),
            _ => {}
        }
        let plan = resolve(
            request,
            &layers,
            &installed(),
            &Default::default(),
            &Default::default(),
        )
        .unwrap_or_else(|_| panic!("resolution failed: {disable}"));
        let settings: serde_json::Value =
            serde_json::from_str(plan.args[1].to_str().unwrap()).unwrap();
        assert_eq!(
            settings["enabledPlugins"]["review@acme"], enabled,
            "{disable}, direct Plugin={direct_plugin}"
        );
        if enabled {
            let origin = if direct_plugin {
                "explicit (Alias work)"
            } else {
                "via Profile team"
            };
            assert!(
                plan.trace.plugins.iter().any(|line| line
                    == &format!("Plugin review: {origin} → review@acme (/user/ayran.toml)")),
                "{:?}",
                plan.trace.plugins
            );
        }
    }
}

#[test]
fn profile_disables_gate_expansion_without_disabling_member_plugins() {
    // Whether review is enabled follows the surviving route, independently of
    // whether its containing Profile was disabled.
    for (disable, selected, plugin_default, explicit_plugin, enabled) in [
        ("cli", vec![], false, false, false),
        ("cli", vec!["base"], false, false, false),
        ("cli", vec!["team"], false, false, false),
        ("cli", vec!["other"], false, false, true),
        ("cli", vec![], true, false, true),
        ("cli", vec![], false, true, true),
        ("layer", vec![], false, false, false),
        ("layer", vec!["team"], false, false, false),
        ("layer", vec!["base"], false, false, true),
        ("layer", vec!["team", "base"], false, false, true),
        ("alias", vec![], false, false, false),
        ("alias", vec!["team"], false, false, false),
        ("alias", vec!["base"], false, false, true),
    ] {
        let mut layers = layers();
        layers.plugins.get_mut("review").unwrap().value.default = plugin_default;
        for (name, plugins, profiles, default) in [
            ("base", vec!["review".into()], vec![], true),
            ("team", vec![], vec!["base".into()], false),
            ("other", vec!["review".into()], vec![], false),
        ] {
            layers.profiles.insert(
                name.into(),
                Sourced {
                    value: ayran_core::config::Profile {
                        plugins,
                        profiles,
                        default,
                        ..Default::default()
                    },
                    path: "/user/ayran.toml".into(),
                },
            );
        }
        let mut request = request();
        request.profiles = selected.iter().map(|name| (*name).to_owned()).collect();
        if explicit_plugin {
            request.plugins.push("review".into());
        }
        match disable {
            "cli" => request.no_profiles.push("base".into()),
            "layer" => {
                layers
                    .disabled_profiles
                    .insert("base".into(), "/project/ayran.toml".into());
            }
            "alias" => {
                layers.aliases.insert(
                    "work".into(),
                    ayran_core::config::Alias {
                        args: vec![],
                        harness_args: true,
                        harness: Harness::Claude,
                        model: None,
                        effort: None,
                        description: None,
                        plugins: vec![],
                        skills: vec![],
                        profiles: vec![],
                        defaults: true,
                        disabled_plugins: vec![],
                        mcp: vec![],
                        disabled_mcp: vec![],
                        disabled_skills: vec![],
                        disabled_profiles: vec!["base".into()],
                    },
                );
                request.alias = Some("work".into());
            }
            _ => unreachable!(),
        }
        let plan = resolve(
            request,
            &layers,
            &installed(),
            &Default::default(),
            &Default::default(),
        )
        .unwrap_or_else(|_| panic!("resolution failed"));
        let settings: serde_json::Value =
            serde_json::from_str(plan.args[1].to_str().unwrap()).unwrap();
        assert_eq!(
            settings["enabledPlugins"]["review@acme"], enabled,
            "Disable={disable}, selected={selected:?}, Plugin Default={plugin_default}, explicit Plugin={explicit_plugin}"
        );
    }
}

#[test]
fn dangling_profile_members_are_checked_only_on_surviving_expansion_routes() {
    for member_kind in ["Plugin", "Profile"] {
        for (default, explicit, disabled, no_defaults, severity) in [
            (false, false, false, false, None),
            (true, false, false, false, Some("note")),
            (false, true, false, false, Some("error")),
            (true, true, false, false, Some("error")),
            (true, false, true, false, None),
            (true, true, true, false, None),
            (true, false, false, true, None),
            (true, true, false, true, Some("error")),
        ] {
            let mut layers = layers();
            layers.profiles.insert(
                "team".into(),
                Sourced {
                    value: ayran_core::config::Profile {
                        profiles: vec!["base".into()],
                        default,
                        ..Default::default()
                    },
                    path: "/project/ayran.toml".into(),
                },
            );
            layers.profiles.insert(
                "base".into(),
                Sourced {
                    value: ayran_core::config::Profile {
                        plugins: if member_kind == "Plugin" {
                            vec!["missing".into()]
                        } else {
                            vec![]
                        },
                        profiles: if member_kind == "Profile" {
                            vec!["missing".into()]
                        } else {
                            vec![]
                        },
                        ..Default::default()
                    },
                    path: "/user/ayran.toml".into(),
                },
            );
            let mut request = request();
            request.no_defaults = no_defaults;
            if explicit {
                request.profiles.push("team".into());
            }
            if disabled {
                request.no_profiles.push("base".into());
            }
            let result = resolve(
                request,
                &layers,
                &installed(),
                &Default::default(),
                &Default::default(),
            );
            let diagnostics = match result {
                Ok(plan) => {
                    assert_ne!(severity, Some("error"));
                    plan.diagnostics
                }
                Err(diagnostics) => {
                    assert_eq!(severity, Some("error"));
                    diagnostics
                }
            };
            let dangling: Vec<_> = diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "dangling-ref")
                .collect();
            assert_eq!(dangling.len(), usize::from(severity.is_some()));
            if let Some(severity) = severity {
                assert_eq!(dangling[0].severity.to_string(), severity);
                assert_eq!(dangling[0].layer.as_deref(), Some("/user/ayran.toml"));
                assert!(dangling[0].message.contains("Profile base"));
                assert!(
                    dangling[0]
                        .message
                        .contains(&format!("{member_kind} missing"))
                );
            }
        }
    }
}

#[test]
fn codex_skill_selection_enables_all_matching_paths_and_never_hides_admin_skills() {
    use ayran_core::config::{Skill, SkillBinding};
    use ayran_core::skills::{CodexSkill, SkillState};
    let mut config = ConfigLayers::default();
    config.skills.insert(
        "team".into(),
        Sourced {
            path: "/project/ayran.toml".into(),
            value: Skill {
                codex: Some(SkillBinding::Native("shared".into())),
                ..Skill::default()
            },
        },
    );
    config.skills.insert(
        "admin".into(),
        Sourced {
            path: "/project/ayran.toml".into(),
            value: Skill {
                codex: Some(SkillBinding::Native("admin".into())),
                ..Skill::default()
            },
        },
    );
    let state = SkillState {
        codex: [
            ("/home/.agents/skills/a/SKILL.md", "shared", true),
            ("/repo/.agents/skills/a/SKILL.md", "shared", false),
            ("/etc/codex/skills/admin/SKILL.md", "admin", false),
        ]
        .into_iter()
        .map(|(path, name, personal)| {
            (
                path.into(),
                CodexSkill {
                    name: name.into(),
                    personal,
                },
            )
        })
        .collect(),
        ..SkillState::default()
    };
    for (selection, expected) in [
        (
            vec![],
            "skills.config=[{path=\"/home/.agents/skills/a/SKILL.md\",enabled=false}]",
        ),
        (
            vec!["team"],
            "skills.config=[{path=\"/home/.agents/skills/a/SKILL.md\",enabled=true},{path=\"/repo/.agents/skills/a/SKILL.md\",enabled=true}]",
        ),
        (
            vec!["admin"],
            "skills.config=[{path=\"/etc/codex/skills/admin/SKILL.md\",enabled=true},{path=\"/home/.agents/skills/a/SKILL.md\",enabled=false}]",
        ),
    ] {
        let mut request = request();
        request.harness = Some(Harness::Codex);
        request.skills = selection.into_iter().map(str::to_owned).collect();
        let plan = resolve(
            request,
            &config,
            &InstalledPlugins::default(),
            &state,
            &Default::default(),
        )
        .unwrap_or_else(|_| panic!("Codex resolution failed"));
        assert_eq!(plan.args, ["-c", expected, "--disable", "apps"]);
        assert!(plan.generated_skills.is_none());
    }
}

#[test]
fn skill_projection_preserves_passthrough_and_rejects_distinct_items_with_one_name() {
    use ayran_core::config::{Skill, SkillBinding};
    use ayran_core::skills::SkillState;

    for (native, selected, expected_code) in [
        (true, vec!["first", "second"], None),
        (false, vec!["first"], None),
        (false, vec!["first", "second"], Some("skill-name-clash")),
    ] {
        let mut config = ConfigLayers::default();
        for (logical, target) in [("first", "/skills/one"), ("second", "/skills/two")] {
            config.skills.insert(
                logical.into(),
                Sourced {
                    path: "/project/ayran.toml".into(),
                    value: Skill {
                        all: Some(SkillBinding::Absent),
                        claude: Some(if native {
                            SkillBinding::Native("lint".into())
                        } else {
                            SkillBinding::Path(target.into())
                        }),
                        ..Skill::default()
                    },
                },
            );
        }
        let state = SkillState {
            names: [
                ("/skills/one".into(), "lint".into()),
                ("/skills/two".into(), "lint".into()),
            ]
            .into(),
            cache_root: "/cache/ayran".into(),
            personal: ["lint".into()].into(),
            ..SkillState::default()
        };
        let mut request = request();
        request.skills = selected.into_iter().map(str::to_owned).collect();
        request.passthrough = vec!["--settings".into(), "user-settings".into()];
        let result = resolve(
            request,
            &config,
            &InstalledPlugins::default(),
            &state,
            &Default::default(),
        );
        if let Some(code) = expected_code {
            let diagnostics = result
                .err()
                .expect("distinct Skills sharing a name must fail");
            assert_eq!(diagnostics[0].code, code);
        } else {
            let plan = result.unwrap_or_else(|_| panic!("Skill projection failed"));
            assert_eq!(
                &plan.args[plan.args.len() - 2..],
                ["--settings", "user-settings"]
            );
            if native {
                assert!(plan.generated_skills.is_none());
                assert_eq!(plan.args.len(), 4);
            } else {
                let cache = plan
                    .generated_skills
                    .expect("path Skill needs generated directory");
                assert_eq!(cache.targets.len(), 1);
                assert_eq!(cache.targets["lint"], std::path::Path::new("/skills/one"));
                assert_eq!(plan.args[2], "--add-dir");
            }
        }
    }
}

fn skill_layers() -> ConfigLayers {
    use ayran_core::config::{Skill, SkillBinding};
    let mut layers = ConfigLayers::default();
    layers.skills.insert(
        "review".into(),
        Sourced {
            value: Skill {
                all: Some(SkillBinding::Native("review".into())),
                default: true,
                ..Default::default()
            },
            path: "/user/ayran.toml".into(),
        },
    );
    layers
}

fn skill_state() -> ayran_core::skills::SkillState {
    ayran_core::skills::SkillState {
        personal: ["review".into()].into(),
        ..Default::default()
    }
}

#[test]
fn skill_precedence_combines_defaults_explicit_origins_and_disables() {
    #[derive(Debug, Default)]
    struct Case {
        cli_selects: bool,
        alias_selects: bool,
        config_disable: bool,
        alias_disable: bool,
        cli_disable: bool,
        no_defaults: bool,
        alias_no_defaults: bool,
    }
    for (case, enabled) in [
        (Case::default(), true),
        (
            Case {
                config_disable: true,
                ..Case::default()
            },
            false,
        ),
        (
            Case {
                alias_disable: true,
                ..Case::default()
            },
            false,
        ),
        (
            Case {
                cli_selects: true,
                config_disable: true,
                alias_disable: true,
                ..Case::default()
            },
            true,
        ),
        (
            Case {
                alias_selects: true,
                config_disable: true,
                alias_disable: true,
                ..Case::default()
            },
            true,
        ),
        (
            Case {
                cli_selects: true,
                alias_selects: true,
                config_disable: true,
                alias_disable: true,
                cli_disable: true,
                ..Case::default()
            },
            false,
        ),
        (
            Case {
                no_defaults: true,
                ..Case::default()
            },
            false,
        ),
        (
            Case {
                cli_selects: true,
                no_defaults: true,
                ..Case::default()
            },
            true,
        ),
        (
            Case {
                alias_no_defaults: true,
                ..Case::default()
            },
            false,
        ),
        (
            Case {
                alias_selects: true,
                config_disable: true,
                alias_disable: true,
                no_defaults: true,
                alias_no_defaults: true,
                ..Case::default()
            },
            true,
        ),
    ] {
        let mut layers = skill_layers();
        let names = |selected| {
            if selected {
                vec!["review".into()]
            } else {
                vec![]
            }
        };
        if case.config_disable {
            layers
                .disabled_skills
                .insert("review".into(), "/project/ayran.toml".into());
        }
        layers.aliases.insert(
            "work".into(),
            ayran_core::config::Alias {
                args: vec![],
                harness_args: true,
                harness: Harness::Claude,
                model: None,
                effort: None,
                description: None,
                skills: names(case.alias_selects),
                plugins: vec![],
                profiles: vec![],
                defaults: !case.alias_no_defaults,
                disabled_profiles: vec![],
                mcp: vec![],
                disabled_mcp: vec![],
                disabled_skills: names(case.alias_disable),
                disabled_plugins: vec![],
            },
        );
        let mut request = request();
        request.alias = Some("work".into());
        request.skills = names(case.cli_selects);
        request.no_skills = names(case.cli_disable);
        request.no_defaults = case.no_defaults;
        let plan = resolve(
            request,
            &layers,
            &installed(),
            &skill_state(),
            &Default::default(),
        )
        .unwrap_or_else(|_| panic!("resolution failed"));
        let settings: serde_json::Value =
            serde_json::from_str(plan.args[1].to_str().unwrap()).unwrap();
        assert_eq!(
            settings["skillOverrides"]["review"],
            if enabled { "on" } else { "off" },
            "{case:?}"
        );
    }
}

#[test]
fn skill_profile_routes_obey_disables_and_preserve_the_strongest_origin() {
    use ayran_core::config::Profile;
    for (default_profile, explicit_profile, direct, config_disable, cli_disable, enabled, origin) in [
        (
            true,
            false,
            false,
            false,
            false,
            true,
            "Default (via Profile team)",
        ),
        (true, true, false, false, false, true, "via Profile base"),
        (false, true, false, true, false, false, "disabled by layer"),
        (true, true, true, true, false, true, "explicit (--skill)"),
        (
            false,
            true,
            true,
            false,
            true,
            false,
            "disabled by --no-skill",
        ),
    ] {
        let mut config = skill_layers();
        config.skills.get_mut("review").unwrap().value.default = false;
        config.profiles.insert(
            "base".into(),
            Sourced {
                value: Profile {
                    skills: vec!["review".into()],
                    ..Default::default()
                },
                path: "/user/ayran.toml".into(),
            },
        );
        config.profiles.insert(
            "team".into(),
            Sourced {
                value: Profile {
                    profiles: vec!["base".into()],
                    default: default_profile,
                    ..Default::default()
                },
                path: "/project/ayran.toml".into(),
            },
        );
        if config_disable {
            config
                .disabled_skills
                .insert("review".into(), "/project/ayran.toml".into());
        }
        let mut req = request();
        if explicit_profile {
            req.profiles.push("team".into());
        }
        if direct {
            req.skills.push("review".into());
        }
        if cli_disable {
            req.no_skills.push("review".into());
        }
        let plan = resolve(
            req,
            &config,
            &Default::default(),
            &skill_state(),
            &Default::default(),
        )
        .unwrap_or_else(|_| panic!("resolution failed"));
        let settings: serde_json::Value =
            serde_json::from_str(plan.args[1].to_str().unwrap()).unwrap();
        assert_eq!(
            settings["skillOverrides"]["review"],
            if enabled { "on" } else { "off" }
        );
        assert!(
            plan.trace.skills.iter().any(|line| line.contains(origin)),
            "{:?}",
            plan.trace.skills
        );
    }
}

#[test]
fn skill_profile_dangling_members_follow_default_and_explicit_origins() {
    use ayran_core::config::Profile;
    for (explicit, disabled) in [(false, false), (true, false), (true, true)] {
        let mut config = ConfigLayers::default();
        config.profiles.insert(
            "team".into(),
            Sourced {
                value: Profile {
                    skills: vec!["missing".into()],
                    default: true,
                    ..Default::default()
                },
                path: "/project/ayran.toml".into(),
            },
        );
        let mut req = request();
        if explicit {
            req.profiles.push("team".into());
        }
        if disabled {
            req.no_profiles.push("team".into());
        }
        match resolve(
            req,
            &config,
            &Default::default(),
            &Default::default(),
            &Default::default(),
        ) {
            Err(errors) => {
                assert!(explicit && !disabled);
                assert_eq!(errors[0].code, "dangling-ref");
            }
            Ok(plan) => {
                let notes: Vec<_> = plan
                    .diagnostics
                    .iter()
                    .filter(|note| note.code == "dangling-ref")
                    .collect();
                assert_eq!(notes.len(), if disabled { 0 } else { 1 });
                if let Some(note) = notes.first() {
                    assert!(matches!(
                        note.severity,
                        ayran_core::diagnostic::Severity::Note
                    ));
                }
            }
        }
    }
}

#[test]
fn skill_false_bindings_are_skipped_only_for_indirect_selection() {
    use ayran_core::config::{Profile, SkillBinding};
    for route in ["default", "profile", "explicit"] {
        let mut config = skill_layers();
        config.skills.get_mut("review").unwrap().value.all = Some(SkillBinding::Absent);
        config.profiles.insert(
            "team".into(),
            Sourced {
                value: Profile {
                    skills: vec!["review".into()],
                    ..Default::default()
                },
                path: "/project/ayran.toml".into(),
            },
        );
        let mut req = request();
        if route == "profile" {
            req.profiles.push("team".into());
        }
        if route == "explicit" {
            req.skills.push("review".into());
        }
        match resolve(
            req,
            &config,
            &Default::default(),
            &Default::default(),
            &Default::default(),
        ) {
            Err(errors) => {
                assert_eq!(route, "explicit");
                assert_eq!(errors[0].code, "binding-absent");
            }
            Ok(plan) => {
                assert_ne!(route, "explicit");
                assert_eq!(
                    plan.diagnostics
                        .iter()
                        .filter(|note| note.code == "binding-skipped")
                        .count(),
                    1
                );
                assert!(
                    plan.trace
                        .skills
                        .iter()
                        .any(|line| line.contains("false Binding"))
                );
            }
        }
    }
}

#[test]
fn disabling_one_logical_skill_keeps_a_shared_native_item_selected_by_another() {
    use ayran_core::config::{Skill, SkillBinding};
    let mut config = skill_layers();
    config.skills.insert(
        "testing".into(),
        Sourced {
            value: Skill {
                all: Some(SkillBinding::Native("review".into())),
                ..Default::default()
            },
            path: "/project/ayran.toml".into(),
        },
    );
    let mut req = request();
    req.skills = vec!["testing".into()];
    req.no_skills = vec!["review".into()];
    let plan = resolve(
        req,
        &config,
        &Default::default(),
        &skill_state(),
        &Default::default(),
    )
    .unwrap_or_else(|_| panic!("resolution failed"));
    let settings: serde_json::Value = serde_json::from_str(plan.args[1].to_str().unwrap()).unwrap();
    assert_eq!(settings["skillOverrides"]["review"], "on");
}

#[test]
fn disabled_unknown_explicit_skills_are_still_errors() {
    let mut req = request();
    req.skills = vec!["missing".into()];
    req.no_skills = vec!["missing".into()];
    let errors = resolve(
        req,
        &ConfigLayers::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .err()
    .unwrap();
    assert_eq!(errors[0].code, "unknown-skill");
}

#[test]
fn skill_trace_keeps_distinct_logical_names_with_a_shared_prefix() {
    use ayran_core::config::{Skill, SkillBinding};
    let mut config = ConfigLayers::default();
    for name in ["x", "x:other"] {
        config.skills.insert(
            name.into(),
            Sourced {
                value: Skill {
                    all: Some(SkillBinding::Native("review".into())),
                    ..Default::default()
                },
                path: "/project/ayran.toml".into(),
            },
        );
    }
    let mut req = request();
    req.skills = vec!["x:other".into()];
    req.no_skills = vec!["x:other".into()];
    let plan = resolve(
        req,
        &config,
        &Default::default(),
        &skill_state(),
        &Default::default(),
    )
    .unwrap_or_else(|_| panic!("resolution failed"));
    assert!(
        plan.trace
            .skills
            .iter()
            .any(|line| line == "Skill x: unselected"),
        "{:?}",
        plan.trace.skills
    );
}

#[test]
fn mcp_default_generates_a_definition_on_every_harness() {
    use ayran_core::mcp::{McpBinding, McpDefinition, McpServer};
    let mut layers = ConfigLayers::default();
    layers.mcp.insert(
        "files".into(),
        Sourced {
            value: McpServer {
                default: true,
                all: Some(McpBinding::Definition(McpDefinition::Stdio {
                    command: "fixture".into(),
                    args: vec![],
                    env: Default::default(),
                    env_vars: vec![],
                })),
                ..Default::default()
            },
            path: "/user/ayran.toml".into(),
        },
    );
    for harness in [Harness::Claude, Harness::Codex, Harness::Copilot] {
        let mut request = request();
        request.harness = Some(harness);
        let plan = resolve(
            request,
            &layers,
            &Default::default(),
            &Default::default(),
            &Default::default(),
        )
        .unwrap_or_else(|_| panic!("resolution failed"));
        if harness == Harness::Codex {
            assert!(
                plan.mcp
                    .overrides
                    .iter()
                    .any(|line| line.starts_with("mcp_servers.files="))
            );
        } else {
            assert_eq!(
                plan.mcp.config.as_ref().unwrap()["mcpServers"]["files"]["command"],
                "fixture"
            );
        }
        assert!(plan.mcp.trace.iter().any(|line| line.contains("Default")));
    }
}
#[test]
fn mcp_precedence_combines_defaults_explicit_origins_and_disables() {
    #[derive(Debug, Default)]
    struct Case {
        cli_selects: bool,
        alias_selects: bool,
        config_disable: bool,
        alias_disable: bool,
        cli_disable: bool,
        no_defaults: bool,
        alias_no_defaults: bool,
    }
    for (case, enabled) in [
        (Case::default(), true),
        (
            Case {
                config_disable: true,
                ..Case::default()
            },
            false,
        ),
        (
            Case {
                alias_disable: true,
                ..Case::default()
            },
            false,
        ),
        (
            Case {
                cli_selects: true,
                config_disable: true,
                alias_disable: true,
                ..Case::default()
            },
            true,
        ),
        (
            Case {
                alias_selects: true,
                config_disable: true,
                alias_disable: true,
                ..Case::default()
            },
            true,
        ),
        (
            Case {
                cli_selects: true,
                alias_selects: true,
                config_disable: true,
                alias_disable: true,
                cli_disable: true,
                ..Case::default()
            },
            false,
        ),
        (
            Case {
                no_defaults: true,
                ..Case::default()
            },
            false,
        ),
        (
            Case {
                cli_selects: true,
                no_defaults: true,
                ..Case::default()
            },
            true,
        ),
        (
            Case {
                alias_no_defaults: true,
                ..Case::default()
            },
            false,
        ),
        (
            Case {
                alias_selects: true,
                config_disable: true,
                alias_disable: true,
                no_defaults: true,
                alias_no_defaults: true,
                ..Case::default()
            },
            true,
        ),
    ] {
        let mut layers = ConfigLayers::default();
        layers.mcp.insert(
            "review".into(),
            Sourced {
                value: ayran_core::mcp::McpServer {
                    all: Some(ayran_core::mcp::McpBinding::Native("native".into())),
                    default: true,
                    ..Default::default()
                },
                path: "/user/ayran.toml".into(),
            },
        );
        let names = |selected| {
            if selected {
                vec!["review".into()]
            } else {
                vec![]
            }
        };
        if case.config_disable {
            layers
                .disabled_mcp
                .insert("review".into(), "/project/ayran.toml".into());
        }
        layers.aliases.insert(
            "work".into(),
            ayran_core::config::Alias {
                args: vec![],
                harness_args: true,
                harness: Harness::Claude,
                model: None,
                effort: None,
                description: None,
                mcp: names(case.alias_selects),
                plugins: vec![],
                profiles: vec![],
                defaults: !case.alias_no_defaults,
                disabled_profiles: vec![],
                skills: vec![],
                disabled_skills: vec![],
                disabled_mcp: names(case.alias_disable),
                disabled_plugins: vec![],
            },
        );
        let mut request = request();
        request.alias = Some("work".into());
        request.mcp = names(case.cli_selects);
        request.no_mcp = names(case.cli_disable);
        request.no_defaults = case.no_defaults;
        let plan = resolve(
            request,
            &layers,
            &installed(),
            &Default::default(),
            &ayran_core::mcp::McpState {
                user: ["native".into()].into(),
                ..Default::default()
            },
        )
        .unwrap_or_else(|_| panic!("resolution failed"));
        assert_eq!(
            plan.mcp.denied.contains(&"native".into()),
            !enabled,
            "{case:?}"
        );
    }
}
#[test]
fn mcp_profile_routes_obey_disables_and_preserve_the_strongest_origin() {
    use ayran_core::config::Profile;
    for (default_profile, explicit_profile, direct, config_disable, cli_disable, enabled, origin) in [
        (
            true,
            false,
            false,
            false,
            false,
            true,
            "Default (via Profile team)",
        ),
        (true, true, false, false, false, true, "via Profile base"),
        (false, true, false, true, false, false, "disabled by layer"),
        (true, true, true, true, false, true, "explicit (--mcp)"),
        (
            false,
            true,
            true,
            false,
            true,
            false,
            "disabled by --no-mcp",
        ),
    ] {
        let mut config = ConfigLayers::default();
        config.mcp.insert(
            "review".into(),
            Sourced {
                value: ayran_core::mcp::McpServer {
                    all: Some(ayran_core::mcp::McpBinding::Native("native".into())),
                    ..Default::default()
                },
                path: "/user/ayran.toml".into(),
            },
        );
        config.profiles.insert(
            "base".into(),
            Sourced {
                value: Profile {
                    mcp: vec!["review".into()],
                    ..Default::default()
                },
                path: "/user/ayran.toml".into(),
            },
        );
        config.profiles.insert(
            "team".into(),
            Sourced {
                value: Profile {
                    profiles: vec!["base".into()],
                    default: default_profile,
                    ..Default::default()
                },
                path: "/project/ayran.toml".into(),
            },
        );
        if config_disable {
            config
                .disabled_mcp
                .insert("review".into(), "/project/ayran.toml".into());
        }
        let mut req = request();
        if explicit_profile {
            req.profiles.push("team".into());
        }
        if direct {
            req.mcp.push("review".into());
        }
        if cli_disable {
            req.no_mcp.push("review".into());
        }
        let plan = resolve(
            req,
            &config,
            &Default::default(),
            &Default::default(),
            &ayran_core::mcp::McpState {
                user: ["native".into()].into(),
                ..Default::default()
            },
        )
        .unwrap_or_else(|_| panic!("resolution failed"));
        assert_eq!(plan.mcp.denied.contains(&"native".into()), !enabled);
        assert!(
            plan.mcp.trace.iter().any(|line| line.contains(origin)),
            "{:?}",
            plan.mcp.trace
        );
    }
}
