use std::fs;
use std::process::{Command, Output};
use tempfile::TempDir;

struct Workspace(TempDir);

#[test]
fn skill_list_filters_bindings_and_profiles_include_direct_skill_members() {
    let workspace = Workspace::new();
    let user = workspace.0.path().join("user.toml");
    fs::write(&user, "[skills.tdd]\nall='tdd'\ncodex=false\ndefault=true\n[profiles.team]\nskills=['tdd', 'undefined']\n").unwrap();
    for harness in ["claude", "codex", "copilot"] {
        let output = workspace.run(&["list", "skills", "--harness", harness, "--json"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["plugins"], serde_json::json!([]));
        assert_eq!(json["profiles"], serde_json::json!([]));
        let binding = if harness == "codex" {
            serde_json::json!({"kind":"absent"})
        } else {
            serde_json::json!({"kind":"native", "id":"tdd"})
        };
        assert_eq!(
            json["skills"][1]["bindings"],
            serde_json::json!({harness: binding})
        );
        let output = workspace.run(&["list", "skills", &format!("--{harness}")]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .starts_with(&format!("name\tdefault\tlayer\tdescription\t{harness}\n"))
        );
    }
    let output = workspace.run(&["list", "profiles", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["profiles"][0]["skills"],
        serde_json::json!(["tdd", "undefined"])
    );
    let output = workspace.run(&["list"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("skills: tdd, undefined"));
}

impl Workspace {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }

    fn with_backslash_path() -> Self {
        // Exercise Windows path escaping on Unix too, where backslashes are legal names.
        #[cfg(unix)]
        return Self(
            tempfile::Builder::new()
                .prefix("ayran\\list-")
                .tempdir()
                .unwrap(),
        );
        #[cfg(windows)]
        Self::new()
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ayran"))
            .current_dir(self.0.path())
            .env("AYRAN_CONFIG", self.0.path().join("user.toml"))
            .env("HOME", self.0.path())
            .env("USERPROFILE", self.0.path())
            .env("PATH", "")
            .args(args)
            .output()
            .unwrap()
    }
}

#[test]
fn profiles_show_merged_definitions_and_direct_members_without_a_harness() {
    let workspace = Workspace::with_backslash_path();
    let user = workspace.0.path().join("user.toml");
    let local = workspace.0.path().join("ayran.local.toml");
    fs::write(&user, "[plugins.review]\nall='review@m'\n[profiles.base]\nplugins=['inner']\ndefault=true\n[profiles.team]\nplugins=['old']\ndefault=true\n[profiles.empty]\n").unwrap();
    fs::write(&local, "[profiles.team]\nplugins=['review', 'build']\nprofiles=['base']\ndescription=\"Team\\ttools\\nnow\"\n").unwrap();
    let output = workspace.run(&["list", "profiles"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "name\tdefault\tlayer\tdescription\tmembers\nbase\t*\t{}\t\tplugins: inner\nempty\t\t{}\t\t\nteam\t\t{}\tTeam\\ttools\\nnow\tplugins: review, build · profiles: base\n",
            user.to_string_lossy().replace('\\', "\\\\"),
            user.to_string_lossy().replace('\\', "\\\\"),
            local.to_string_lossy().replace('\\', "\\\\")
        )
    );
    for harness in ["claude", "codex", "copilot"] {
        assert_eq!(
            output.stdout,
            workspace
                .run(&["list", "profiles", "--harness", harness])
                .stdout
        );
        assert_eq!(
            output.stdout,
            workspace
                .run(&["list", "profiles", &format!("--{harness}")])
                .stdout
        );
    }
    let all = workspace.run(&["list"]);
    assert_eq!(all.status.code(), Some(0), "{all:?}");
    assert!(String::from_utf8_lossy(&all.stdout).contains("review@m"));
    assert!(
        String::from_utf8_lossy(&all.stdout)
            .contains(String::from_utf8_lossy(&output.stdout).as_ref())
    );
    let plugins = workspace.run(&["list", "plugins"]);
    assert_eq!(plugins.status.code(), Some(0), "{plugins:?}");
    assert!(!String::from_utf8_lossy(&plugins.stdout).contains("members"));
}

#[test]
fn json_profiles_are_direct_members_and_kind_filters_select_rows() {
    let workspace = Workspace::new();
    let user = workspace.0.path().join("user.toml");
    fs::write(&user, "[plugins.review]\nall='review@m'\n[profiles.base]\nplugins=['undefined']\n[profiles.empty]\n[profiles.team]\nplugins=['review', 'build']\nprofiles=['base']\ndefault=true\ndescription=\"Team\\ttools\\nnow\"\n[disable]\nprofiles=['team']\n").unwrap();
    let output = workspace.run(&["list", "profiles", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "version":1, "aliases":[], "sessions":[], "marketplaces":[], "presets":[], "plugins":[], "skills":[], "mcp":[], "diagnostics":[],
            "summary":{"errors":0,"warnings":0,"notes":0},
            "profiles":[
                {"name":"base", "default":false, "layer":user, "description":null,
                 "plugins":["undefined"], "skills":[], "mcp":[], "profiles":[]},
                {"name":"empty", "default":false, "layer":user, "description":null,
                 "plugins":[], "skills":[], "mcp":[], "profiles":[]},
                {"name":"team", "default":true, "layer":user, "description":"Team\ttools\nnow",
                 "plugins":["review", "build"], "skills":[], "mcp":[], "profiles":["base"]}
            ]
        })
    );
    assert_eq!(
        output.stdout,
        workspace
            .run(&["list", "profiles", "--json", "--codex"])
            .stdout
    );
    let all = workspace.run(&["list", "--json"]);
    assert_eq!(all.status.code(), Some(0), "{all:?}");
    let all: serde_json::Value = serde_json::from_slice(&all.stdout).unwrap();
    assert_eq!(all["profiles"], json["profiles"]);
    assert_eq!(all["plugins"][0]["name"], "review");
    let plugins = workspace.run(&["list", "plugins", "--json"]);
    let plugins: serde_json::Value = serde_json::from_slice(&plugins.stdout).unwrap();
    assert_eq!(plugins["profiles"], serde_json::json!([]));
    assert_eq!(plugins["plugins"], all["plugins"]);
}

#[test]
fn text_keeps_one_row_per_plugin_when_descriptions_contain_line_breaks() {
    let workspace = Workspace::new();
    fs::write(
        workspace.0.path().join("user.toml"),
        "[plugins.review]\nclaude=\"review@m\"\ndescription=\"Review\\tcode\\ncarefully\\rnow\"\n",
    )
    .unwrap();
    let output = workspace.run(&["list", "plugins", "--claude"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 2);
    let columns: Vec<_> = text.lines().nth(1).unwrap().split('\t').collect();
    assert_eq!(columns.len(), 5);
    assert_eq!(columns[3], "Review\\tcode\\ncarefully\\rnow");
}

#[test]
fn json_reports_config_errors_in_the_same_envelope_and_empty_lists_succeed() {
    let workspace = Workspace::new();
    let output = workspace.run(&["list", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["plugins"], serde_json::json!([]));
    assert_eq!(json["skills"].as_array().unwrap().len(), 1);
    assert_eq!(json["skills"][0]["name"], "ayran");
    assert_eq!(json["profiles"], serde_json::json!([]));
    let profiles = workspace.run(&["list", "profiles", "--json"]);
    assert_ne!(output.stdout, profiles.stdout);
    assert_eq!(profiles.status.code(), Some(0), "{profiles:?}");
    fs::write(workspace.0.path().join("ayran.toml"), "broken = [").unwrap();
    let output = workspace.run(&["list", "plugins", "--json"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["plugins"], serde_json::json!([]));
    assert_eq!(json["skills"], serde_json::json!([]));
    assert_eq!(json["profiles"], serde_json::json!([]));
    let profiles = workspace.run(&["list", "profiles", "--json"]);
    assert_eq!(profiles.status.code(), Some(3), "{profiles:?}");
    assert_eq!(output.stdout, profiles.stdout);
    assert_eq!(
        json["summary"],
        serde_json::json!({"errors":1,"warnings":0,"notes":0})
    );
    assert_eq!(json["diagnostics"].as_array().unwrap().len(), 1);
    let diagnostic = &json["diagnostics"][0];
    assert_eq!(diagnostic["code"], "config-invalid");
    assert_eq!(diagnostic["severity"], "error");
    for field in ["harness", "capability", "item", "layer", "cause", "hint"] {
        assert!(diagnostic.get(field).is_some(), "{field}");
    }
    assert!(!diagnostic["message"].as_str().unwrap().is_empty());
    let output = workspace.run(&["list"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("error[config-invalid]"));
}

#[test]
fn json_has_a_versioned_envelope_and_lossless_effective_bindings() {
    let workspace = Workspace::new();
    let user = workspace.0.path().join("user.toml");
    let project = workspace.0.path().join("ayran.toml");
    let local = workspace.0.path().join("ayran.local.toml");
    fs::write(&user, "[plugins.review]\nall='old@m'\ndefault=true\ndescription='Old'\n[plugins.build]\nall='build@m'\ncodex=false\ndefault=true\n").unwrap();
    fs::write(&project, "[plugins.review]\nall='project@m'\n").unwrap();
    fs::write(&local, "[plugins.review]\nclaude=false\ncopilot={path='working-copy'}\ndescription=\"Review\\tcode\\ncarefully\"\n").unwrap();
    let output = workspace.run(&["list", "plugins", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "version": 1, "aliases": [], "sessions": [], "marketplaces": [], "presets": [], "diagnostics": [], "summary": {"errors":0,"warnings":0,"notes":0},
            "profiles": [], "skills": [], "mcp": [],
            "plugins": [
                {"name":"build", "default":true, "layer":user, "description":null, "bindings": {
                    "claude":{"kind":"native", "id":"build@m"}, "codex":{"kind":"absent"}, "copilot":{"kind":"native", "id":"build@m"}
                }},
                {"name":"review", "default":false, "layer":local, "description":"Review\tcode\ncarefully", "bindings": {
                    "claude":{"kind":"absent"}, "codex":null, "copilot":{"kind":"path", "path":workspace.0.path().join("working-copy")}
                }}
            ]
        })
    );
    let all: serde_json::Value =
        serde_json::from_slice(&workspace.run(&["list", "--json"]).stdout).unwrap();
    assert_eq!(json["plugins"], all["plugins"]);
    let output = workspace.run(&["list", "--json", "--codex"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["plugins"][0]["bindings"],
        serde_json::json!({"codex":{"kind":"absent"}})
    );
    assert_eq!(
        json["plugins"][1]["bindings"],
        serde_json::json!({"codex":null})
    );
}

#[test]
fn harness_filters_keep_every_plugin_but_only_the_requested_binding_column() {
    let workspace = Workspace::new();
    fs::write(workspace.0.path().join("user.toml"), "default_harness='claude'\n[plugins.review]\nall='review@m'\ncodex=false\n[plugins.build]\nclaude='build@m'\n").unwrap();
    for harness in ["claude", "codex", "copilot"] {
        let shorthand = format!("--{harness}");
        let output = workspace.run(&["list", "plugins", &shorthand]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            output.stdout,
            workspace
                .run(&["list", "plugins", "--harness", harness])
                .stdout
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            text.lines().next().unwrap(),
            format!("name\tdefault\tlayer\tdescription\t{harness}")
        );
        assert_eq!(text.lines().count(), 3);
        if harness == "codex" {
            assert!(text.lines().nth(1).unwrap().ends_with("\t✗"));
            assert!(text.lines().nth(2).unwrap().ends_with("\t—"));
        }
    }
    let unfiltered = workspace.run(&["list"]);
    assert!(
        String::from_utf8_lossy(&unfiltered.stdout)
            .lines()
            .next()
            .unwrap()
            .ends_with("claude\tcodex\tcopilot")
    );
    for args in [
        vec!["list", "--claude", "--codex"],
        vec!["list", "--codex", "--harness", "codex"],
        vec!["list", "--harness", "codex", "--harness", "claude"],
    ] {
        assert_eq!(workspace.run(&args).status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn list_shows_merged_plugins_and_distinguishes_missing_and_absent_bindings() {
    let workspace = Workspace::with_backslash_path();
    fs::write(
        workspace.0.path().join("user.toml"),
        "[plugins.review]\nall='old@m'\ndefault=true\ndescription='Old'\n",
    )
    .unwrap();
    let project = workspace.0.path().join("ayran.toml");
    fs::write(&project, "[plugins.review]\nclaude=false\ncopilot={path='working-copy'}\ndescription='Review code'\n[plugins.build]\nall='build@m'\ncodex=false\ndefault=true\n").unwrap();
    let output = workspace.run(&["list", "plugins"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "name\tdefault\tlayer\tdescription\tclaude\tcodex\tcopilot\nbuild\t*\t{}\t\tbuild@m\t—\tbuild@m\nreview\t\t{}\tReview code\t—\t✗\tpath\n",
            project.to_string_lossy().replace('\\', "\\\\"),
            project.to_string_lossy().replace('\\', "\\\\")
        )
    );
    assert!(
        workspace
            .run(&["list"])
            .stdout
            .starts_with(&workspace.run(&["list", "plugins"]).stdout)
    );
}

#[test]
fn list_shows_merged_skills_and_distinguishes_missing_and_absent_bindings() {
    let workspace = Workspace::with_backslash_path();
    fs::write(
        workspace.0.path().join("user.toml"),
        "[skills.review]\nall='old'\ndefault=true\ndescription='Old'\n",
    )
    .unwrap();
    let project = workspace.0.path().join("ayran.toml");
    fs::write(&project, "[skills.review]\nclaude=false\ncopilot={path='working-copy'}\ndescription='Review code'\n[skills.build]\nall='build'\ncodex=false\ndefault=true\n").unwrap();
    let output = workspace.run(&["list", "skills"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "name\tdefault\tlayer\tdescription\tclaude\tcodex\tcopilot\nayran\t\tbuilt-in\t\tbuiltin\tbuiltin\tbuiltin\nbuild\t*\t{}\t\tbuild\t—\tbuild\nreview\t\t{}\tReview code\t—\t✗\tpath\n",
            project.to_string_lossy().replace('\\', "\\\\"),
            project.to_string_lossy().replace('\\', "\\\\")
        )
    );
    assert!(
        String::from_utf8_lossy(&workspace.run(&["list"]).stdout)
            .contains(String::from_utf8_lossy(&workspace.run(&["list", "skills"]).stdout).as_ref())
    );
}

#[test]
fn skill_json_has_a_versioned_envelope_and_lossless_effective_bindings() {
    let workspace = Workspace::new();
    let user = workspace.0.path().join("user.toml");
    let project = workspace.0.path().join("ayran.toml");
    let local = workspace.0.path().join("ayran.local.toml");
    fs::write(&user, "[skills.review]\nall='old'\ndefault=true\ndescription='Old'\n[skills.build]\nall='build'\ncodex=false\ndefault=true\n").unwrap();
    fs::write(&project, "[skills.review]\nall='project'\n").unwrap();
    fs::write(&local, "[skills.review]\nclaude=false\ncopilot={path='working-copy'}\ndescription=\"Review\\tcode\\ncarefully\"\n").unwrap();
    let output = workspace.run(&["list", "skills", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "version": 1, "aliases": [], "sessions": [], "marketplaces": [], "presets": [], "diagnostics": [], "summary": {"errors":0,"warnings":0,"notes":0},
            "profiles": [], "plugins": [], "mcp": [],
            "skills": [
                {"name":"ayran", "default":false, "layer":"built-in", "description":null, "bindings": {
                    "claude":{"kind":"builtin", "name":"ayran"}, "codex":{"kind":"builtin", "name":"ayran"}, "copilot":{"kind":"builtin", "name":"ayran"}
                }},
                {"name":"build", "default":true, "layer":user, "description":null, "bindings": {
                    "claude":{"kind":"native", "id":"build"}, "codex":{"kind":"absent"}, "copilot":{"kind":"native", "id":"build"}
                }},
                {"name":"review", "default":false, "layer":local, "description":"Review\tcode\ncarefully", "bindings": {
                    "claude":{"kind":"absent"}, "codex":null, "copilot":{"kind":"path", "path":workspace.0.path().join("working-copy")}
                }}
            ]
        })
    );
    assert_eq!(output.stdout, workspace.run(&["list", "--json"]).stdout);
    let output = workspace.run(&["list", "--json", "--codex"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["skills"][1]["bindings"],
        serde_json::json!({"codex":{"kind":"absent"}})
    );
    assert_eq!(
        json["skills"][2]["bindings"],
        serde_json::json!({"codex":null})
    );
}

#[test]
fn mcp_list_shows_merged_bindings_without_definition_secrets() {
    let workspace = Workspace::new();
    fs::write(
        workspace.0.path().join("user.toml"),
        "[mcp.replaced]\nall='old'\ndefault=true\n",
    )
    .unwrap();
    let project = workspace.0.path().join("ayran.toml");
    fs::write(&project, r#"
[mcp.replaced]
claude = false
copilot = 'native-server'
description = 'Replaced'
[mcp.files]
all = { command = 'npx', args = ['secret-argument'], env = { TOKEN = 'secret-env' }, env_vars = ['FILES_TOKEN'] }
default = true
[mcp.web]
all = { url = 'https://secret-url.example', headers = { Authorization = 'secret-header' }, bearer_token_env = 'WEB_TOKEN' }
[profiles.team]
mcp = ['files', 'undefined']
"#).unwrap();
    let output = workspace.run(&["list", "mcp", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["plugins"], serde_json::json!([]));
    assert_eq!(json["skills"], serde_json::json!([]));
    assert_eq!(json["profiles"], serde_json::json!([]));
    assert_eq!(
        json["mcp"],
        serde_json::json!([
            {"name":"files", "default":true, "layer":project, "description":null, "bindings": {
                "claude":{"kind":"definition", "transport":"stdio"}, "codex":{"kind":"definition", "transport":"stdio"}, "copilot":{"kind":"definition", "transport":"stdio"}
            }},
            {"name":"replaced", "default":false, "layer":project, "description":"Replaced", "bindings": {
                "claude":{"kind":"absent"}, "codex":null, "copilot":{"kind":"native", "id":"native-server"}
            }},
            {"name":"web", "default":false, "layer":project, "description":null, "bindings": {
                "claude":{"kind":"definition", "transport":"http"}, "codex":{"kind":"definition", "transport":"http"}, "copilot":{"kind":"definition", "transport":"http"}
            }}
        ])
    );
    let text = workspace.run(&["list", "mcp"]);
    assert_eq!(text.status.code(), Some(0), "{text:?}");
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("definition (stdio)"));
    assert!(text.contains("definition (http)"));
    assert!(text.contains("Replaced\t—\t✗\tnative-server"));
    for args in [vec!["list"], vec!["list", "--json"]] {
        let output = workspace.run(&args);
        let all = String::from_utf8(output.stdout).unwrap();
        assert!(all.contains("files"));
        assert!(!all.contains("secret-"));
    }
    assert!(
        String::from_utf8_lossy(&workspace.run(&["list", "profiles"]).stdout)
            .contains("mcp: files, undefined")
    );
    let all: serde_json::Value =
        serde_json::from_slice(&workspace.run(&["list", "--json"]).stdout).unwrap();
    assert_eq!(all["mcp"], json["mcp"]);
    assert_eq!(
        all["profiles"][0]["mcp"],
        serde_json::json!(["files", "undefined"])
    );
    for harness in ["claude", "codex", "copilot"] {
        let output = workspace.run(&["list", "mcp", "--harness", harness, "--json"]);
        let filtered: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            filtered["mcp"][1]["bindings"],
            serde_json::json!({harness:json["mcp"][1]["bindings"][harness]})
        );
        let output = workspace.run(&["list", "mcp", &format!("--{harness}")]);
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .starts_with(&format!("name\tdefault\tlayer\tdescription\t{harness}\n"))
        );
    }
}

#[test]
fn aliases_show_sorted_escaped_rows_and_plain_list_includes_them() {
    let workspace = Workspace::new();
    fs::write(
        workspace.0.path().join("user.toml"),
        r#"
[aliases.zed]
harness = 'copilot'
[aliases.alpha]
harness = 'codex'
description = "Code\tcarefully\nnow\rhere\\end"
[aliases.middle]
harness = 'claude'
"#,
    )
    .unwrap();
    let output = workspace.run(&["list", "aliases"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "name\tharness\tdescription\nalpha\tcodex\tCode\\tcarefully\\nnow\\rhere\\\\end\nmiddle\tclaude\t\nzed\tcopilot\t\n"
    );
    let all = workspace.run(&["list"]);
    assert_eq!(all.status.code(), Some(0), "{all:?}");
    assert!(
        String::from_utf8_lossy(&all.stdout)
            .ends_with(String::from_utf8_lossy(&output.stdout).as_ref())
    );
    let plugins = workspace.run(&["list", "plugins"]);
    assert!(!String::from_utf8_lossy(&plugins.stdout).contains("name\tharness"));
}

#[test]
fn alias_json_rows_are_sorted_and_harness_filters_apply_to_both_formats() {
    let workspace = Workspace::new();
    fs::write(
        workspace.0.path().join("user.toml"),
        r#"
[aliases.zed]
harness = 'copilot'
[aliases.alpha]
harness = 'codex'
description = 'Code carefully'
[aliases.middle]
harness = 'claude'
"#,
    )
    .unwrap();
    let output = workspace.run(&["list", "aliases", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let expected = serde_json::json!([
        {"name":"alpha", "harness":"codex", "description":"Code carefully"},
        {"name":"middle", "harness":"claude", "description":null},
        {"name":"zed", "harness":"copilot", "description":null}
    ]);
    assert_eq!(
        json,
        serde_json::json!({
            "version":1, "plugins":[], "skills":[], "mcp":[], "profiles":[], "sessions":[], "marketplaces":[], "presets":[],
            "aliases":expected, "diagnostics":[], "summary":{"errors":0,"warnings":0,"notes":0}
        })
    );
    let all = workspace.run(&["list", "--json"]);
    assert_eq!(all.status.code(), Some(0), "{all:?}");
    let all: serde_json::Value = serde_json::from_slice(&all.stdout).unwrap();
    assert_eq!(all["aliases"], expected);
    for (harness, row, text) in [
        ("codex", 0, "alpha\tcodex\tCode carefully\n"),
        ("claude", 1, "middle\tclaude\t\n"),
        ("copilot", 2, "zed\tcopilot\t\n"),
    ] {
        let shorthand = format!("--{harness}");
        for filter in [vec!["--harness", harness], vec![shorthand.as_str()]] {
            let mut args = vec!["list", "aliases"];
            args.extend(&filter);
            let human = workspace.run(&args);
            assert_eq!(human.status.code(), Some(0), "{human:?}");
            assert_eq!(
                String::from_utf8_lossy(&human.stdout),
                format!("name\tharness\tdescription\n{text}")
            );
            args.push("--json");
            let output = workspace.run(&args);
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            let filtered: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(filtered["aliases"], serde_json::json!([expected[row]]));
            args.remove(1);
            let all = workspace.run(&args);
            assert_eq!(all.status.code(), Some(0), "{all:?}");
            let all: serde_json::Value = serde_json::from_slice(&all.stdout).unwrap();
            assert_eq!(all["aliases"], filtered["aliases"]);
        }
    }
    for kind in ["plugins", "skills", "mcp", "profiles", "sessions"] {
        let output = workspace.run(&["list", kind, "--json"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["aliases"], serde_json::json!([]), "{kind}");
    }
}

#[test]
fn empty_alias_lists_and_config_error_envelopes_keep_the_aliases_array() {
    let workspace = Workspace::new();
    let output = workspace.run(&["list", "aliases"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"name\tharness\tdescription\n");
    let all = workspace.run(&["list"]);
    assert!(!String::from_utf8_lossy(&all.stdout).contains("name\tharness"));
    for kind in [None, Some("aliases"), Some("sessions")] {
        let mut args = vec!["list"];
        args.extend(kind);
        args.push("--json");
        let output = workspace.run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["aliases"], serde_json::json!([]));
    }
    fs::write(workspace.0.path().join("ayran.toml"), "broken = [").unwrap();
    let output = workspace.run(&["list", "aliases", "--json"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["aliases"], serde_json::json!([]));
    assert_eq!(json["diagnostics"][0]["code"], "config-invalid");
}

#[test]
fn mcp_list_distinguishes_connector_bindings_from_native_servers() {
    let workspace = Workspace::new();
    fs::write(workspace.0.path().join("user.toml"),
        "[mcp.linear]\nclaude = { connector = 'claude.ai Linear' }\ncodex = { connector = 'connector_x' }\n").unwrap();
    let output = workspace.run(&["list", "mcp", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["mcp"][0]["bindings"]["claude"],
        serde_json::json!({"kind":"connector", "id":"claude.ai Linear"})
    );
    assert_eq!(
        json["mcp"][0]["bindings"]["codex"],
        serde_json::json!({"kind":"connector", "id":"connector_x"})
    );
    let output = workspace.run(&["list", "mcp", "--claude"]);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("\tconnector\n"));
}

#[test]
fn builtin_layer_binding_validation_and_listing() {
    let workspace = Workspace::new();
    let output = workspace.run(&["list", "skills", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["skills"],
        serde_json::json!([{
            "name":"ayran", "default":false, "description":null, "layer":"built-in",
            "bindings": {
                "claude":{"kind":"builtin", "name":"ayran"},
                "codex":{"kind":"builtin", "name":"ayran"},
                "copilot":{"kind":"builtin", "name":"ayran"}
            }
        }])
    );
    let text = workspace.run(&["list", "skills"]);
    assert!(
        String::from_utf8_lossy(&text.stdout)
            .contains("ayran\t\tbuilt-in\t\tbuiltin\tbuiltin\tbuiltin")
    );
    for binding in ["all", "claude", "codex", "copilot"] {
        fs::write(
            workspace.0.path().join("user.toml"),
            format!("[skills.help]\n{binding} = {{ builtin = 'ayran' }}\n"),
        )
        .unwrap();
        assert!(workspace.run(&["list", "skills"]).status.success());
    }
    for table in ["skills.help", "plugins.help", "mcp.help"] {
        for value in [
            "{ builtin = 'unknown' }",
            "{ builtin = 42 }",
            "{ builtin = 'ayran', path = '.' }",
            "{ builtin = 'ayran', source = 'github:o/r' }",
        ] {
            fs::write(
                workspace.0.path().join("user.toml"),
                format!("[{table}]\nall = {value}\n"),
            )
            .unwrap();
            let output = workspace.run(&["list", "--json"]);
            assert_eq!(
                output.status.code(),
                Some(3),
                "{table}: {value}: {output:?}"
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("config-invalid"));
        }
    }
    for table in ["plugins.help", "mcp.help"] {
        fs::write(
            workspace.0.path().join("user.toml"),
            format!("[{table}]\nall = {{ builtin = 'ayran' }}\n"),
        )
        .unwrap();
        assert_eq!(workspace.run(&["list"]).status.code(), Some(3));
    }
}
