use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;

struct Workspace(TempDir);

#[cfg(unix)]
#[test]
fn aliases_without_path_executables_and_ayrans_own_binary_do_not_warn() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let w = Workspace::new();
    w.config("[aliases.missing]\nharness='claude'\n[aliases.plain]\nharness='claude'\n[aliases.folder]\nharness='claude'\n[aliases.self_link]\nharness='claude'\n");
    fs::write(w.0.path().join("plain"), "not executable").unwrap();
    fs::set_permissions(w.0.path().join("plain"), fs::Permissions::from_mode(0o644)).unwrap();
    fs::create_dir(w.0.path().join("folder")).unwrap();
    symlink(env!("CARGO_BIN_EXE_ayran"), w.0.path().join("self_link")).unwrap();
    let output = w.run(&["doctor", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["summary"],
        serde_json::json!({"errors":0,"warnings":0,"notes":3})
    );
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["code"] == "harness-not-found")
    );
}

#[cfg(unix)]
#[test]
fn alias_shadowing_reports_the_executable_path_in_the_config_group() {
    use std::os::unix::fs::PermissionsExt;
    let w = Workspace::new();
    let executable = w.0.path().join("ls");
    fs::write(&executable, "#!/bin/sh\nexit 99\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    w.config("[aliases.ls]\nharness='claude'\n");
    let output = w.run(&["doctor", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["diagnostics"][0],
        serde_json::json!({
            "code":"alias-shadows-command", "severity":"warning",
            "message":format!("Alias ls shadows PATH executable {}", executable.display()),
            "item":executable, "harness":null, "capability":null, "cause":null, "layer":null, "hint":null
        })
    );
    let output = w.run(&["doctor", "--codex"]);
    assert!(
        String::from_utf8_lossy(&output.stdout).starts_with(&format!(
            "config\nwarning[alias-shadows-command]: Alias ls shadows PATH executable {}\n",
            executable.display()
        ))
    );
    let output = w.run(&["doctor", "-q"]);
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(!text.contains("alias-shadows-command"));
    assert!(text.ends_with("0 errors, 1 warning, 3 notes\n"));
}

#[test]
fn reserved_alias_names_are_config_errors_collected_together() {
    let w = Workspace::new();
    w.config("[aliases.echo]\nharness='claude'\n[aliases.ayran]\nharness='codex'\n[aliases.IF]\nharness='copilot'\n");
    let output = w.run(&["doctor", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let diagnostics: Vec<_> = json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "alias-reserved-name")
        .collect();
    assert_eq!(diagnostics.len(), 3, "{json}");
    for diagnostic in diagnostics {
        assert_eq!(diagnostic["severity"], "error");
        for field in ["harness", "capability", "item", "cause"] {
            assert_eq!(diagnostic[field], serde_json::Value::Null);
        }
    }
    let output = w.run(&["doctor", "--codex", "-q"]);
    assert!(
        String::from_utf8_lossy(&output.stdout).starts_with("config\nerror[alias-reserved-name]:")
    );
    assert_eq!(output.status.code(), Some(1));
}
impl Workspace {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }
    fn config(&self, text: &str) {
        fs::write(self.0.path().join("user.toml"), text).unwrap();
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ayran"))
            .current_dir(self.0.path())
            .env("HOME", self.0.path())
            .env("AYRAN_CONFIG", self.0.path().join("user.toml"))
            .env("PATH", self.0.path())
            .env("XDG_CACHE_HOME", self.0.path().join("cache"))
            .env("XDG_STATE_HOME", self.0.path().join("state"))
            .env("XDG_CONFIG_HOME", self.0.path().join("config"))
            .env("CLAUDE_CONFIG_DIR", self.0.path().join(".claude"))
            .env("CODEX_HOME", self.0.path().join(".codex"))
            .env("COPILOT_HOME", self.0.path().join(".copilot"))
            .args(args)
            .output()
            .unwrap()
    }
}
#[test]
fn absent_harnesses_print_home_headings_and_exact_json() {
    let w = Workspace::new();
    let output = w.run(&["doctor"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "config\nclaude (home: {0}/.claude)\nnote[harness-not-found]: Harness claude was not found on PATH\ncodex (home: {0}/.codex)\nnote[harness-not-found]: Harness codex was not found on PATH\ncopilot (home: {0}/.copilot)\nnote[harness-not-found]: Harness copilot was not found on PATH\n0 errors, 0 warnings, 3 notes\n",
            w.0.path().display()
        )
    );
    let output = w.run(&["doctor", "--json"]);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!({"version":1,"diagnostics":(["claude", "codex", "copilot"].map(|h| serde_json::json!({
            "code":"harness-not-found", "severity":"note", "message":format!("Harness {h} was not found on PATH"),
            "harness":h,"capability":null,"item":null,"cause":null,"layer":null,"hint":null
        }))),"summary":{"errors":0,"warnings":0,"notes":3}})
    );
}
#[test]
fn default_and_alias_gaps_are_graded_and_quiet_keeps_counts() {
    let w = Workspace::new();
    w.config("[skills.tdd]\ndefault=true\nclaude=false\ncopilot=false\n");
    let output = w.run(&["doctor", "--codex", "-q"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "config\ncodex (home: {}/.codex)\nerror[harness-not-found]: Harness codex was not found on PATH\n1 error, 1 warning, 0 notes\n",
            w.0.path().display()
        )
    );
    let output = w.run(&["doctor", "--json", "--codex"]);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let json = serde_json::json!({"diagnostics": report["diagnostics"].as_array().unwrap().iter()
        .filter(|d| d["code"] == "missing-binding").collect::<Vec<_>>()});
    assert_eq!(json["diagnostics"][0]["harness"], "codex");
    assert_eq!(
        json["diagnostics"][0]["capability"],
        serde_json::json!({"kind":"skill","name":"tdd"})
    );
    assert_eq!(
        json["diagnostics"][0]["hint"],
        "write `codex = false` if this gap is deliberate"
    );
    w.config("[skills.tdd]\nclaude=false\ncopilot=false\n[aliases.work]\nharness='codex'\nskills=['tdd']\n");
    let output = w.run(&["doctor", "--codex", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["summary"],
        serde_json::json!({"errors":2,"warnings":0,"notes":0})
    );
}
#[test]
fn independent_config_errors_are_collected_but_invalid_layers_stop_checks() {
    let w = Workspace::new();
    w.config("[skills.'bad,name']\nall=false\n[profiles.a]\nprofiles=['a']\n[profiles.b]\nskills=['undefined']\n[mcp.web]\nall={url='${URL}'}\n");
    fs::write(
        w.0.path().join("ayran.toml"),
        "[aliases.project]\nharness='codex'\n[harnesses.codex]\nhome='isolated'\n",
    )
    .unwrap();
    let output = w.run(&["doctor", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let codes: Vec<_> = json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    for code in [
        "invalid-name",
        "profile-cycle",
        "dangling-ref",
        "literal-interpolation",
        "user-level-only",
    ] {
        assert!(codes.contains(&code), "{json}");
    }
    assert_eq!(codes.iter().filter(|c| **c == "user-level-only").count(), 2);
    fs::write(w.0.path().join("ayran.local.toml"), "unknown=true").unwrap();
    let output = w.run(&["doctor", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["diagnostics"].as_array().unwrap().len(), 4);
    assert_eq!(json["diagnostics"][0]["code"], "config-invalid");
}
#[cfg(unix)]
#[test]
fn installed_defaults_and_default_profile_members_are_reachable() {
    use std::os::unix::fs::PermissionsExt;
    let w = Workspace::new();
    fs::write(w.0.path().join("codex"), "#!/bin/sh\necho codex 9.0.0\n").unwrap();
    fs::set_permissions(w.0.path().join("codex"), fs::Permissions::from_mode(0o755)).unwrap();
    w.config("[skills.tdd]\nclaude=false\ncopilot=false\n[profiles.team]\ndefault=true\nskills=['tdd']\n");
    let output = w.run(&["doctor", "--codex", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["diagnostics"][0]["code"], "missing-binding");
    assert_eq!(json["diagnostics"][0]["severity"], "error");
}
#[test]
fn paths_unsupported_bindings_and_skill_clashes_are_all_reported() {
    let w = Workspace::new();
    fs::create_dir(w.0.path().join("one")).unwrap();
    fs::create_dir(w.0.path().join("two")).unwrap();
    for name in ["one", "two"] {
        fs::write(
            w.0.path().join(name).join("SKILL.md"),
            "---\nname: review\n---\n",
        )
        .unwrap();
    }
    w.config("[skills.one]\nall={path='one'}\n[skills.two]\nall={path='two'}\n[plugins.missing]\nall={path='missing'}\n[mcp.command]\nall={command='./missing-command'}\n");
    let output = w.run(&["doctor", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let diagnostics = json["diagnostics"].as_array().unwrap();
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d["code"] == "unsupported-binding")
            .count(),
        3
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d["code"] == "skill-name-clash")
            .count(),
        2
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d["code"] == "path-not-found")
            .count(),
        6
    );
    assert_eq!(output.status.code(), Some(0));
}
#[test]
fn doctor_usage_and_completion_follow_the_command_grammar() {
    let w = Workspace::new();
    for args in [
        vec!["doctor", "--codex", "--claude"],
        vec!["doctor", "--codex", "--codex"],
        vec!["doctor", "--harness=codex", "--codex"],
        vec!["doctor", "--harness", "codex", "--harness", "codex"],
        vec!["doctor", "--skill", "tdd"],
    ] {
        assert_eq!(w.run(&args).status.code(), Some(2), "{args:?}");
    }
    let output = w.run(&["__complete", "--", "ayran", "doc"]);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "doctor\n");
    let output = w.run(&["__complete", "--", "ayran", "doctor", "--harness=co"]);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "--harness=codex\n--harness=copilot\n"
    );
}
#[test]
fn skills_selected_only_by_different_aliases_have_a_latent_clash() {
    let w = Workspace::new();
    for name in ["one", "two"] {
        fs::create_dir(w.0.path().join(name)).unwrap();
        fs::write(
            w.0.path().join(name).join("SKILL.md"),
            "---\nname: review\n---\n",
        )
        .unwrap();
    }
    w.config("[skills.one]\nall={path='one'}\n[skills.two]\nall={path='two'}\n[aliases.first]\nharness='claude'\ndefaults=false\nskills=['one']\n[aliases.second]\nharness='claude'\ndefaults=false\nskills=['two']\n");
    let output = w.run(&["doctor", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["diagnostics"][0]["code"], "skill-name-clash");
    assert_eq!(json["diagnostics"][0]["severity"], "warning");
}

#[test]
fn missing_harness_is_a_note_unless_explicitly_requested() {
    let w = Workspace::new();
    for (args, severity, status) in [
        (vec!["doctor", "--json"], "note", 0),
        (vec!["doctor", "--copilot", "--json"], "error", 1),
    ] {
        let output = w.run(&args);
        assert_eq!(output.status.code(), Some(status));
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let d = json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["harness"] == "copilot")
            .unwrap();
        assert_eq!(d["code"], "harness-not-found");
        assert_eq!(d["severity"], severity);
    }
}

#[cfg(unix)]
impl Workspace {
    fn harness(&self, name: &str, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = self.0.path().join(name);
        fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
#[cfg(unix)]
#[test]
fn doctor_probes_fresh_and_refreshes_even_a_too_old_cached_version() {
    let w = Workspace::new();
    w.harness(
        "codex",
        "if [ -f old ]; then echo 'codex 0.1.0'; else echo 'codex 9.0.0'; fi",
    );
    assert_eq!(w.run(&["doctor", "--codex"]).status.code(), Some(0));
    let cache = w.0.path().join("cache/ayran/versions/codex.json");
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
    assert_eq!(value["version"], "9.0.0");
    fs::write(w.0.path().join("old"), "").unwrap();
    let output = w.run(&["doctor", "--codex", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["diagnostics"][0]["code"], "harness-too-old");
    assert_eq!(json["diagnostics"][0]["harness"], "codex");
    let value: serde_json::Value = serde_json::from_slice(&fs::read(cache).unwrap()).unwrap();
    assert_eq!(value["version"], "0.1.0");
}

#[cfg(unix)]
#[test]
fn missing_isolated_home_is_empty_and_is_not_created() {
    let w = Workspace::new();
    w.harness("codex", "echo 'codex 9.0.0'");
    w.config("[harnesses.codex]\nhome='isolated'\n");
    let output = w.run(&["doctor", "--codex"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let path = w.0.path().join("state/ayran/homes/codex");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "config\ncodex (isolated home, not created yet: {})\nnote[leak]: codex-remote-plugin: Codex account/workspace remote Plugins cannot be enumerated or hidden from local Harness state; unselected remote Plugins may remain visible\n0 errors, 0 warnings, 1 note\n",
            path.display()
        )
    );
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn unreadable_state_is_reported_even_with_invalid_config() {
    let w = Workspace::new();
    w.harness("codex", "echo 'codex 9.0.0'");
    fs::create_dir(w.0.path().join(".codex")).unwrap();
    fs::write(w.0.path().join(".codex/config.toml"), "[").unwrap();
    for config in ["", "unknown=true"] {
        w.config(config);
        let output = w.run(&["doctor", "--codex", "--json"]);
        assert_eq!(output.status.code(), Some(1));
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            json["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["code"] == "enumeration-failed" && d["harness"] == "codex"),
            "{json}"
        );
    }
}

#[cfg(unix)]
#[test]
fn every_native_binding_is_checked_and_graded_by_reachability() {
    let w = Workspace::new();
    w.harness("codex", "echo 'codex 0.1.0'");
    w.config("[plugins.review]\ndefault=true\ncodex='missing@market'\n[skills.tdd]\ncodex='missing-skill'\n[mcp.files]\ndefault=true\ncodex='missing-server'\n");
    let output = w.run(&["doctor", "--codex", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let gaps: Vec<_> = json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "native-not-found")
        .collect();
    assert_eq!(gaps.len(), 3, "{json}");
    for (name, severity, item) in [
        ("review", "error", "missing@market"),
        ("tdd", "warning", "missing-skill"),
        ("files", "error", "missing-server"),
    ] {
        let d = gaps
            .iter()
            .find(|d| d["capability"]["name"] == name)
            .unwrap();
        assert_eq!(d["severity"], severity);
        assert_eq!(d["item"], item);
    }
}

#[cfg(unix)]
#[test]
fn codex_definition_collisions_include_project_servers_and_reachability() {
    let w = Workspace::new();
    w.harness("codex", "echo 'codex 9.0.0'");
    fs::create_dir(w.0.path().join(".codex")).unwrap();
    fs::write(
        w.0.path().join(".codex/config.toml"),
        "[mcp_servers.files]\ncommand='files'\n",
    )
    .unwrap();
    for (default, severity, status) in [(false, "warning", 0), (true, "error", 1)] {
        w.config(&format!(
            "[mcp.files]\ndefault={default}\ncodex={{command='files'}}\n"
        ));
        let output = w.run(&["doctor", "--codex", "--json"]);
        assert_eq!(output.status.code(), Some(status));
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["diagnostics"][0]["code"], "mcp-definition-collision");
        assert_eq!(json["diagnostics"][0]["severity"], severity);
        assert_eq!(json["diagnostics"][0]["item"], "files");
    }
}

#[cfg(unix)]
#[test]
fn isolated_copilot_native_plugins_use_the_same_shared_bindings_as_launch() {
    let w = Workspace::new();
    w.harness("copilot", "echo 'copilot 9.0.0'");
    fs::create_dir_all(w.0.path().join(".copilot/installed-plugins/market/review")).unwrap();
    fs::write(
        w.0.path()
            .join(".copilot/installed-plugins/market/review/plugin.json"),
        r#"{"name":"review"}"#,
    )
    .unwrap();
    w.config("[harnesses.copilot]\nhome='isolated'\n[plugins.review]\ndefault=true\ncopilot='review@market'\n");
    for created in [false, true] {
        if created {
            fs::create_dir_all(w.0.path().join("state/ayran/homes/copilot")).unwrap();
        }
        let output = w.run(&["doctor", "--copilot", "--json"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["diagnostics"], serde_json::json!([]));
    }
}

#[cfg(unix)]
#[test]
fn worst_case_copilot_personal_skills_ignore_defaults_and_use_launch_home() {
    let w = Workspace::new();
    w.harness("copilot", "echo 'copilot 9.0.0'");
    for name in ["review", "tdd"] {
        let root = w.0.path().join(".copilot/skills").join(name);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("SKILL.md"), format!("---\nname: {name}\n---\n")).unwrap();
    }
    w.config("[skills.review]\ndefault=true\nall='review'\n");
    let output = w.run(&["doctor", "--copilot", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["diagnostics"],
        serde_json::json!([{
            "code":"leak", "severity":"warning", "harness":"copilot",
            "cause":"copilot-personal-skill", "item":"review, tdd",
            "message":"copilot-personal-skill: unselected personal Skills remain visible: review, tdd",
            "capability":null,"hint":null,"layer":null
        }])
    );
    let output = w.run(&["doctor", "--copilot"]);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "config\ncopilot (home: {}/.copilot)\nwarning[leak]: copilot-personal-skill: unselected personal Skills remain visible: review, tdd\n0 errors, 1 warning, 0 notes\n",
            w.0.path().display()
        )
    );
    w.config("unknown=true");
    let output = w.run(&["doctor", "--copilot", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["summary"],
        serde_json::json!({"errors":1,"warnings":1,"notes":0})
    );
    w.config("[harnesses.copilot]\nhome='isolated'\n");
    let output = w.run(&["doctor", "--copilot", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["diagnostics"], serde_json::json!([]));
}

#[cfg(unix)]
#[test]
fn doctor_groups_claude_shadows_and_inventory_notes_keep_launch_severity() {
    let w = Workspace::new();
    w.harness("claude", "echo 'claude 9.0.0'");
    for name in ["review", "tdd"] {
        for scope in [".claude/skills", "project/.claude/skills"] {
            let root = w.0.path().join(scope).join(name);
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join("SKILL.md"), format!("---\nname: {name}\n---\n")).unwrap();
        }
    }
    // Put the project Skills under a distinct working directory from the personal home.
    fs::write(
        w.0.path().join(".claude/.claude.json"),
        r#"{"mcpServers":{"files":{},"search":{}}}"#,
    )
    .unwrap();
    fs::write(
        w.0.path().join("project/.mcp.json"),
        r#"{"mcpServers":{"files":{},"search":{}}}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ayran"))
        .current_dir(w.0.path().join("project"))
        .env("HOME", w.0.path())
        .env("PATH", w.0.path())
        .env("AYRAN_CONFIG", w.0.path().join("user.toml"))
        .env("CLAUDE_CONFIG_DIR", w.0.path().join(".claude"))
        .env("XDG_CACHE_HOME", w.0.path().join("cache"))
        .env("XDG_CONFIG_HOME", w.0.path().join("config"))
        .args(["doctor", "--claude", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let leaks = json["diagnostics"].as_array().unwrap();
    assert_eq!(leaks.len(), 3, "{json}");
    for (cause, item, severity) in [
        ("claude-project-shadow", "review, tdd", "warning"),
        ("mcp-project-shadow", "files, search", "warning"),
        ("claude-synced-plugin", "", "note"),
    ] {
        let d = leaks.iter().find(|d| d["cause"] == cause).unwrap();
        assert_eq!(d["severity"], severity);
        if !item.is_empty() {
            assert_eq!(d["item"], item);
        }
    }
    for (harness, cause) in [
        ("claude", "claude-synced-plugin"),
        ("codex", "codex-remote-plugin"),
    ] {
        w.harness(harness, &format!("echo '{harness} 9.0.0'"));
        let output = w.run(&["doctor", &format!("--{harness}"), "--json"]);
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            json["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["cause"] == cause && d["severity"] == "note")
        );
        let output = w.run(&[
            &format!("--{harness}"),
            "--no-defaults",
            "--dry-run",
            "--json",
        ]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            json["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["cause"] == cause && d["severity"] == "warning")
        );
    }
}

#[cfg(unix)]
#[test]
fn doctor_checks_servers_of_every_installed_plugin_including_direct_installs() {
    let w = Workspace::new();
    for harness in ["codex", "copilot"] {
        w.harness(harness, &format!("echo '{harness} 9.0.0'"));
        let home = w.0.path().join(format!(".{harness}"));
        fs::create_dir_all(&home).unwrap();
        let root = if harness == "codex" {
            fs::write(
                home.join("config.toml"),
                "[plugins.'review@market']\nenabled=false\n[mcp_servers.files]\ncommand='files'\n",
            )
            .unwrap();
            home.join("plugins/cache/market/review/1.0.0")
        } else {
            fs::write(
                home.join("mcp-config.json"),
                r#"{"mcpServers":{"files":{},"search":{}}}"#,
            )
            .unwrap();
            home.join("installed-plugins/market/review")
        };
        fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        fs::write(
            root.join(".claude-plugin/plugin.json"),
            r#"{"name":"review","mcpServers":{"files":{"command":"files"}}}"#,
        )
        .unwrap();
        if harness == "copilot" {
            let direct = home.join("installed-plugins/_direct/other/.claude-plugin");
            fs::create_dir_all(&direct).unwrap();
            fs::write(
                direct.join("plugin.json"),
                r#"{"name":"other","mcpServers":{"search":{"command":"search"}}}"#,
            )
            .unwrap();
        }
        let output = w.run(&["doctor", &format!("--{harness}"), "--json"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let shadows: Vec<_> = json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["cause"] == "plugin-server-shadow")
            .collect();
        assert_eq!(shadows.len(), 1, "{json}");
        assert_eq!(shadows[0]["severity"], "warning");
        assert_eq!(
            shadows[0]["item"],
            if harness == "codex" {
                "files"
            } else {
                "files, search"
            }
        );
        fs::write(root.join(".claude-plugin/plugin.json"), "{").unwrap();
        let output = w.run(&["doctor", &format!("--{harness}"), "--json"]);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            json["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["code"] == "enumeration-failed")
        );
    }
}

#[cfg(unix)]
#[test]
fn duplicate_bindings_compare_native_ids_and_canonical_directories_not_definitions() {
    use std::os::unix::fs::symlink;
    let w = Workspace::new();
    w.harness("copilot", "echo 'copilot 9.0.0'");
    let plugin = w.0.path().join("plugin");
    let skill = w.0.path().join("skill");
    fs::create_dir_all(&plugin).unwrap();
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "---\nname: tdd\n---\n").unwrap();
    symlink(&plugin, w.0.path().join("plugin-link")).unwrap();
    symlink(&skill, w.0.path().join("skill-link")).unwrap();
    w.config("[plugins.a]\nall='review@market'\n[plugins.b]\nall='review@market'\n[plugins.c]\nall={path='plugin'}\n[plugins.d]\nall={path='plugin-link'}\n[skills.a]\nall='review@market'\n[skills.b]\nall='review@market'\n[skills.c]\nall={path='skill'}\n[skills.d]\nall={path='skill-link'}\n[mcp.a]\nall='review@market'\n[mcp.b]\nall='review@market'\n[mcp.c]\nall={command='files'}\n[mcp.d]\nall={command='files'}\n");
    let output = w.run(&["doctor", "--copilot", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let duplicates: Vec<_> = json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "duplicate-binding")
        .collect();
    assert_eq!(duplicates.len(), 5, "{json}");
    for d in &duplicates {
        assert_eq!(d["severity"], "note");
        assert_eq!(d["harness"], "copilot");
    }
    for kind in ["plugin", "skill", "mcp"] {
        assert!(
            duplicates.iter().any(|d| d["capability"]["kind"] == kind
                && d["item"] == "review@market"
                && d["message"].as_str().unwrap().contains("a and b")),
            "{json}"
        );
    }
    for (kind, target) in [("plugin", plugin), ("skill", skill)] {
        assert!(
            duplicates.iter().any(|d| d["capability"]["kind"] == kind
                && d["item"] == target.display().to_string()
                && d["message"].as_str().unwrap().contains("c and d")),
            "{json}"
        );
    }
}

#[cfg(unix)]
#[test]
fn plugin_mcp_clashes_use_codex_ids_and_skills_keep_their_namespaces() {
    let w = Workspace::new();
    w.harness("codex", "echo codex 9.0.0");
    let home = w.0.path().join(".codex");
    fs::create_dir_all(&home).unwrap();
    fs::write(
        home.join("config.toml"),
        "[plugins.'alpha@market']\nenabled=false\n[plugins.'zeta@market']\nenabled=true\n",
    )
    .unwrap();
    for (id, namespace) in [("alpha", "zulu"), ("zeta", "able")] {
        let root = home.join(format!("plugins/cache/market/{id}/1.0.0"));
        fs::create_dir_all(root.join(".codex-plugin")).unwrap();
        fs::create_dir_all(root.join("skills/folder")).unwrap();
        fs::write(root.join(".codex-plugin/plugin.json"), serde_json::json!({"name":namespace,"mcpServers":{"shared":{"command":"/bin/false","args":[id]}}}).to_string()).unwrap();
        fs::write(
            root.join("skills/folder/SKILL.md"),
            "---\nname: review\ndescription: Review\n---\n",
        )
        .unwrap();
    }
    let output = w.run(&["doctor", "--codex", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let clashes: Vec<_> = json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "plugin-item-clash")
        .collect();
    assert_eq!(clashes.len(), 1, "{json}");
    assert_eq!(clashes[0]["item"], "shared");
    assert_eq!(clashes[0]["severity"], "warning");
    assert_eq!(
        clashes[0]["message"],
        "Plugins alpha@market and zeta@market compete for MCP server shared; Codex keeps alpha@market (alphabetically first Plugin ID)"
    );
}

#[cfg(unix)]
#[test]
fn claude_plugin_dedup_compares_args_and_url_queries_and_respects_disabled_manual_servers() {
    let w = Workspace::new();
    fs::create_dir(w.0.path().join(".git")).unwrap();
    w.harness("claude", "echo claude 9.0.0");
    let root = w.0.path().join(".claude/skills/alpha");
    fs::create_dir_all(root.join(".claude-plugin")).unwrap();
    fs::write(
        root.join(".claude-plugin/plugin.json"),
        serde_json::json!({"name":"alpha","mcpServers":{
            "duplicate":{"command":"/bin/false","args":["same"],"env":{"X":"plugin"}},
            "different":{"command":"/bin/false","args":["different"]},
            "remote":{"url":"https://example.com/mcp/?q=1#fragment"},
            "query":{"url":"https://example.com/mcp?q=2"},
            "disabled":{"command":"/bin/false","args":["disabled"]},
            "dynamic":{"command":"${UNKNOWN}","args":["same"]}
        }})
        .to_string(),
    )
    .unwrap();
    fs::write(
        w.0.path().join(".claude/.claude.json"),
        serde_json::json!({"mcpServers":{
        "manual":{"command":"/bin/false","args":["same"],"env":{"X":"manual"}},
        "remote":{"url":"https://example.com/mcp?q=1"},
        "off":{"command":"/bin/false","args":["disabled"]}
    },"projects":{w.0.path().display().to_string():{"disabledMcpServers":["off"]}}})
        .to_string(),
    )
    .unwrap();
    let output = w.run(&["doctor", "--claude", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let dedup: Vec<_> = json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "plugin-server-dedup")
        .collect();
    assert_eq!(dedup.len(), 2, "{json}");
    assert_eq!(dedup[0]["item"], "plugin:alpha:duplicate");
    assert_eq!(dedup[1]["item"], "plugin:alpha:remote");
    assert_eq!(dedup[0]["severity"], "note");
    assert!(
        dedup[0]["message"]
            .as_str()
            .unwrap()
            .contains("keeps manual")
    );
    w.config("[mcp.definition]\nclaude={command='/bin/false',args=['different']}\n");
    let output = w.run(&["doctor", "--claude", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "plugin-server-dedup"
                && d["item"] == "plugin:alpha:different"
                && d["message"].as_str().unwrap().contains("definition")),
        "{json}"
    );
}

#[cfg(unix)]
#[test]
fn copilot_content_checks_describe_scan_order_and_project_skill_shadowing() {
    let w = Workspace::new();
    fs::create_dir(w.0.path().join(".git")).unwrap();
    w.harness("copilot", "echo copilot 9.0.0");
    for id in ["alpha", "zeta"] {
        let root =
            w.0.path()
                .join(format!(".copilot/installed-plugins/market/{id}"));
        fs::create_dir_all(root.join("custom/folder")).unwrap();
        fs::write(root.join("plugin.json"), serde_json::json!({"name":id,"skills":"./custom","mcpServers":{"shared":{"command":"/bin/false","args":[id]}}}).to_string()).unwrap();
        fs::write(
            root.join("custom/folder/SKILL.md"),
            "---\nname: review\ndescription: Review\n---\n",
        )
        .unwrap();
    }
    let path_skill = w.0.path().join("standalone");
    let project_skill = w.0.path().join(".github/skills/review");
    for root in [&path_skill, &project_skill] {
        fs::create_dir_all(root).unwrap();
        fs::write(
            root.join("SKILL.md"),
            "---\nname: review\ndescription: Review\n---\n",
        )
        .unwrap();
    }
    w.config("[skills.review]\nclaude=false\ncodex=false\ncopilot={path='standalone'}\n");
    let output = w.run(&["doctor", "--copilot", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let diagnostics = json["diagnostics"].as_array().unwrap();
    let clashes: Vec<_> = diagnostics
        .iter()
        .filter(|d| d["code"] == "plugin-item-clash")
        .collect();
    assert_eq!(clashes.len(), 1, "{json}");
    assert!(
        clashes[0]["message"]
            .as_str()
            .unwrap()
            .contains("loading order")
    );
    let shadows: Vec<_> = diagnostics
        .iter()
        .filter(|d| d["code"] == "skill-shadowed")
        .collect();
    assert_eq!(shadows.len(), 1, "{json}");
    assert_eq!(
        shadows[0]["capability"],
        serde_json::json!({"kind":"skill","name":"review"})
    );
    assert_eq!(shadows[0]["severity"], "warning");
    fs::remove_dir_all(project_skill).unwrap();
    let output = w.run(&["doctor", "--copilot", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        !json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "skill-shadowed"),
        "{json}"
    );
}

#[cfg(unix)]
#[test]
fn claude_registry_custom_components_and_exact_skill_names_are_audited() {
    let w = Workspace::new();
    fs::create_dir(w.0.path().join(".git")).unwrap();
    w.harness("claude", "echo claude 9.0.0");
    let root = w.0.path().join("installed-version");
    fs::create_dir_all(root.join(".claude-plugin")).unwrap();
    fs::create_dir_all(root.join("custom/folder")).unwrap();
    fs::write(
        root.join("custom/folder/SKILL.md"),
        "---\nname: display\ndescription: Review\n---\n",
    )
    .unwrap();
    fs::write(root.join(".claude-plugin/plugin.json"), serde_json::json!({"name":"alpha","skills":"./custom","mcpServers":[{"same":{"command":"/bin/false","args":["old"]}},"./servers.json"]}).to_string()).unwrap();
    fs::write(
        root.join("servers.json"),
        serde_json::json!({"mcpServers":{"same":{"command":"/bin/false","args":["new"]}}})
            .to_string(),
    )
    .unwrap();
    let home = w.0.path().join(".claude");
    fs::create_dir_all(home.join("plugins")).unwrap();
    fs::write(home.join("plugins/installed_plugins.json"),serde_json::json!({"version":2,"plugins":{"directory@market":[{"scope":"user","installPath":root}]}}).to_string()).unwrap();
    fs::write(
        home.join(".claude.json"),
        serde_json::json!({"mcpServers":{"manual":{"command":"/bin/false","args":["new"]}}})
            .to_string(),
    )
    .unwrap();
    let standalone = w.0.path().join("standalone");
    fs::create_dir_all(&standalone).unwrap();
    fs::write(
        standalone.join("SKILL.md"),
        "---\nname: alpha:folder\ndescription: Standalone\n---\n",
    )
    .unwrap();
    w.config("[skills.standalone]\nclaude={path='standalone'}\ncodex=false\ncopilot=false\n");
    let output = w.run(&["doctor", "--claude", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(0), "{json}");
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "skill-shadowed" && d["item"] == "alpha:folder"),
        "{json}"
    );
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "plugin-server-dedup" && d["item"] == "plugin:alpha:same"),
        "{json}"
    );
    // A coexisting suffix or display alias is not a lost canonical name.
    fs::write(
        standalone.join("SKILL.md"),
        "---\nname: display\ndescription: Standalone\n---\n",
    )
    .unwrap();
    let output = w.run(&["doctor", "--claude", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        !json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "skill-shadowed"),
        "{json}"
    );
}

#[cfg(unix)]
#[test]
fn malformed_plugin_payloads_fail_doctor_without_adding_launch_checks() {
    let w = Workspace::new();
    w.harness("codex", "echo codex 9.0.0");
    let root = w.0.path().join(".codex/plugins/cache/market/alpha/1.0.0");
    fs::create_dir_all(root.join(".codex-plugin")).unwrap();
    fs::write(
        w.0.path().join(".codex/config.toml"),
        "[plugins.'alpha@market']\nenabled=false\n",
    )
    .unwrap();
    fs::write(
        root.join(".codex-plugin/plugin.json"),
        "{\"name\":\"alpha\",\"skills\":42}",
    )
    .unwrap();
    let output = w.run(&["doctor", "--codex", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(1), "{json}");
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "enumeration-failed" && d["harness"] == "codex"),
        "{json}"
    );
    let output = w.run(&["--codex", "--dry-run", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        !json["diagnostics"].as_array().unwrap().iter().any(|d| [
            "plugin-item-clash",
            "plugin-server-dedup",
            "enumeration-failed"
        ]
        .iter()
        .any(|code| d["code"] == *code)),
        "{json}"
    );
    fs::write(root.join(".codex-plugin/plugin.json"), "{").unwrap();
    let output = w.run(&["doctor", "--codex", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
}

#[cfg(unix)]
#[test]
fn claude_marketplace_overrides_and_approved_project_servers_are_inventoried() {
    let w = Workspace::new();
    fs::create_dir(w.0.path().join(".git")).unwrap();
    w.harness("claude", "echo claude 9.0.0");
    let root = w.0.path().join("plugin");
    let marketplace = w.0.path().join("marketplace");
    fs::create_dir_all(root.join(".claude-plugin")).unwrap();
    fs::create_dir_all(root.join("extra/review")).unwrap();
    fs::create_dir_all(marketplace.join(".claude-plugin")).unwrap();
    fs::write(
        root.join(".claude-plugin/plugin.json"),
        r#"{"name":"alpha"}"#,
    )
    .unwrap();
    fs::write(
        root.join("extra/review/SKILL.md"),
        "---\ndescription: Review\n---\n",
    )
    .unwrap();
    fs::write(marketplace.join(".claude-plugin/marketplace.json"),serde_json::json!({"name":"market","plugins":[{"name":"alpha","source":"./plugin","skills":"./extra","mcpServers":{"shared":{"command":"/bin/false","args":["project"]}}}]}).to_string()).unwrap();
    let home = w.0.path().join(".claude");
    fs::create_dir_all(home.join("plugins")).unwrap();
    fs::write(home.join("plugins/installed_plugins.json"),serde_json::json!({"version":2,"plugins":{"alpha@market":[{"scope":"user","installPath":root}]}}).to_string()).unwrap();
    fs::write(
        home.join("plugins/known_marketplaces.json"),
        serde_json::json!({"market":{"installLocation":marketplace}}).to_string(),
    )
    .unwrap();
    fs::write(
        w.0.path().join(".mcp.json"),
        r#"{"mcpServers":{"project":{"command":"/bin/false","args":["project"]}}}"#,
    )
    .unwrap();
    let skill = w.0.path().join("standalone");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: alpha:review\ndescription: Review\n---\n",
    )
    .unwrap();
    w.config("[skills.standalone]\nclaude={path='standalone'}\ncodex=false\ncopilot=false\n");
    let report = |w: &Workspace| {
        let output = w.run(&["doctor", "--claude", "--json"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let json = report(&w);
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "skill-shadowed"),
        "{json}"
    );
    assert!(
        !json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "plugin-server-dedup"),
        "{json}"
    );
    fs::write(home.join("settings.local.json"), "{}").unwrap();
    fs::write(
        home.join("settings.json"),
        r#"{"enabledMcpjsonServers":["project"]}"#,
    )
    .unwrap();
    let json = report(&w);
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "plugin-server-dedup"
                && d["message"].as_str().unwrap().contains("keeps project")),
        "{json}"
    );
    fs::write(
        home.join("settings.json"),
        r#"{"enabledMcpjsonServers":["project"],"deniedMcpServers":[{"serverName":"project"}]}"#,
    )
    .unwrap();
    let json = report(&w);
    assert!(
        !json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "plugin-server-dedup"),
        "{json}"
    );
}

#[cfg(unix)]
#[test]
fn claude_deduplicates_symlinked_skill_files_across_plugins() {
    use std::os::unix::fs::symlink;
    let w = Workspace::new();
    fs::create_dir(w.0.path().join(".git")).unwrap();
    w.harness("claude", "echo claude 9.0.0");
    let common = w.0.path().join("common");
    fs::create_dir_all(&common).unwrap();
    fs::write(common.join("SKILL.md"), "---\ndescription: Shared\n---\n").unwrap();
    for id in ["alpha", "zeta"] {
        let root = w.0.path().join(format!(".claude/skills/{id}"));
        fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        fs::create_dir_all(root.join("skills")).unwrap();
        fs::write(
            root.join(".claude-plugin/plugin.json"),
            r#"{"name":"same"}"#,
        )
        .unwrap();
        symlink(&common, root.join("skills/review")).unwrap();
    }
    let output = w.run(&["doctor", "--claude", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(0), "{json}");
    assert!(
        !json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "plugin-item-clash"),
        "{json}"
    );
}

#[cfg(unix)]
#[test]
fn copilot_native_personal_skill_shadowed_by_project_is_reported() {
    let w = Workspace::new();
    fs::create_dir(w.0.path().join(".git")).unwrap();
    w.harness("copilot", "echo copilot 9.0.0");
    for relative in [".copilot/skills/review", ".github/skills/review"] {
        let root = w.0.path().join(relative);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("SKILL.md"),
            "---\nname: review\ndescription: Review\n---\n",
        )
        .unwrap();
    }
    w.config("[skills.review]\nclaude=false\ncodex=false\ncopilot='review'\n");
    let output = w.run(&["doctor", "--copilot", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(0), "{json}");
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "skill-shadowed"
                && d["severity"] == "warning"
                && d["capability"]["name"] == "review"),
        "{json}"
    );
    fs::remove_dir_all(w.0.path().join(".copilot/skills/review")).unwrap();
    let output = w.run(&["doctor", "--copilot", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        !json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "skill-shadowed"),
        "{json}"
    );
}

#[cfg(unix)]
#[test]
fn claude_connector_cache_misses_are_notes_and_declared_connectors_warn_of_leaks() {
    let w = Workspace::new();
    w.harness("claude", "echo 9.0.0");
    w.config("[mcp.linear]\nclaude={connector='claude.ai Linear'}\ncodex=false\ncopilot=false\n");
    fs::create_dir_all(w.0.path().join(".claude")).unwrap();
    for present in [false, true] {
        fs::write(
            w.0.path().join(".claude/.claude.json"),
            if present {
                r#"{"claudeAiMcpEverConnected":["claude.ai Linear"]}"#
            } else {
                "{}"
            },
        )
        .unwrap();
        let output = w.run(&["doctor", "--claude", "--json"]);
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(output.status.code(), Some(0), "{json}");
        assert_eq!(json["summary"]["warnings"], 0, "{json}");
        let diagnostics = json["diagnostics"].as_array().unwrap();
        let unseen: Vec<_> = diagnostics
            .iter()
            .filter(|d| d["code"] == "connector-not-seen")
            .collect();
        assert_eq!(unseen.len(), usize::from(!present), "{json}");
        if let Some(d) = unseen.first() {
            assert_eq!(d["severity"], "note");
            assert_eq!(d["harness"], "claude");
            assert_eq!(d["capability"]["name"], "linear");
            assert_eq!(d["item"], "claude.ai Linear");
            assert!(d["message"].as_str().unwrap().contains("never"));
            assert!(d["message"].as_str().unwrap().contains("misspelt"));
        }
        assert!(
            diagnostics.iter().any(|d| d["code"] == "leak"
                && d["cause"] == "claude-connector"
                && d["severity"] == "note"),
            "{json}"
        );
        assert!(
            !diagnostics.iter().any(|d| d["code"] == "native-not-found"),
            "{json}"
        );
    }
    let output = w.run(&["doctor", "--claude"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("note[leak]"));
    w.config("");
    let output = w.run(&["doctor", "--claude", "--json"]);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("claude-connector"));
}

#[cfg(unix)]
#[test]
fn codex_connector_notes_use_tools_from_every_cache_file() {
    let w = Workspace::new();
    w.harness("codex", "echo codex 9.0.0");
    w.config("[mcp.linear]\ncodex={connector='connector_linear'}\nclaude=false\ncopilot=false\n");
    let cache = w.0.path().join(".codex/cache/codex_apps_tools");
    for present in [false, true] {
        if present {
            fs::create_dir_all(&cache).unwrap();
            fs::write(
                cache.join("first.json"),
                r#"{"schema_version":1,"tools":[{"connector_id":"connector_other"}]}"#,
            )
            .unwrap();
            fs::write(cache.join("second.json"), r#"{"schema_version":1,"tools":[{"connector_id":"connector_linear"},{"connector_id":"connector_linear"}]}"#).unwrap();
        }
        let output = w.run(&["doctor", "--codex", "--json"]);
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(output.status.code(), Some(0), "{json}");
        let diagnostics = json["diagnostics"].as_array().unwrap();
        let unseen: Vec<_> = diagnostics
            .iter()
            .filter(|d| d["code"] == "connector-not-seen")
            .collect();
        assert_eq!(unseen.len(), usize::from(!present), "{json}");
        if let Some(d) = unseen.first() {
            assert_eq!(d["severity"], "note");
            assert_eq!(d["harness"], "codex");
            assert_eq!(d["item"], "connector_linear");
        }
        assert!(
            !diagnostics
                .iter()
                .any(|d| d["code"] == "native-not-found" || d["cause"] == "claude-connector"),
            "{json}"
        );
    }
}

#[cfg(unix)]
#[test]
fn unsupported_connector_bindings_are_graded_by_reachability() {
    let w = Workspace::new();
    for harness in ["claude", "codex", "copilot"] {
        w.harness(harness, "echo 9.0.0");
    }
    for default in [false, true] {
        w.config(&format!("[mcp.shared]\ndefault={default}\nall={{connector='connector_x'}}\n[mcp.copilot_only]\nclaude=false\ncodex=false\ncopilot={{connector='connector_y'}}\n"));
        let output = w.run(&["doctor", "--json"]);
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(output.status.code(), Some(i32::from(default)), "{json}");
        let gaps: Vec<_> = json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["code"] == "unsupported-binding")
            .collect();
        assert_eq!(gaps.len(), 4, "{json}");
        for harness in ["claude", "codex", "copilot"] {
            assert!(
                gaps.iter().any(|d| d["harness"] == harness
                    && d["capability"]["name"] == "shared"
                    && d["severity"] == if default { "error" } else { "warning" }),
                "{json}"
            );
        }
        assert!(
            gaps.iter()
                .any(|d| d["capability"]["name"] == "copilot_only" && d["severity"] == "warning"),
            "{json}"
        );
        assert!(
            !json["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["code"] == "connector-not-seen"),
            "{json}"
        );
    }
}

#[cfg(unix)]
#[test]
fn equal_connector_ids_are_duplicate_bindings_but_native_ids_are_distinct() {
    let w = Workspace::new();
    w.harness("claude", "echo 9.0.0");
    w.config("[mcp.first]\nclaude={connector='claude.ai Linear'}\ncodex=false\ncopilot=false\n[mcp.second]\nclaude={connector='claude.ai Linear'}\ncodex=false\ncopilot=false\n[mcp.native]\nclaude='claude.ai Linear'\ncodex=false\ncopilot=false\n");
    fs::create_dir_all(w.0.path().join(".claude")).unwrap();
    fs::write(
        w.0.path().join(".claude/.claude.json"),
        r#"{"claudeAiMcpEverConnected":["claude.ai Linear"],"mcpServers":{"claude.ai Linear":{}}}"#,
    )
    .unwrap();
    let output = w.run(&["doctor", "--claude", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(0), "{json}");
    let duplicates: Vec<_> = json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "duplicate-binding")
        .collect();
    assert_eq!(duplicates.len(), 1, "{json}");
    assert_eq!(duplicates[0]["severity"], "note");
    assert_eq!(duplicates[0]["item"], "claude.ai Linear");
    assert_eq!(duplicates[0]["capability"]["name"], "second");
}

#[cfg(unix)]
#[test]
fn connector_state_checks_skip_absent_harnesses_and_use_isolated_homes() {
    let w = Workspace::new();
    w.config("[mcp.linear]\nclaude={connector='claude.ai Linear'}\ncodex={connector='connector_linear'}\ncopilot=false\n");
    let output = w.run(&["doctor", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        !json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "connector-not-seen" || d["cause"] == "claude-connector"),
        "{json}"
    );
    for harness in ["claude", "codex"] {
        w.harness(harness, "echo 9.0.0");
    }
    fs::create_dir_all(w.0.path().join(".claude")).unwrap();
    fs::write(
        w.0.path().join(".claude/.claude.json"),
        r#"{"claudeAiMcpEverConnected":["claude.ai Linear"]}"#,
    )
    .unwrap();
    let cache = w.0.path().join(".codex/cache/codex_apps_tools");
    fs::create_dir_all(&cache).unwrap();
    fs::write(
        cache.join("shared.json"),
        r#"{"tools":[{"connector_id":"connector_linear"}]}"#,
    )
    .unwrap();
    w.config("[harnesses.claude]\nhome='isolated'\n[harnesses.codex]\nhome='isolated'\n[mcp.linear]\nclaude={connector='claude.ai Linear'}\ncodex={connector='connector_linear'}\ncopilot=false\n");
    let output = w.run(&["doctor", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(0), "{json}");
    let unseen: Vec<_> = json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "connector-not-seen")
        .collect();
    assert_eq!(unseen.len(), 2, "{json}");
    let homes = w.0.path().join("state/ayran/homes");
    assert!(!homes.exists(), "doctor must not create an isolated home");
}

#[cfg(unix)]
#[test]
fn unreadable_codex_connector_cache_reports_enumeration_failure_without_false_misses() {
    let w = Workspace::new();
    w.harness("codex", "echo 9.0.0");
    w.config("[mcp.linear]\ncodex={connector='connector_linear'}\nclaude=false\ncopilot=false\n");
    let cache = w.0.path().join(".codex/cache/codex_apps_tools");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("broken.json"), "{").unwrap();
    let output = w.run(&["doctor", "--codex", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(1), "{json}");
    let diagnostics = json["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|d| d["code"] == "enumeration-failed" && d["harness"] == "codex"),
        "{json}"
    );
    assert!(
        !diagnostics
            .iter()
            .any(|d| d["code"] == "connector-not-seen"),
        "{json}"
    );
}

#[cfg(unix)]
#[test]
fn codex_connector_cache_is_not_read_without_a_connector_binding() {
    let w = Workspace::new();
    w.harness("codex", "echo 9.0.0");
    w.config("[mcp.missing]\ncodex='missing'\nclaude=false\ncopilot=false\n");
    let cache = w.0.path().join(".codex/cache/codex_apps_tools");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("broken.json"), "{").unwrap();
    let output = w.run(&["doctor", "--codex", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(0), "{json}");
    let diagnostics = json["diagnostics"].as_array().unwrap();
    assert!(
        !diagnostics
            .iter()
            .any(|d| d["code"] == "enumeration-failed"),
        "{json}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d["code"] == "native-not-found" && d["capability"]["name"] == "missing"),
        "{json}"
    );
}

#[cfg(unix)]
#[test]
fn copilot_custom_skill_doctor_reports_the_worst_case_in_the_session_home() {
    let w = Workspace::new();
    w.harness("copilot", "echo 'copilot 9.0.0'");
    for name in ["review", "tdd"] {
        let root = w.0.path().join("custom-skills").join(name);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("SKILL.md"), format!("---\nname: {name}\n---\n")).unwrap();
    }
    fs::create_dir_all(w.0.path().join(".copilot")).unwrap();
    let settings = r#"{"skillDirectories":["custom-skills", "missing"]}"#;
    fs::write(w.0.path().join(".copilot/settings.json"), settings).unwrap();
    w.config("[skills.review]\ndefault=true\ncopilot='review'\n");
    let output = w.run(&["doctor", "--copilot", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["diagnostics"],
        serde_json::json!([{
            "code":"leak", "severity":"warning", "harness":"copilot",
            "cause":"copilot-custom-skill-dir", "item":"review, tdd",
            "message":"copilot-custom-skill-dir: unselected custom-dir Skills remain visible: review, tdd",
            "capability":null,"hint":null,"layer":null
        }])
    );
    let output = w.run(&["doctor", "--copilot"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "config\ncopilot (home: {}/.copilot)\nwarning[leak]: copilot-custom-skill-dir: unselected custom-dir Skills remain visible: review, tdd\n0 errors, 1 warning, 0 notes\n",
            w.0.path().display()
        )
    );
    w.config("[harnesses.copilot]\nhome='isolated'\n");
    let output = w.run(&["doctor", "--copilot", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["diagnostics"], serde_json::json!([]));
    let isolated = w.0.path().join("state/ayran/homes/copilot");
    fs::create_dir_all(&isolated).unwrap();
    fs::write(isolated.join("settings.json"), settings).unwrap();
    let output = w.run(&["doctor", "--copilot", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["diagnostics"][0]["item"], "review, tdd");
    assert_eq!(json["diagnostics"][0]["cause"], "copilot-custom-skill-dir");
    fs::write(
        isolated.join("settings.json"),
        r#"{"skillDirectories":[false]}"#,
    )
    .unwrap();
    let output = w.run(&["doctor", "--copilot", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["diagnostics"][0]["code"], "enumeration-failed");
}
