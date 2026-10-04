#![cfg(unix)]
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::{Command, Output},
};
use tempfile::TempDir;
struct Workspace(TempDir);
impl Workspace {
    fn new() -> Self {
        let w = Self(tempfile::tempdir().unwrap());
        w.write("user.toml", "");
        for h in ["claude", "codex", "copilot"] {
            w.write(h, &format!("#!/bin/sh\necho '{h} 9.0.0'\n"));
            fs::set_permissions(w.0.path().join(h), fs::Permissions::from_mode(0o755)).unwrap();
        }
        w
    }
    fn write(&self, path: &str, text: &str) {
        let path = self.0.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn run(&self, args: &[&str]) -> Output {
        self.run_in(self.0.path(), args)
    }
    fn run_in(&self, cwd: &std::path::Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ayran"))
            .current_dir(cwd)
            .env("HOME", self.0.path())
            .env("USERPROFILE", self.0.path())
            .env("AYRAN_CONFIG", self.0.path().join("user.toml"))
            .env("XDG_STATE_HOME", self.0.path().join("state"))
            .env("XDG_CACHE_HOME", self.0.path().join("cache"))
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env_remove("COPILOT_HOME")
            .env("PATH", self.0.path())
            .args(args)
            .output()
            .unwrap()
    }
    fn json(&self, args: &[&str], status: i32) -> serde_json::Value {
        let output = self.run(args);
        assert_eq!(output.status.code(), Some(status), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn claude(&self) {
        self.write(
            ".claude/plugins/installed_plugins.json",
            r#"{"version":2,"plugins":{"review@m":[{"scope":"user","installPath":"unused"}]}}"#,
        );
        self.write(
            ".claude/settings.json",
            r#"{"enabledPlugins":{"review@m":true},"theme":"dark"}"#,
        );
        self.write(
            "claude",
            r#"#!/usr/bin/python3
import json, os, pathlib, sys
if sys.argv[1:] == ['--version']:
    print('2.1.288')
    sys.exit(0)
root = pathlib.Path(os.environ['HOME'])
home = pathlib.Path(os.environ.get('CLAUDE_CONFIG_DIR', root / '.claude'))
assert pathlib.Path.cwd() == home
assert sys.argv[1] == 'plugin'
assert sys.argv[2] in ['enable', 'disable']
assert sys.argv[3:6] == ['--scope', 'user', '--json']
assert len(sys.argv) == 7
(root / 'claude-call.json').write_text(json.dumps({'args': sys.argv[1:], 'home': str(home)}))
reply = root / 'claude-reply.json'
if reply.exists():
    result = json.loads(reply.read_text())
    if result.get('id', sys.argv[6]) == sys.argv[6]:
        print(json.dumps(result))
        sys.exit(1)
path = home / 'settings.json'
settings = json.loads(path.read_text()) if path.exists() else {}
enabled = settings.setdefault('enabledPlugins', {})
id = sys.argv[6]
if sys.argv[2] == 'enable' and id == 'on@skills-dir':
    enabled.pop(id, None)
else:
    enabled[id] = sys.argv[2] == 'enable'
path.write_text(json.dumps(settings))
print(json.dumps({'success': True}))
"#,
        );
    }
    fn codex(&self) {
        self.write(".codex/config.toml", "[plugins.'review@m']\nenabled=true\n");
        self.write(
            ".codex/plugins/cache/m/review/1.0.0/plugin.json",
            "{\"name\":\"review\"}",
        );
    }
    fn copilot(&self) {
        self.write(
            ".copilot/installed-plugins/m/review/plugin.json",
            r#"{"name":"review"}"#,
        );
        self.write(
            ".copilot/config.json",
            r#"{"installedPlugins":[{"name":"review","marketplace":"m","enabled":true}]}"#,
        );
        self.write(
            "copilot",
            r#"#!/usr/bin/python3
import json, os, pathlib, sys
if sys.argv[1:] == ['--version']:
    print('1.0.91')
    sys.exit(0)
root = pathlib.Path(os.environ['HOME'])
home = pathlib.Path(os.environ['COPILOT_HOME'])
assert pathlib.Path.cwd() == home
assert len(sys.argv) == 4 and sys.argv[1] == 'plugin'
assert sys.argv[2] in ['enable', 'disable']
(root / 'copilot-call.json').write_text(json.dumps({'args': sys.argv[1:], 'home': str(home)}))
if (root / 'copilot-fail').exists():
    print('write refused', file=sys.stderr)
    sys.exit(1)
path = home / 'config.json'
config = json.loads(path.read_text()) if path.exists() else {}
enabled = {}
for plugin in config.get('installedPlugins', []):
    id = plugin['name'] + ('@' + plugin['marketplace'] if plugin.get('marketplace') else '')
    enabled[id] = plugin.get('enabled', True)
    if id == sys.argv[3]:
        plugin['enabled'] = sys.argv[2] == 'enable'
path.write_text(json.dumps(config))
path = home / 'settings.json'
settings = json.loads(path.read_text()) if path.exists() else {}
enabled = settings.setdefault('enabledPlugins', enabled)
enabled[sys.argv[3]] = sys.argv[2] == 'enable'
path.write_text(json.dumps(settings))
"#,
        );
    }
}

#[test]
fn copilot_list_reads_comment_prefixed_managed_config() {
    let w = Workspace::new();
    w.copilot();
    let path = w.0.path().join(".copilot/config.json");
    let contents = format!(
        "// User settings belong in settings.json.\n// This file is managed automatically.\n{}",
        fs::read_to_string(&path).unwrap()
    );
    fs::write(&path, &contents).unwrap();

    let output = w.json(&["native", "plugin", "list", "--copilot", "--json"], 0);
    assert_eq!(output["plugins"][0]["id"], "review@m");
    assert_eq!(output["plugins"][0]["state"], "on");
    assert_eq!(fs::read_to_string(path).unwrap(), contents);

    w.write(
        ".copilot/settings.json",
        "// User settings\n{\"enabledPlugins\":{\"review@m\":false,},}\n",
    );
    let output = w.json(&["native", "plugin", "list", "--copilot", "--json"], 0);
    assert_eq!(output["plugins"][0]["state"], "off");
    let output = w.json(
        &[
            "native",
            "plugin",
            "enable",
            "review@m",
            "--copilot",
            "--dry-run",
            "--json",
        ],
        0,
    );
    assert_eq!(output["changes"][0]["before"], "off");
    assert_eq!(output["changes"][0]["outcome"], "planned");
    assert!(!w.0.path().join("copilot-call.json").exists());
}

#[test]
fn copilot_disable_changes_state_through_the_native_cli() {
    let w = Workspace::new();
    w.copilot();
    let json = w.json(
        &[
            "native",
            "plugin",
            "disable",
            "review@m",
            "--copilot",
            "--json",
        ],
        0,
    );
    assert_eq!(
        json["changes"][0],
        serde_json::json!({
            "harness":"copilot", "id":"review@m", "logical":null, "before":"on", "after":"off",
            "written":"copilot plugin disable 'review@m'", "outcome":"changed"
        })
    );
    assert_eq!(
        w.json(&["native", "plugin", "list", "--copilot", "--json"], 0)["plugins"][0]["state"],
        "off"
    );
}
#[test]
fn list_reports_native_state_binding_and_reachability() {
    let w = Workspace::new();
    w.codex();
    w.write(
        "user.toml",
        "[plugins.review]\ncodex='review@m'\ndefault=true\n",
    );
    let json = w.json(&["native", "plugin", "list", "--codex", "--json"], 0);
    assert_eq!(json["version"], 1);
    assert_eq!(
        json["plugins"],
        serde_json::json!([{
            "harness":"codex", "id":"review@m", "state":"on", "layer":w.0.path().join(".codex/config.toml"),
            "logical":["review"], "default":true, "reachable":true
        }])
    );
}

#[test]
fn logical_names_resolve_across_installed_harnesses_with_skipped_binding_notes() {
    let w = Workspace::new();
    w.codex();
    w.write(
        "user.toml",
        "[plugins.'review@m']\ncodex='review@m'\nclaude=false\ncopilot={path='plugin'}\n",
    );
    let json = w.json(
        &[
            "native",
            "plugin",
            "disable",
            "review@m",
            "--dry-run",
            "--json",
        ],
        0,
    );
    assert_eq!(
        json["changes"],
        serde_json::json!([{
            "harness":"codex", "id":"review@m", "logical":"review@m", "before":"on", "after":"off",
            "written":null, "outcome":"planned"
        }])
    );
    assert_eq!(json["summary"]["notes"], 2);
    assert_eq!(
        w.json(&["native", "plugin", "list", "--codex", "--json"], 0)["plugins"][0]["state"],
        "on"
    );
}

#[test]
fn native_id_requires_harness_and_force_id_bypasses_logical_name() {
    let w = Workspace::new();
    w.codex();
    w.write("user.toml", "[plugins.'review@m']\ncodex='missing@m'\n");
    let logical = w.json(
        &[
            "native",
            "plugin",
            "disable",
            "review@m",
            "--codex",
            "--dry-run",
            "--json",
        ],
        3,
    );
    assert_eq!(logical["diagnostics"][0]["code"], "native-not-found");
    assert_eq!(logical["changes"], serde_json::json!([]));
    let forced = w.json(
        &[
            "native",
            "plugin",
            "disable",
            "review@m",
            "--codex",
            "--id",
            "--dry-run",
            "--json",
        ],
        0,
    );
    assert_eq!(forced["changes"][0]["logical"], serde_json::Value::Null);
    assert_eq!(forced["changes"][0]["id"], "review@m");
    let unflagged = w.json(
        &[
            "native",
            "plugin",
            "disable",
            "review@m",
            "--id",
            "--dry-run",
            "--json",
        ],
        2,
    );
    assert_eq!(unflagged["diagnostics"][0]["code"], "usage");
}
#[test]
fn validation_failure_clears_all_plans_without_writing() {
    let w = Workspace::new();
    w.codex();
    let json = w.json(
        &[
            "native",
            "plugin",
            "disable",
            "review@m",
            "missing@m",
            "--codex",
            "--dry-run",
            "--json",
        ],
        3,
    );
    assert_eq!(json["changes"], serde_json::json!([]));
    let json = w.json(
        &[
            "native",
            "plugin",
            "disable",
            "review@m",
            "missing@m",
            "--codex",
            "--json",
        ],
        3,
    );
    assert_eq!(json["diagnostics"][0]["code"], "native-not-found");
    assert_eq!(json["changes"], serde_json::json!([]));
    assert_eq!(
        w.json(&["native", "plugin", "list", "--codex", "--json"], 0)["plugins"][0]["state"],
        "on"
    );
}
#[test]
fn claude_marketplace_defaults_off_but_skills_dir_follows_manifest() {
    let w = Workspace::new();
    w.write(
        ".claude/plugins/installed_plugins.json",
        r#"{"version":2,"plugins":{"review@m":[{"scope":"user","installPath":"unused"}]}}"#,
    );
    w.write(
        ".claude/skills/on/.claude-plugin/plugin.json",
        r#"{"name":"on"}"#,
    );
    w.write(
        ".claude/skills/off/.claude-plugin/plugin.json",
        r#"{"name":"off","defaultEnabled":false}"#,
    );
    let json = w.json(&["native", "plugin", "list", "--claude", "--json"], 0);
    let states: Vec<_> = json["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["id"].as_str().unwrap(), p["state"].as_str().unwrap()))
        .collect();
    assert_eq!(
        states,
        [
            ("off@skills-dir", "off"),
            ("on@skills-dir", "on"),
            ("review@m", "off")
        ]
    );
    w.write(
        ".claude/settings.json",
        r#"{"enabledPlugins":{"review@m":false}}"#,
    );
    // HOME is not the working directory here, so exercise a separate project layer.
    w.write(
        "project/.claude/settings.json",
        r#"{"enabledPlugins":{"review@m":true}}"#,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ayran"))
        .current_dir(w.0.path().join("project"))
        .env("HOME", w.0.path())
        .env("PATH", w.0.path())
        .env("AYRAN_CONFIG", w.0.path().join("user.toml"))
        .env_remove("CLAUDE_CONFIG_DIR")
        .args(["native", "plugin", "list", "--claude", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let row = json["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "review@m")
        .unwrap();
    assert_eq!(row["state"], "on");
    assert_eq!(
        row["layer"],
        w.0.path()
            .join("project/.claude/settings.json")
            .display()
            .to_string()
    );
}
#[test]
fn copilot_settings_map_disables_omitted_ids() {
    let w = Workspace::new();
    w.write(
        ".copilot/installed-plugins/m/review/plugin.json",
        r#"{"name":"review"}"#,
    );
    w.write(
        ".copilot/config.json",
        r#"{"installedPlugins":[{"name":"review","marketplace":"m","enabled":true}]}"#,
    );
    assert_eq!(
        w.json(&["native", "plugin", "list", "--copilot", "--json"], 0)["plugins"][0]["state"],
        "on"
    );
    w.write(".copilot/settings.json", r#"{"enabledPlugins":{}}"#);
    let json = w.json(&["native", "plugin", "list", "--copilot", "--json"], 0);
    assert_eq!(json["plugins"][0]["state"], "off");
    assert_eq!(
        json["plugins"][0]["layer"],
        w.0.path()
            .join(".copilot/settings.json")
            .display()
            .to_string()
    );
}
#[test]
fn direct_copilot_installs_are_listed_and_can_be_resolved_as_native_ids() {
    let w = Workspace::new();
    w.write(
        ".copilot/installed-plugins/_direct/source-id/plugin.json",
        r#"{"name":"direct"}"#,
    );
    let json = w.json(&["native", "plugin", "list", "--copilot", "--json"], 0);
    assert_eq!(json["plugins"][0]["id"], "source-id");
    assert_eq!(json["plugins"][0]["state"], "on");
    let json = w.json(
        &[
            "native",
            "plugin",
            "disable",
            "source-id",
            "--copilot",
            "--dry-run",
            "--json",
        ],
        0,
    );
    assert_eq!(json["changes"][0]["id"], "source-id");
}

#[test]
fn completion_offers_names_and_ids_for_the_requested_state() {
    let w = Workspace::new();
    w.codex();
    w.write("user.toml", "[plugins.review]\ncodex='review@m'\n");
    let candidates = |action| {
        let out = w.run(&[
            "__complete",
            "--",
            "ayran",
            "native",
            "plugin",
            action,
            "--codex",
            "",
        ]);
        assert_eq!(out.status.code(), Some(0));
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert!(candidates("disable").contains(&"review".into()));
    assert!(candidates("disable").contains(&"review@m".into()));
    assert!(!candidates("enable").contains(&"review".into()));
    w.write(
        ".codex/config.toml",
        "[plugins.'review@m']\nenabled=false\n",
    );
    assert!(candidates("enable").contains(&"review@m".into()));
    assert!(!candidates("disable").contains(&"review@m".into()));
    let out = w.run(&[
        "__complete",
        "--",
        "ayran",
        "native",
        "plugin",
        "enable",
        "--codex",
        "--id",
        "",
    ]);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(!text.lines().any(|s| s == "review"));
    assert!(text.lines().any(|s| s == "review@m"));
}

#[test]
fn doctor_suggests_permanent_disable_only_for_unreachable_on_plugins() {
    let w = Workspace::new();
    w.codex();
    w.write("user.toml", "[plugins.review]\ncodex='review@m'\n");
    let json = w.json(&["doctor", "--codex", "--json"], 0);
    let notes: Vec<_> = json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "native-plugin-unreachable")
        .collect();
    assert_eq!(notes.len(), 1, "{json}");
    assert_eq!(notes[0]["severity"], "note");
    assert!(
        notes[0]["message"]
            .as_str()
            .unwrap()
            .contains("ayran native plugin disable review@m --codex")
    );
    for selection in [
        "[profiles.team]\nplugins=['review']\ndefault=true\n",
        "[aliases.cr]\nharness='codex'\nplugins=['review']\n",
    ] {
        w.write(
            "user.toml",
            &format!("[plugins.review]\ncodex='review@m'\n{selection}"),
        );
        let json = w.json(&["doctor", "--codex", "--json"], 0);
        assert!(
            !json["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["code"] == "native-plugin-unreachable"),
            "{json}"
        );
        assert_eq!(
            w.json(&["native", "plugin", "list", "--codex", "--json"], 0)["plugins"][0]["reachable"],
            true
        );
    }
}

#[test]
fn ordinary_files_in_claude_skill_roots_do_not_block_plugin_defaults() {
    let w = Workspace::new();
    w.write(".claude/skills/README.md", "ordinary file");
    w.write(
        ".claude/skills/on/.claude-plugin/plugin.json",
        r#"{"name":"on"}"#,
    );
    assert_eq!(
        w.json(&["native", "plugin", "list", "--claude", "--json"], 0)["plugins"][0]["state"],
        "on"
    );
}

#[test]
fn codex_file_profile_is_deciding_layer_and_project_override_is_unknown() {
    let w = Workspace::new();
    w.codex();
    w.write(
        "user.toml",
        "[harnesses.codex]\nargs=['--profile', 'quiet']\n",
    );
    w.write(
        ".codex/quiet.config.toml",
        "[plugins.'review@m']\nenabled=false\n",
    );
    let json = w.json(&["native", "plugin", "list", "--codex", "--json"], 0);
    assert_eq!(json["plugins"][0]["state"], "off");
    assert_eq!(
        json["plugins"][0]["layer"],
        w.0.path()
            .join(".codex/quiet.config.toml")
            .display()
            .to_string()
    );
    w.write(
        "project/.codex/config.toml",
        "[plugins.'review@m']\nenabled=true\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ayran"))
        .current_dir(w.0.path().join("project"))
        .env("HOME", w.0.path())
        .env("PATH", w.0.path())
        .env("AYRAN_CONFIG", w.0.path().join("user.toml"))
        .env_remove("CODEX_HOME")
        .args(["native", "plugin", "list", "--codex", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["plugins"][0]["state"], "unknown");
    assert_eq!(
        json["plugins"][0]["layer"],
        w.0.path()
            .join("project/.codex/config.toml")
            .display()
            .to_string()
    );
}
#[test]
fn isolated_home_is_used_and_missing_home_remains_uncreated() {
    let w = Workspace::new();
    w.codex();
    w.write("user.toml", "[harnesses.codex]\nhome='isolated'\n");
    assert_eq!(
        w.json(&["native", "plugin", "list", "--codex", "--json"], 0)["plugins"],
        serde_json::json!([])
    );
    assert!(!w.0.path().join("state/ayran/homes/codex").exists());
    w.write(
        "state/ayran/homes/codex/config.toml",
        "[plugins.'isolated@m']\nenabled=false\n",
    );
    w.write(
        "state/ayran/homes/codex/plugins/cache/m/isolated/1.0.0/plugin.json",
        r#"{"name":"isolated"}"#,
    );
    let json = w.json(&["native", "plugin", "list", "--codex", "--json"], 0);
    assert_eq!(json["plugins"].as_array().unwrap().len(), 1);
    assert_eq!(json["plugins"][0]["id"], "isolated@m");
    assert_eq!(json["plugins"][0]["state"], "off");
}
#[test]
fn missing_bindings_are_noted_and_uninstalled_harnesses_are_skipped() {
    let w = Workspace::new();
    w.codex();
    w.write("user.toml", "[plugins.review]\ncodex='review@m'\n");
    let json = w.json(
        &[
            "native",
            "plugin",
            "enable",
            "review",
            "--dry-run",
            "--json",
        ],
        0,
    );
    assert_eq!(json["summary"]["notes"], 2);
    fs::remove_file(w.0.path().join("copilot")).unwrap();
    let json = w.json(
        &[
            "native",
            "plugin",
            "enable",
            "review",
            "--dry-run",
            "--json",
        ],
        0,
    );
    assert_eq!(json["summary"]["notes"], 1);
    let json = w.json(&["native", "plugin", "list", "--copilot", "--json"], 3);
    assert_eq!(json["diagnostics"][0]["code"], "harness-not-found");
}
#[test]
fn harness_flags_are_exclusive_and_human_output_shows_interpretation() {
    let w = Workspace::new();
    w.codex();
    w.write("user.toml", "[plugins.review]\ncodex='review@m'\n");
    for args in [
        vec!["--codex", "--claude"],
        vec!["--codex", "--harness", "codex"],
    ] {
        let mut command = vec!["native", "plugin", "list"];
        command.extend(args);
        assert_eq!(w.run(&command).status.code(), Some(2));
    }
    let output = w.run(&[
        "native",
        "plugin",
        "disable",
        "review",
        "--harness",
        "codex",
        "--dry-run",
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "review → codex review@m: on → off (planned)\n"
    );
}

#[test]
fn claude_disable_changes_user_state_through_the_native_cli() {
    let w = Workspace::new();
    w.claude();
    let json = w.json(
        &[
            "native", "plugin", "disable", "review@m", "--claude", "--json",
        ],
        0,
    );
    assert_eq!(
        json["changes"][0],
        serde_json::json!({
            "harness":"claude", "id":"review@m", "logical":null, "before":"on", "after":"off",
            "written":"claude plugin disable --scope user --json 'review@m'", "outcome":"changed"
        })
    );
    assert_eq!(
        w.json(&["native", "plugin", "list", "--claude", "--json"], 0)["plugins"][0]["state"],
        "off"
    );
}

#[test]
fn claude_enable_and_already_in_state_are_reported() {
    let w = Workspace::new();
    w.claude();
    w.write(
        ".claude/settings.json",
        r#"{"enabledPlugins":{"review@m":false}}"#,
    );
    let json = w.json(
        &[
            "native", "plugin", "enable", "review@m", "--claude", "--json",
        ],
        0,
    );
    assert_eq!(json["changes"][0]["before"], "off");
    assert_eq!(json["changes"][0]["after"], "on");
    assert_eq!(json["changes"][0]["outcome"], "changed");
    assert_eq!(
        json["changes"][0]["written"],
        "claude plugin enable --scope user --json 'review@m'"
    );
    fs::remove_file(w.0.path().join("claude-call.json")).unwrap();
    let json = w.json(
        &[
            "native", "plugin", "enable", "review@m", "--claude", "--json",
        ],
        0,
    );
    assert_eq!(json["changes"][0]["outcome"], "unchanged");
    assert!(json["changes"][0]["written"].is_null());
    assert!(!w.0.path().join("claude-call.json").exists());
}

#[test]
fn claude_old_harness_refuses_all_writes() {
    let w = Workspace::new();
    w.claude();
    w.write("claude", "#!/bin/sh\necho '2.1.282'\n");
    let json = w.json(
        &[
            "native", "plugin", "disable", "review@m", "--claude", "--json",
        ],
        3,
    );
    assert_eq!(json["diagnostics"][0]["code"], "harness-too-old");
    assert_eq!(json["changes"], serde_json::json!([]));
    assert!(!w.0.path().join("claude-call.json").exists());
}

#[test]
fn claude_dry_run_and_unknown_id_never_call_the_writer() {
    let w = Workspace::new();
    w.claude();
    let json = w.json(
        &[
            "native",
            "plugin",
            "disable",
            "review@m",
            "--claude",
            "--dry-run",
            "--json",
        ],
        0,
    );
    assert_eq!(json["changes"][0]["outcome"], "planned");
    assert!(json["changes"][0]["written"].is_null());
    let json = w.json(
        &[
            "native", "plugin", "disable", "review@m", "nope@x", "--claude", "--json",
        ],
        3,
    );
    assert_eq!(json["changes"], serde_json::json!([]));
    assert_eq!(json["diagnostics"][0]["code"], "native-not-found");
    assert!(!w.0.path().join("claude-call.json").exists());
    assert_eq!(
        w.json(&["native", "plugin", "list", "--claude", "--json"], 0)["plugins"][0]["state"],
        "on"
    );
}
#[test]
fn claude_project_and_local_overrides_warn_without_repeating_user_writes() {
    for layer in ["settings.json", "settings.local.json"] {
        let w = Workspace::new();
        w.claude();
        w.write(
            &format!("project/.claude/{layer}"),
            r#"{"enabledPlugins":{"review@m":true}}"#,
        );
        let run = || {
            let out = w.run_in(
                &w.0.path().join("project"),
                &[
                    "native", "plugin", "disable", "review@m", "--claude", "--json",
                ],
            );
            assert_eq!(out.status.code(), Some(0), "{out:?}");
            serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
        };
        let json = run();
        assert_eq!(json["changes"][0]["outcome"], "changed");
        assert_eq!(json["diagnostics"][0]["code"], "native-plugin-overridden");
        assert_eq!(json["diagnostics"][0]["severity"], "warning");
        assert_eq!(
            json["diagnostics"][0]["layer"],
            w.0.path()
                .join(format!("project/.claude/{layer}"))
                .display()
                .to_string()
        );
        fs::remove_file(w.0.path().join("claude-call.json")).unwrap();
        let json = run();
        assert_eq!(json["changes"][0]["before"], "off");
        assert_eq!(json["changes"][0]["outcome"], "unchanged");
        assert_eq!(json["summary"]["warnings"], 1);
        assert!(!w.0.path().join("claude-call.json").exists());
    }
}
#[test]
fn claude_changes_user_state_even_when_project_already_matches_the_goal() {
    let w = Workspace::new();
    w.claude();
    w.write(
        "project/.claude/settings.json",
        r#"{"enabledPlugins":{"review@m":false}}"#,
    );
    let out = w.run_in(
        &w.0.path().join("project"),
        &[
            "native", "plugin", "disable", "review@m", "--claude", "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["changes"][0]["before"], "on");
    assert_eq!(json["changes"][0]["outcome"], "changed");
    assert_eq!(json["summary"]["warnings"], 0);
}
#[test]
fn claude_isolated_home_receives_native_writes() {
    let w = Workspace::new();
    w.claude();
    w.write("project/README", "");
    let json_in_project = |args: &[&str]| {
        let out = w.run_in(&w.0.path().join("project"), args);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };
    w.write(
        ".claude/settings.json",
        r#"{"enabledPlugins":{"review@m":false}}"#,
    );
    w.write("user.toml", "[harnesses.claude]\nhome='isolated'\n");
    w.write(
        "state/ayran/homes/claude/plugins/installed_plugins.json",
        r#"{"version":2,"plugins":{"review@m":[{"scope":"user","installPath":"unused"}]}}"#,
    );
    let json = json_in_project(&[
        "native", "plugin", "enable", "review@m", "--claude", "--json",
    ]);
    assert_eq!(json["changes"][0]["outcome"], "changed");
    let call: serde_json::Value =
        serde_json::from_slice(&fs::read(w.0.path().join("claude-call.json")).unwrap()).unwrap();
    assert_eq!(
        call["home"],
        w.0.path()
            .join("state/ayran/homes/claude")
            .display()
            .to_string()
    );
    assert_eq!(
        json_in_project(&["native", "plugin", "list", "--claude", "--json"])["plugins"][0]["state"],
        "on"
    );
    w.write("user.toml", "");
    assert_eq!(
        json_in_project(&["native", "plugin", "list", "--claude", "--json"])["plugins"][0]["state"],
        "off"
    );
}
#[test]
fn claude_skills_dir_enable_allows_the_native_cli_to_remove_the_key() {
    let w = Workspace::new();
    w.claude();
    w.write(
        ".claude/skills/on/.claude-plugin/plugin.json",
        r#"{"name":"on"}"#,
    );
    w.json(
        &[
            "native",
            "plugin",
            "disable",
            "on@skills-dir",
            "--claude",
            "--json",
        ],
        0,
    );
    let json = w.json(
        &[
            "native",
            "plugin",
            "enable",
            "on@skills-dir",
            "--claude",
            "--json",
        ],
        0,
    );
    assert_eq!(json["changes"][0]["outcome"], "changed");
    let json = w.json(&["native", "plugin", "list", "--claude", "--json"], 0);
    assert_eq!(json["plugins"][0]["state"], "on");
    assert!(
        json["plugins"][0]["layer"]
            .as_str()
            .unwrap()
            .ends_with("plugin.json")
    );
}
#[test]
fn claude_already_in_goal_state_exit_one_is_unchanged() {
    let w = Workspace::new();
    w.claude();
    w.write(
        "claude-reply.json",
        r#"{"failureCode":"already_in_goal_state"}"#,
    );
    let json = w.json(
        &[
            "native", "plugin", "disable", "review@m", "--claude", "--json",
        ],
        0,
    );
    assert_eq!(json["changes"][0]["outcome"], "unchanged");
    assert_eq!(
        json["changes"][0]["written"],
        "claude plugin disable --scope user --json 'review@m'"
    );
}
#[test]
fn claude_writer_failure_reports_the_command_and_refuses_later_targets() {
    let w = Workspace::new();
    w.claude();
    w.write(
        ".claude/skills/on/.claude-plugin/plugin.json",
        r#"{"name":"on"}"#,
    );
    w.write(
        "claude-reply.json",
        r#"{"failureCode":"write_failed","message":"read-only home"}"#,
    );
    let json = w.json(
        &[
            "native",
            "plugin",
            "disable",
            "review@m",
            "on@skills-dir",
            "--claude",
            "--json",
        ],
        3,
    );
    assert_eq!(json["changes"][0]["outcome"], "failed");
    assert_eq!(
        json["changes"][0]["written"],
        "claude plugin disable --scope user --json 'review@m'"
    );
    assert_eq!(json["changes"][1]["outcome"], "failed");
    assert!(json["changes"][1]["written"].is_null());
    assert_eq!(json["diagnostics"][0]["code"], "native-write-failed");
    assert!(
        json["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("read-only home")
    );
}

#[test]
fn claude_cached_managed_plugin_state_warns_conservatively() {
    let w = Workspace::new();
    w.claude();
    w.write(
        ".claude/remote-settings.json",
        r#"{"enabledPlugins":{"review@m":true}}"#,
    );
    let json = w.json(
        &[
            "native", "plugin", "disable", "review@m", "--claude", "--json",
        ],
        0,
    );
    assert_eq!(json["changes"][0]["outcome"], "changed");
    assert_eq!(json["diagnostics"][0]["code"], "native-plugin-overridden");
    assert_eq!(
        json["diagnostics"][0]["layer"],
        w.0.path()
            .join(".claude/remote-settings.json")
            .display()
            .to_string()
    );
}

#[test]
fn claude_project_skills_dir_can_be_disabled_in_a_fresh_isolated_home() {
    let w = Workspace::new();
    w.claude();
    w.write("user.toml", "[harnesses.claude]\nhome='isolated'\n");
    w.write(
        "project/.claude/skills/on/.claude-plugin/plugin.json",
        r#"{"name":"on"}"#,
    );
    let out = w.run_in(
        &w.0.path().join("project"),
        &[
            "native",
            "plugin",
            "disable",
            "on@skills-dir",
            "--claude",
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["changes"][0]["outcome"], "changed");
    assert!(
        w.0.path()
            .join("state/ayran/homes/claude/settings.json")
            .exists()
    );
}

#[test]
fn claude_partial_failure_keeps_the_completed_change_visible() {
    let w = Workspace::new();
    w.claude();
    w.write(
        ".claude/skills/on/.claude-plugin/plugin.json",
        r#"{"name":"on"}"#,
    );
    w.write(
        "claude-reply.json",
        r#"{"id":"on@skills-dir","failureCode":"write_failed"}"#,
    );
    let json = w.json(
        &[
            "native",
            "plugin",
            "disable",
            "review@m",
            "on@skills-dir",
            "--claude",
            "--json",
        ],
        3,
    );
    assert_eq!(json["changes"][0]["outcome"], "changed");
    assert_eq!(json["changes"][1]["outcome"], "failed");
    assert!(json["changes"][0]["written"].is_string());
    assert!(json["changes"][1]["written"].is_string());
    let listed = w.json(&["native", "plugin", "list", "--claude", "--json"], 0);
    assert_eq!(listed["plugins"][1]["state"], "off");
}
#[test]
fn old_copilot_preflight_prevents_claude_and_codex_writes() {
    let w = Workspace::new();
    w.claude();
    w.codex();
    w.write(
        "user.toml",
        "[plugins.review]\nclaude='review@m'\ncodex='review@m'\ncopilot='review@m'\n",
    );
    w.write(
        ".copilot/installed-plugins/m/review/plugin.json",
        r#"{"name":"review"}"#,
    );
    w.write("copilot", "#!/bin/sh\necho '1.0.87'\n");
    let json = w.json(&["native", "plugin", "disable", "review", "--json"], 3);
    assert_eq!(json["diagnostics"][0]["code"], "harness-too-old");
    assert_eq!(json["changes"], serde_json::json!([]));
    assert!(!w.0.path().join("claude-call.json").exists());
    assert_eq!(
        w.json(&["native", "plugin", "list", "--claude", "--json"], 0)["plugins"][0]["state"],
        "on"
    );
}

#[test]
fn copilot_unknown_disable_warns_and_prevents_all_writes() {
    let w = Workspace::new();
    w.copilot();
    for args in [
        vec![
            "native",
            "plugin",
            "disable",
            "unknown@m",
            "--copilot",
            "--json",
        ],
        vec![
            "native",
            "plugin",
            "disable",
            "review@m",
            "unknown@m",
            "--copilot",
            "--json",
        ],
    ] {
        let json = w.json(&args, 0);
        assert_eq!(json["changes"], serde_json::json!([]));
        assert_eq!(json["diagnostics"][0]["code"], "native-not-found");
        assert_eq!(json["diagnostics"][0]["severity"], "warning");
    }
    assert!(!w.0.path().join("copilot-call.json").exists());
    assert_eq!(
        w.json(&["native", "plugin", "list", "--copilot", "--json"], 0)["plugins"][0]["state"],
        "on"
    );
    let json = w.json(
        &[
            "native",
            "plugin",
            "enable",
            "unknown@m",
            "--copilot",
            "--json",
        ],
        3,
    );
    assert_eq!(json["diagnostics"][0]["severity"], "error");
}

#[test]
fn copilot_enable_and_both_already_in_state_actions_skip_writes() {
    let w = Workspace::new();
    w.copilot();
    // An existing settings map takes precedence over legacy config.json.
    w.write(".copilot/settings.json", r#"{"enabledPlugins":{}}"#);
    let json = w.json(
        &[
            "native",
            "plugin",
            "enable",
            "review@m",
            "--copilot",
            "--json",
        ],
        0,
    );
    assert_eq!(json["changes"][0]["before"], "off");
    assert_eq!(json["changes"][0]["after"], "on");
    assert_eq!(
        json["changes"][0]["written"],
        "copilot plugin enable 'review@m'"
    );
    assert_eq!(json["changes"][0]["outcome"], "changed");
    assert_eq!(
        w.json(&["native", "plugin", "list", "--copilot", "--json"], 0)["plugins"][0]["state"],
        "on"
    );
    for action in ["enable", "disable"] {
        if action == "disable" {
            w.json(
                &[
                    "native",
                    "plugin",
                    action,
                    "review@m",
                    "--copilot",
                    "--json",
                ],
                0,
            );
        }
        fs::remove_file(w.0.path().join("copilot-call.json")).unwrap();
        let json = w.json(
            &[
                "native",
                "plugin",
                action,
                "review@m",
                "--copilot",
                "--json",
            ],
            0,
        );
        assert_eq!(json["changes"][0]["outcome"], "unchanged");
        assert!(json["changes"][0]["written"].is_null());
        assert!(!w.0.path().join("copilot-call.json").exists());
    }
}

#[test]
fn copilot_dry_run_runs_no_harness_command() {
    let w = Workspace::new();
    w.copilot();
    w.write("copilot", "#!/bin/sh\nexit 1\n");
    for action in ["enable", "disable"] {
        let json = w.json(
            &[
                "native",
                "plugin",
                action,
                "review@m",
                "--copilot",
                "--dry-run",
                "--json",
            ],
            0,
        );
        assert_eq!(json["changes"][0]["outcome"], "planned");
        assert!(json["changes"][0]["written"].is_null());
    }
    assert!(!w.0.path().join(".copilot/settings.json").exists());
}

#[test]
fn copilot_writer_targets_isolated_and_custom_homes() {
    for isolated in [true, false] {
        let w = Workspace::new();
        w.copilot();
        let relative = if isolated {
            "state/ayran/homes/copilot"
        } else {
            "custom-copilot"
        };
        let home = w.0.path().join(relative);
        w.write(
            &format!("{relative}/installed-plugins/m/review/plugin.json"),
            r#"{"name":"review"}"#,
        );
        w.write(
            &format!("{relative}/config.json"),
            r#"{"installedPlugins":[{"name":"review","marketplace":"m","enabled":true}]}"#,
        );
        if isolated {
            w.write("user.toml", "[harnesses.copilot]\nhome='isolated'\n");
        }
        let run = |args: &[&str]| {
            let mut command = Command::new(env!("CARGO_BIN_EXE_ayran"));
            command
                .current_dir(w.0.path())
                .env("HOME", w.0.path())
                .env("AYRAN_CONFIG", w.0.path().join("user.toml"))
                .env("XDG_STATE_HOME", w.0.path().join("state"))
                .env("XDG_CACHE_HOME", w.0.path().join("cache"))
                .env("PATH", w.0.path())
                // Isolated mode must ignore a real-home override.
                .env(
                    "COPILOT_HOME",
                    if isolated {
                        w.0.path().join(".copilot")
                    } else {
                        home.clone()
                    },
                )
                .args(args);
            let out = command.output().unwrap();
            assert_eq!(out.status.code(), Some(0), "{out:?}");
            serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
        };
        let json = run(&[
            "native",
            "plugin",
            "disable",
            "review@m",
            "--copilot",
            "--json",
        ]);
        assert_eq!(json["changes"][0]["outcome"], "changed");
        assert_eq!(
            run(&["native", "plugin", "list", "--copilot", "--json"])["plugins"][0]["state"],
            "off"
        );
        let call: serde_json::Value =
            serde_json::from_slice(&fs::read(w.0.path().join("copilot-call.json")).unwrap())
                .unwrap();
        assert_eq!(call["home"], home.display().to_string());
        assert!(!w.0.path().join(".copilot/settings.json").exists());
    }
}

#[test]
fn copilot_write_failure_reports_the_command_and_stops_later_writes() {
    let w = Workspace::new();
    w.copilot();
    w.write(
        ".copilot/installed-plugins/_direct/source-id/plugin.json",
        r#"{"name":"direct"}"#,
    );
    w.write("copilot-fail", "");
    let json = w.json(
        &[
            "native",
            "plugin",
            "disable",
            "review@m",
            "source-id",
            "--copilot",
            "--json",
        ],
        3,
    );
    assert_eq!(json["changes"][0]["outcome"], "failed");
    assert_eq!(
        json["changes"][0]["written"],
        "copilot plugin disable 'review@m'"
    );
    assert_eq!(json["changes"][1]["outcome"], "failed");
    assert!(json["changes"][1]["written"].is_null());
    assert_eq!(json["diagnostics"][0]["code"], "native-write-failed");
    assert!(
        json["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("write refused")
    );
    assert_eq!(
        w.json(&["native", "plugin", "list", "--copilot", "--json"], 0)["plugins"][0]["state"],
        "on"
    );
}

#[test]
fn codex_disable_preserves_comments_and_enable_is_idempotent() {
    let w = Workspace::new();
    w.codex();
    let original = "# user preferences\nmodel = 'test' # keep model\n\n[plugins.'review@m'] # keep table\nenabled = true # keep toggle\nother = 'kept'\n";
    w.write(".codex/config.toml", original);
    let json = w.json(
        &[
            "native", "plugin", "disable", "review@m", "--codex", "--json",
        ],
        0,
    );
    assert_eq!(
        json["changes"][0],
        serde_json::json!({
            "harness":"codex", "id":"review@m", "logical":null, "before":"on", "after":"off",
            "written":w.0.path().join(".codex/config.toml"), "outcome":"changed"
        })
    );
    assert_eq!(
        fs::read_to_string(w.0.path().join(".codex/config.toml")).unwrap(),
        "# user preferences\nmodel = 'test' # keep model\n\n[plugins.'review@m'] # keep table\nenabled = false # keep toggle\nother = 'kept'\n"
    );
    assert_eq!(
        w.json(&["native", "plugin", "list", "--codex", "--json"], 0)["plugins"][0]["state"],
        "off"
    );
    let json = w.json(
        &[
            "native", "plugin", "enable", "review@m", "--codex", "--json",
        ],
        0,
    );
    assert_eq!(json["changes"][0]["before"], "off");
    assert_eq!(json["changes"][0]["outcome"], "changed");
    assert_eq!(
        fs::read_to_string(w.0.path().join(".codex/config.toml")).unwrap(),
        original
    );
    let before = fs::metadata(w.0.path().join(".codex/config.toml"))
        .unwrap()
        .modified()
        .unwrap();
    let json = w.json(
        &[
            "native", "plugin", "enable", "review@m", "--codex", "--json",
        ],
        0,
    );
    assert_eq!(json["changes"][0]["outcome"], "unchanged");
    assert!(json["changes"][0]["written"].is_null());
    assert_eq!(
        fs::metadata(w.0.path().join(".codex/config.toml"))
            .unwrap()
            .modified()
            .unwrap(),
        before
    );
}

#[test]
fn codex_dotted_id_and_missing_enabled_value_are_written_explicitly() {
    let w = Workspace::new();
    w.write(".codex/config.toml", "[plugins.\"a.b@m.kt\"]\n");
    w.write(
        ".codex/plugins/cache/m.kt/a.b/local/plugin.json",
        r#"{"name":"a.b"}"#,
    );
    let json = w.json(
        &[
            "native", "plugin", "disable", "a.b@m.kt", "--codex", "--json",
        ],
        0,
    );
    assert_eq!(json["changes"][0]["before"], "unknown");
    assert_eq!(json["changes"][0]["outcome"], "changed");
    assert_eq!(
        fs::read_to_string(w.0.path().join(".codex/config.toml")).unwrap(),
        "[plugins.\"a.b@m.kt\"]\nenabled = false\n"
    );
    let json = w.json(
        &[
            "native", "plugin", "enable", "a.b@m.kt", "--codex", "--json",
        ],
        0,
    );
    assert_eq!(json["changes"][0]["outcome"], "changed");
    assert_eq!(
        w.json(&["native", "plugin", "list", "--codex", "--json"], 0)["plugins"][0]["state"],
        "on"
    );
}

#[test]
fn codex_profile_and_project_overrides_warn_but_user_state_controls_idempotence() {
    for profile in [false, true] {
        let w = Workspace::new();
        w.codex();
        w.write("project/README", "");
        let override_path = if profile {
            w.write("user.toml", "[harnesses.codex]\nargs=['-p', 'work']\n");
            ".codex/work.config.toml"
        } else {
            "project/.codex/config.toml"
        };
        w.write(override_path, "[plugins.'review@m']\nenabled=true\n");
        let original_override = fs::read(w.0.path().join(override_path)).unwrap();
        for outcome in ["changed", "unchanged"] {
            let output = w.run_in(
                &w.0.path().join("project"),
                &[
                    "native", "plugin", "disable", "review@m", "--codex", "--json",
                ],
            );
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(json["changes"][0]["outcome"], outcome);
            assert_eq!(
                json["changes"][0]["before"],
                if outcome == "changed" { "on" } else { "off" }
            );
            assert_eq!(json["diagnostics"][0]["code"], "native-plugin-overridden");
            assert_eq!(json["diagnostics"][0]["severity"], "warning");
            assert_eq!(
                json["diagnostics"][0]["layer"],
                w.0.path().join(override_path).display().to_string()
            );
            assert_eq!(
                fs::read(w.0.path().join(override_path)).unwrap(),
                original_override
            );
        }
    }
}

#[test]
fn codex_isolated_home_receives_writes_and_dry_run_leaves_it_untouched() {
    let w = Workspace::new();
    w.codex();
    w.write("user.toml", "[harnesses.codex]\nhome='isolated'\n");
    let path = "state/ayran/homes/codex/config.toml";
    w.write(path, "[plugins.'review@m']\nenabled=false\n");
    w.write(
        "state/ayran/homes/codex/plugins/cache/m/review/local/plugin.json",
        r#"{"name":"review"}"#,
    );
    let json = w.json(
        &[
            "native",
            "plugin",
            "enable",
            "review@m",
            "--codex",
            "--dry-run",
            "--json",
        ],
        0,
    );
    assert_eq!(json["changes"][0]["outcome"], "planned");
    assert!(json["changes"][0]["written"].is_null());
    assert_eq!(
        fs::read_to_string(w.0.path().join(path)).unwrap(),
        "[plugins.'review@m']\nenabled=false\n"
    );
    let json = w.json(
        &[
            "native", "plugin", "enable", "review@m", "--codex", "--json",
        ],
        0,
    );
    assert_eq!(
        json["changes"][0]["written"],
        w.0.path().join(path).display().to_string()
    );
    assert_eq!(
        w.json(&["native", "plugin", "list", "--codex", "--json"], 0)["plugins"][0]["state"],
        "on"
    );
    assert_eq!(
        fs::read_to_string(w.0.path().join(".codex/config.toml")).unwrap(),
        "[plugins.'review@m']\nenabled=true\n"
    );
}

#[test]
fn codex_old_harness_and_inactive_cache_refuse_writes() {
    let w = Workspace::new();
    w.codex();
    w.write("codex", "#!/bin/sh\necho 'codex-cli 0.1.0'\n");
    let json = w.json(
        &[
            "native", "plugin", "disable", "review@m", "--codex", "--json",
        ],
        3,
    );
    assert_eq!(json["diagnostics"][0]["code"], "harness-too-old");
    assert_eq!(json["changes"], serde_json::json!([]));
    fs::remove_dir_all(w.0.path().join(".codex/plugins/cache/m/review")).unwrap();
    let json = w.json(
        &[
            "native", "plugin", "disable", "review@m", "--codex", "--json",
        ],
        3,
    );
    assert_eq!(json["diagnostics"][0]["code"], "native-not-found");
    assert_eq!(
        fs::read_to_string(w.0.path().join(".codex/config.toml")).unwrap(),
        "[plugins.'review@m']\nenabled=true\n"
    );
}

#[test]
fn marketplace_list_reports_native_registrations_and_drift() {
    for h in ["claude", "codex", "copilot"] {
        let w = Workspace::new();
        w.write("ayran.toml", &format!("[marketplaces.team]\n{h}={{source='github:owner/repo',name='m',ref='v1'}}\n[marketplaces.alias]\n{h}={{source='github:owner/repo',name='m',ref='v1'}}\n[marketplaces.missing]\nall={{source='github:owner/missing'}}\n"));
        match h {
            "claude" => {
                w.claude();
                w.write(".claude/plugins/known_marketplaces.json", r#"{"m":{"source":{"source":"github","repo":"owner/repo","ref":"v1"}},"extra":{"source":{"source":"git","url":"https://example.com/extra.git"}}}"#);
            }
            "codex" => {
                w.codex();
                w.write(".codex/config.toml", "[plugins.'review@m']\nenabled=false\n[marketplaces.m]\nsource_type='github'\nsource='owner/repo'\nref='v1'\n[marketplaces.extra]\nsource_type='git'\nsource='https://example.com/extra.git'\n");
            }
            _ => {
                w.copilot();
                w.write(".copilot/settings.json", r#"{"extraKnownMarketplaces":{"m":{"source":{"source":"github","repo":"owner/repo","ref":"v1"}},"extra":{"source":{"source":"git","url":"https://example.com/extra.git"}}}}"#);
            }
        }
        let json = w.json(
            &["native", "marketplace", "list", "--harness", h, "--json"],
            0,
        );
        assert_eq!(json["version"], 1);
        assert_eq!(json["summary"]["errors"], 0);
        assert_eq!(json["plugins"], serde_json::json!([]));
        assert_eq!(
            json["marketplaces"],
            serde_json::json!([
                {"harness":h,"name":"extra","source":"https://example.com/extra.git","ref":null,"logical":[],"match":"undeclared","installed_plugins":0},
                {"harness":h,"name":"m","source":"github:owner/repo","ref":"v1","logical":["alias","team"],"match":"matches","installed_plugins":1}
            ])
        );
    }
}

#[test]
fn marketplace_list_compares_source_and_ref_for_each_harness() {
    for h in ["claude", "codex", "copilot"] {
        for declaration in [
            "source='github:other/repo',ref='v1'",
            "source='github:owner/repo',ref='v2'",
        ] {
            let w = Workspace::new();
            w.write(
                "ayran.toml",
                &format!("[marketplaces.team]\n{h}={{{declaration},name='m'}}\n"),
            );
            if h == "codex" {
                w.write(
                    ".codex/config.toml",
                    "[marketplaces.m]\nsource_type='github'\nsource='owner/repo'\nref='v1'\n",
                );
            } else {
                let entry =
                    r#"{"m":{"source":{"source":"github","repo":"owner/repo","ref":"v1"}}}"#;
                if h == "claude" {
                    w.write(".claude/plugins/known_marketplaces.json", entry);
                } else {
                    w.write(
                        ".copilot/settings.json",
                        &format!("{{\"extraKnownMarketplaces\":{entry}}}"),
                    );
                }
            }
            assert_eq!(
                w.json(
                    &["native", "marketplace", "list", "--harness", h, "--json"],
                    0
                )["marketplaces"][0]["match"],
                "conflict"
            );
        }
    }
}

#[test]
fn marketplace_list_handles_empty_and_malformed_registries() {
    for (h, path) in [
        ("claude", ".claude/plugins/known_marketplaces.json"),
        ("codex", ".codex/config.toml"),
        ("copilot", ".copilot/settings.json"),
    ] {
        let w = Workspace::new();
        assert_eq!(
            w.json(
                &["native", "marketplace", "list", "--harness", h, "--json"],
                0
            )["marketplaces"],
            serde_json::json!([])
        );
        assert!(!w.0.path().join("state").exists());
        w.write(path, "{");
        let json = w.json(
            &["native", "marketplace", "list", "--harness", h, "--json"],
            3,
        );
        assert_eq!(json["diagnostics"][0]["code"], "enumeration-failed");
    }
}

#[test]
fn marketplace_list_preserves_unknown_sources_without_relaxing_install() {
    for (h, path, registry) in [
        (
            "claude",
            ".claude/plugins/known_marketplaces.json",
            r#"{"m":{"source":{"source":"npm","package":"example"}}}"#,
        ),
        (
            "codex",
            ".codex/config.toml",
            "[marketplaces.m]\nsource_type='npm'\nsource='example'\n",
        ),
        (
            "copilot",
            ".copilot/settings.json",
            r#"{"extraKnownMarketplaces":{"m":{"source":{"source":"npm","package":"example"}}}}"#,
        ),
    ] {
        let w = Workspace::new();
        w.write(path, registry);
        w.write(
            "ayran.toml",
            &format!("[marketplaces.m]\n{h}={{source='github:owner/repo'}}\n"),
        );
        let json = w.json(
            &["native", "marketplace", "list", "--harness", h, "--json"],
            0,
        );
        assert_eq!(json["marketplaces"][0]["source"], "npm:example");
        assert_eq!(json["marketplaces"][0]["match"], "conflict");
        let json = w.json(&["install", "--harness", h, "--dry-run", "--json"], 3);
        assert_eq!(json["diagnostics"][0]["code"], "enumeration-failed");
    }
}

#[test]
fn marketplace_list_uses_isolated_home_and_harness_flags() {
    for (h, file, registry) in [
        (
            "claude",
            "plugins/known_marketplaces.json",
            r#"{"isolated":{"source":{"source":"github","repo":"owner/repo"}}}"#,
        ),
        (
            "codex",
            "config.toml",
            "[marketplaces.isolated]\nsource_type='github'\nsource='owner/repo'\n",
        ),
        (
            "copilot",
            "settings.json",
            r#"{"extraKnownMarketplaces":{"isolated":{"source":{"source":"github","repo":"owner/repo"}}}}"#,
        ),
    ] {
        let w = Workspace::new();
        w.write(
            "user.toml",
            &format!("default_harness='codex'\n[harnesses.{h}]\nhome='isolated'\n"),
        );
        let path = format!("state/ayran/homes/{h}/{file}");
        w.write(&path, registry);
        w.write(&format!(".{h}/{file}"), "malformed real home");
        let original = fs::read(w.0.path().join(&path)).unwrap();
        let flag = format!("--{h}");
        for args in [
            vec!["native", "marketplace", "list", flag.as_str(), "--json"],
            vec!["native", "marketplace", "list", "--harness", h, "--json"],
        ] {
            let json = w.json(&args, 0);
            assert_eq!(json["marketplaces"][0]["name"], "isolated");
            assert_eq!(json["marketplaces"][0]["harness"], h);
        }
        assert_eq!(fs::read(w.0.path().join(path)).unwrap(), original);
        assert!(!w.0.path().join("state/ayran/trust.json").exists());
    }
    let w = Workspace::new();
    w.write("user.toml", "default_harness='claude'\n");
    w.write(
        ".codex/config.toml",
        "[marketplaces.only]\nsource_type='github'\nsource='owner/repo'\n",
    );
    assert_eq!(
        w.json(&["native", "marketplace", "list", "--json"], 0)["marketplaces"][0]["harness"],
        "codex"
    );
    assert_eq!(
        w.run(&["native", "marketplace", "list", "--claude", "--codex"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        w.run(&[
            "native",
            "marketplace",
            "list",
            "--claude",
            "--harness",
            "claude"
        ])
        .status
        .code(),
        Some(2)
    );
    for h in ["claude", "codex", "copilot"] {
        fs::remove_file(w.0.path().join(h)).unwrap();
        let flag = format!("--{h}");
        let json = w.json(&["native", "marketplace", "list", &flag, "--json"], 3);
        assert_eq!(json["diagnostics"][0]["code"], "harness-not-found");
    }
}

#[test]
fn marketplace_list_counts_copilot_directory_installs_without_config_records() {
    let w = Workspace::new();
    w.write(
        ".copilot/settings.json",
        r#"{"extraKnownMarketplaces":{"m":{"source":{"source":"github","repo":"owner/repo"}}}}"#,
    );
    w.write(
        ".copilot/installed-plugins/m/review/plugin.json",
        r#"{"name":"review"}"#,
    );
    assert_eq!(
        w.json(&["native", "marketplace", "list", "--copilot", "--json"], 0)["marketplaces"][0]["installed_plugins"],
        1
    );
}

#[test]
fn marketplace_list_rejects_malformed_unknown_source_values() {
    for (h, path, registry) in [
        (
            "claude",
            ".claude/plugins/known_marketplaces.json",
            r#"{"m":{"source":{"source":"url","url":12}}}"#,
        ),
        (
            "copilot",
            ".copilot/settings.json",
            r#"{"extraKnownMarketplaces":{"m":{"source":{"source":"url","url":12}}}}"#,
        ),
    ] {
        let w = Workspace::new();
        w.write(path, registry);
        assert_eq!(
            w.json(
                &["native", "marketplace", "list", "--harness", h, "--json"],
                3
            )["diagnostics"][0]["code"],
            "enumeration-failed"
        );
    }
}

#[test]
fn native_skill_list_reports_claude_state_and_all_logical_bindings() {
    let w = Workspace::new();
    w.write(
        ".claude/skills/review/SKILL.md",
        "---\nname: review\ndescription: Review code\n---\n",
    );
    w.write(
        ".claude/settings.json",
        r#"{"skillOverrides":{"review":"off"}}"#,
    );
    w.write(
        "user.toml",
        "[skills.first]\nclaude='review'\ndefault=true\n[skills.second]\nclaude='review'\n",
    );
    let out = w.json(&["native", "skill", "list", "--claude", "--json"], 0);
    let row = out["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "review")
        .unwrap();
    assert_eq!(row["source"], "personal");
    assert_eq!(row["state"], "off");
    assert_eq!(row["logical"], serde_json::json!(["first", "second"]));
    assert_eq!(row["default"], true);
    assert_eq!(row["reachable"], true);
    assert_eq!(
        row["layer"],
        w.0.path()
            .join(".claude/settings.json")
            .display()
            .to_string()
    );
}

#[test]
fn native_mcp_list_reports_active_codex_profile_and_uncertain_project_state() {
    let w = Workspace::new();
    w.write(
        "user.toml",
        "[harnesses.codex]\nargs=['--profile','work']\n[mcp.review]\ncodex='review'\n",
    );
    w.write(
        ".codex/config.toml",
        "[mcp_servers.review]\ncommand='review'\nenabled=false\n",
    );
    w.write(
        ".codex/work.config.toml",
        "[mcp_servers.profiled]\ncommand='profiled'\nenabled=false\n",
    );
    w.write("project/.git/HEAD", "ref: refs/heads/main\n");
    w.write(
        "project/.codex/config.toml",
        "[mcp_servers.review]\nenabled=true\n",
    );
    let output = w.run_in(
        &w.0.path().join("project"),
        &["native", "mcp", "list", "--codex", "--json"],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let out: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = out["mcp"].as_array().unwrap();
    let profile = rows.iter().find(|r| r["name"] == "profiled").unwrap();
    assert_eq!(profile["source"], "profile");
    assert_eq!(profile["state"], "off");
    assert_eq!(profile["logical"], serde_json::json!([]));
    let review = rows.iter().find(|r| r["name"] == "review").unwrap();
    assert_eq!(review["state"], "unknown");
    assert_eq!(review["logical"], serde_json::json!(["review"]));
}

#[test]
fn native_skill_list_includes_copilot_custom_environment_directories() {
    let w = Workspace::new();
    w.write(
        "custom/review/SKILL.md",
        "---\nname: custom-review\ndescription: Review code\n---\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ayran"))
        .current_dir(w.0.path())
        .env("HOME", w.0.path())
        .env("AYRAN_CONFIG", w.0.path().join("user.toml"))
        .env("PATH", w.0.path())
        .env_remove("COPILOT_HOME")
        .env("COPILOT_SKILLS_DIRS", " custom , , missing ")
        .args(["native", "skill", "list", "--copilot", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let out: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(out["skills"][0]["name"], "custom-review");
    assert_eq!(out["skills"][0]["source"], "custom");
    assert_eq!(out["skills"][0]["state"], "on");
}

#[test]
fn native_skills_use_codex_path_rules_preserve_scope_and_exclude_plugin_skills() {
    let w = Workspace::new();
    w.write(
        ".codex/skills/review/SKILL.md",
        "---\nname: review\ndescription: Review code\n---\n",
    );
    w.write("project/.git/HEAD", "ref: refs/heads/main\n");
    w.write(
        "project/.agents/skills/project/SKILL.md",
        "---\nname: project\ndescription: Project review\n---\n",
    );
    w.write(
        ".codex/skills/plugin/.claude-plugin/plugin.json",
        r#"{"name":"plugin"}"#,
    );
    w.write(
        ".codex/skills/plugin/skills/inside/SKILL.md",
        "---\nname: inside\ndescription: Plugin review\n---\n",
    );
    let path = w.0.path().join(".codex/skills/review/SKILL.md");
    w.write(".codex/config.toml", &format!("[[skills.config]]\npath='{}'\nenabled=false\n[[skills.config]]\nname='project'\nenabled=false\n", path.display()));
    let output = w.run_in(
        &w.0.path().join("project"),
        &["native", "skill", "list", "--codex", "--json"],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let out: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = out["skills"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r["state"] == "off"));
    assert_eq!(
        rows.iter().find(|r| r["name"] == "project").unwrap()["source"],
        "project"
    );
    assert_eq!(
        rows.iter().find(|r| r["name"] == "review").unwrap()["source"],
        "personal"
    );
}

#[test]
fn native_mcp_lists_claude_local_project_connectors_and_copilot_native_off() {
    let w = Workspace::new();
    w.write("project/.git/HEAD", "ref: refs/heads/main\n");
    w.write(
        "project/.mcp.json",
        r#"{"mcpServers":{"project":{"command":"project"},"plugin:inside":{"command":"inside"}}}"#,
    );
    let root = w.0.path().join("project");
    w.write(".claude.json", &serde_json::json!({"mcpServers":{"user":{"command":"user"}},"claudeAiMcpEverConnected":["account"],"projects":{root.display().to_string(): {"mcpServers":{"local":{"command":"local"}},"disabledMcpServers":["local"]}}}).to_string());
    let output = w.run_in(&root, &["native", "mcp", "list", "--claude", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let out: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = out["mcp"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    let local = rows.iter().find(|r| r["name"] == "local").unwrap();
    assert_eq!(local["source"], "local");
    assert_eq!(local["state"], "off");
    assert_eq!(
        rows.iter().find(|r| r["name"] == "account").unwrap()["source"],
        "connector"
    );
    assert_eq!(
        rows.iter().find(|r| r["name"] == "project").unwrap()["source"],
        "project"
    );
    w.write(
        ".copilot/mcp-config.json",
        r#"{"mcpServers":{"review":{"command":"review"}}}"#,
    );
    w.write(
        ".copilot/settings.json",
        "// Native settings\n{\"disabledMcpServers\":[\"review\"],}\n",
    );
    let out = w.json(&["native", "mcp", "list", "--copilot", "--json"], 0);
    assert_eq!(out["mcp"][0]["state"], "off");
    w.write(
        "user.toml",
        "[harnesses.copilot]\nargs=['--enable-mcp-server=review']\n",
    );
    let out = w.json(&["native", "mcp", "list", "--copilot", "--json"], 0);
    assert_eq!(out["mcp"][0]["state"], "unknown");
}

#[test]
fn native_capability_lists_ignore_default_harness_and_enforce_flags_and_errors() {
    let w = Workspace::new();
    w.write("user.toml", "default_harness='codex'\n");
    w.write(
        ".claude/skills/review/SKILL.md",
        "---\nname: review\ndescription: Review code\n---\n",
    );
    let out = w.json(&["native", "skill", "list", "--json"], 0);
    assert!(
        out["skills"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["harness"] == "claude")
    );
    for kind in ["skill", "mcp"] {
        assert_eq!(
            w.run(&["native", kind, "list", "--claude", "--codex"])
                .status
                .code(),
            Some(2)
        );
    }
    fs::remove_file(w.0.path().join("copilot")).unwrap();
    for kind in ["skill", "mcp"] {
        let out = w.json(&["native", kind, "list", "--copilot", "--json"], 3);
        assert_eq!(out["diagnostics"][0]["code"], "harness-not-found");
    }
    w.write(".claude.json", "broken");
    let out = w.json(&["native", "mcp", "list", "--claude", "--json"], 3);
    assert_eq!(out["diagnostics"][0]["code"], "enumeration-failed");
    w.write(".claude/settings.json", "broken");
    let out = w.json(&["native", "skill", "list", "--claude", "--json"], 3);
    assert_eq!(out["diagnostics"][0]["code"], "enumeration-failed");
}

#[test]
fn native_capability_lists_use_isolated_homes_without_creating_them() {
    let w = Workspace::new();
    w.write(
        ".claude/skills/shared/SKILL.md",
        "---\nname: shared\ndescription: Shared review\n---\n",
    );
    w.write(
        ".codex/config.toml",
        "[mcp_servers.shared]\ncommand='shared'\n",
    );
    w.write("user.toml", "[harnesses.claude]\nhome='isolated'\n[harnesses.codex]\nhome='isolated'\n[harnesses.copilot]\nhome='isolated'\n");
    for kind in ["skill", "mcp"] {
        let out = w.json(&["native", kind, "list", "--json"], 0);
        let rows = out[if kind == "skill" { "skills" } else { "mcp" }]
            .as_array()
            .unwrap();
        assert!(!rows.iter().any(|r| r["name"] == "shared"));
    }
    assert!(!w.0.path().join("state").exists());
    assert!(!w.0.path().join("cache").exists());
}

#[test]
fn native_claude_skill_state_matches_settings_precedence_and_uncertainty() {
    let w = Workspace::new();
    w.write(
        ".claude/skills/review/SKILL.md",
        "---\nname: review\ndescription: Review code\n---\n",
    );
    w.write(
        ".claude/settings.json",
        r#"{"skillOverrides":{"review":"off"}}"#,
    );
    w.write("project/.git/HEAD", "ref: refs/heads/main\n");
    w.write(
        "project/.claude/settings.json",
        r#"{"skillOverrides":{"review":"on"}}"#,
    );
    let args = ["native", "skill", "list", "--claude", "--json"];
    let output = w.run_in(&w.0.path().join("project"), &args);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let out: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let row = out["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "review")
        .unwrap();
    assert_eq!(row["state"], "on");
    assert_eq!(
        row["layer"],
        w.0.path()
            .join("project/.claude/settings.json")
            .display()
            .to_string()
    );
    w.write(
        "user.toml",
        "[harnesses.claude]\nargs=['--settings','{}']\n",
    );
    let out = w.json(&args, 0);
    let row = out["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "review")
        .unwrap();
    assert_eq!(row["state"], "unknown");
}

#[test]
fn native_mcp_list_preserves_same_name_across_user_and_project_sources() {
    let w = Workspace::new();
    w.write(
        ".copilot/mcp-config.json",
        r#"{"mcpServers":{"review":{"command":"user"}}}"#,
    );
    w.write(
        ".mcp.json",
        r#"{"mcpServers":{"review":{"command":"project"}}}"#,
    );
    let out = w.json(&["native", "mcp", "list", "--copilot", "--json"], 0);
    let rows = out["mcp"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|r| r["source"] == "user"));
    assert!(rows.iter().any(|r| r["source"] == "project"));
}

#[test]
fn native_custom_skill_list_excludes_skills_inside_plugin_trees() {
    let w = Workspace::new();
    w.write("plugin/.claude-plugin/plugin.json", r#"{"name":"plugin"}"#);
    w.write(
        "plugin/skills/inside/SKILL.md",
        "---\nname: inside\ndescription: Plugin skill\n---\n",
    );
    w.write(
        ".copilot/settings.json",
        r#"{"skillDirectories":["plugin/skills"]}"#,
    );
    let out = w.json(&["native", "skill", "list", "--copilot", "--json"], 0);
    assert_eq!(out["skills"], serde_json::json!([]));
}

#[test]
fn native_copilot_skill_list_matches_invocation_alias_bindings() {
    let w = Workspace::new();
    w.write(
        ".copilot/skills/review/SKILL.md",
        "---\nname: native-review\ndescription: Review code\n---\n",
    );
    w.write(
        "user.toml",
        "[skills.review]\ncopilot='review'\ndefault=true\n",
    );
    let out = w.json(&["native", "skill", "list", "--copilot", "--json"], 0);
    assert_eq!(out["skills"][0]["logical"], serde_json::json!(["review"]));
    assert_eq!(out["skills"][0]["default"], true);
    assert_eq!(out["skills"][0]["reachable"], true);
}

#[test]
fn native_codex_skill_toggle_overrides_ordered_rules_and_retains_inline_toml() {
    let w = Workspace::new();
    w.write(
        ".codex/skills/review/SKILL.md",
        "---\nname: review\ndescription: Review code\n---\n",
    );
    let path = w.0.path().join(".codex/skills/review/SKILL.md");
    let original = format!(
        "# keep\nmodel = 'preferred'\n[skills]\nconfig = [{{path='{}', enabled=false, extra='keep'}}, {{name='review', enabled=true}}] # rules\n",
        path.display()
    );
    w.write(".codex/config.toml", &original);
    let args = ["native", "skill", "disable", "review", "--codex", "--json"];
    let out = w.json(&args, 0);
    assert_eq!(out["changes"][0]["before"], "on");
    assert_eq!(out["changes"][0]["outcome"], "changed");
    let off = fs::read_to_string(w.0.path().join(".codex/config.toml")).unwrap();
    assert!(off.contains("# keep") && off.contains("# rules") && off.contains("extra='keep'"));
    let list = w.json(&["native", "skill", "list", "--codex", "--json"], 0);
    assert_eq!(list["skills"][0]["state"], "off");
    assert_eq!(w.json(&args, 0)["changes"][0]["outcome"], "unchanged");
    let out = w.json(
        &["native", "skill", "enable", "review", "--codex", "--json"],
        0,
    );
    assert_eq!(out["changes"][0]["outcome"], "changed");
    assert_eq!(
        w.json(&["native", "skill", "list", "--codex", "--json"], 0)["skills"][0]["state"],
        "on"
    );
}

#[test]
fn native_claude_skill_can_replace_partial_visibility_with_on_or_off() {
    let w = Workspace::new();
    w.write(
        ".claude/skills/review/SKILL.md",
        "---\nname: review\ndescription: Review code\n---\n",
    );
    w.write(
        ".claude/settings.json",
        r#"{"skillOverrides":{"review":"name-only"},"theme":"dark"}"#,
    );
    let out = w.json(
        &["native", "skill", "disable", "review", "--claude", "--json"],
        0,
    );
    assert_eq!(out["changes"][0]["before"], "unknown");
    assert_eq!(out["changes"][0]["outcome"], "changed");
    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(w.0.path().join(".claude/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(settings["theme"], "dark");
    assert_eq!(settings["skillOverrides"]["review"], "off");
}

#[test]
fn native_skill_off_skips_launch_hides_but_selection_turns_it_on() {
    for harness in ["claude", "codex"] {
        let w = Workspace::new();
        w.write(
            &format!(".{harness}/skills/review/SKILL.md"),
            "---\nname: review\ndescription: Review code\n---\n",
        );
        w.write(
            "user.toml",
            &format!("[skills.logical]\n{harness}='review'\n"),
        );
        let flag = format!("--{harness}");
        let toggle = ["native", "skill", "disable", "logical", &flag, "--json"];
        assert_eq!(w.json(&toggle, 0)["changes"][0]["outcome"], "changed");
        let config = w.0.path().join(format!(
            ".{harness}/{}",
            if harness == "claude" {
                "settings.json"
            } else {
                "config.toml"
            }
        ));
        let original = fs::read(&config).unwrap();
        let trace = w.run(&[&flag, "--dry-run"]);
        assert!(trace.status.success(), "{trace:?}");
        assert!(
            String::from_utf8_lossy(&trace.stderr).contains("natively off"),
            "{trace:?}"
        );
        let selected = w.json(&[&flag, "--skill", "logical", "--dry-run", "--json"], 0);
        let argv = selected["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>()
            .join(" ");
        if harness == "claude" {
            assert!(argv.contains("\"review\":\"on\""), "{argv}");
        } else {
            assert!(
                argv.contains("skills.config=") && argv.contains("enabled=true"),
                "{argv}"
            );
        }
        assert_eq!(fs::read(&config).unwrap(), original);
        let enable = ["native", "skill", "enable", "logical", &flag, "--json"];
        assert_eq!(w.json(&enable, 0)["changes"][0]["outcome"], "changed");
        assert_eq!(w.json(&enable, 0)["changes"][0]["outcome"], "unchanged");
    }
}

#[test]
fn native_skill_validation_and_unsupported_targets_prevent_every_write() {
    let w = Workspace::new();
    for h in ["claude", "codex", "copilot"] {
        w.write(
            &format!(".{h}/skills/review/SKILL.md"),
            "---\nname: review\ndescription: Review code\n---\n",
        );
    }
    w.write("user.toml", "[skills.review]\nall='review'\n");
    let out = w.json(&["native", "skill", "disable", "review", "--json"], 3);
    assert_eq!(out["changes"], serde_json::json!([]));
    assert!(
        out["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "native-unsupported")
    );
    for (h, file) in [
        ("claude", "settings.json"),
        ("codex", "config.toml"),
        ("copilot", "settings.json"),
    ] {
        assert!(!w.0.path().join(format!(".{h}/{file}")).exists());
    }
    let out = w.json(
        &[
            "native", "skill", "disable", "review", "missing", "--claude", "--json",
        ],
        3,
    );
    assert_eq!(out["changes"], serde_json::json!([]));
    assert!(
        out["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "native-not-found")
    );
    assert!(!w.0.path().join(".claude/settings.json").exists());
    assert_eq!(
        w.run(&["native", "skill", "disable", "unknown", "--id"])
            .status
            .code(),
        Some(2)
    );
    w.write(
        "user.toml",
        "[skills.review]\nclaude='review'\ncodex='review'\ncopilot=false\n",
    );
    w.write(".codex/config.toml", "skills = 42\n");
    let out = w.json(&["native", "skill", "disable", "review", "--json"], 3);
    assert_eq!(out["changes"], serde_json::json!([]));
    assert!(!w.0.path().join(".claude/settings.json").exists());
    assert_eq!(
        fs::read_to_string(w.0.path().join(".codex/config.toml")).unwrap(),
        "skills = 42\n"
    );
}

#[test]
fn native_skill_dry_run_skips_bindings_and_keeps_isolated_homes_uncreated() {
    let w = Workspace::new();
    w.write("user.toml", "[harnesses.claude]\nhome='isolated'\n[skills.logical]\nclaude='debug'\ncodex={path='source'}\ncopilot=false\n");
    let out = w.json(
        &[
            "native",
            "skill",
            "disable",
            "logical",
            "--dry-run",
            "--json",
        ],
        0,
    );
    assert_eq!(out["changes"][0]["id"], "debug");
    assert_eq!(out["changes"][0]["outcome"], "planned");
    assert_eq!(
        out["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["code"] == "native-binding-skipped")
            .count(),
        2
    );
    assert!(!w.0.path().join("state").exists());
    assert!(!w.0.path().join("cache").exists());
    let out = w.json(
        &[
            "native", "skill", "disable", "logical", "--claude", "--json",
        ],
        0,
    );
    assert_eq!(out["changes"][0]["outcome"], "changed");
    assert!(
        w.0.path()
            .join("state/ayran/homes/claude/settings.json")
            .exists()
    );
    assert!(!w.0.path().join(".claude/settings.json").exists());
    let out = w.json(&["native", "skill", "list", "--claude", "--json"], 0);
    assert_eq!(
        out["skills"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == "debug")
            .unwrap()["state"],
        "off"
    );
}

#[test]
fn native_skill_effective_overrides_warn_while_user_state_controls_idempotence() {
    let w = Workspace::new();
    w.write(
        ".claude/skills/review/SKILL.md",
        "---\nname: review\ndescription: Review code\n---\n",
    );
    w.write(
        ".claude/settings.json",
        r#"{"skillOverrides":{"review":"off"}}"#,
    );
    w.write(
        "project/.claude/settings.json",
        r#"{"skillOverrides":{"review":"on"}}"#,
    );
    let args = ["native", "skill", "disable", "review", "--claude", "--json"];
    let output = w.run_in(&w.0.path().join("project"), &args);
    assert!(output.status.success(), "{output:?}");
    let out: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(out["changes"][0]["outcome"], "unchanged");
    assert!(
        out["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "native-skill-overridden" && d["severity"] == "warning")
    );
}

#[test]
fn native_skill_completion_follows_state_harness_and_native_id_flags() {
    let w = Workspace::new();
    w.write(
        ".claude/skills/review/SKILL.md",
        "---\nname: review\ndescription: Review code\n---\n",
    );
    w.write("user.toml", "[skills.logical]\nclaude='review'\n");
    let complete = |words: &[&str]| {
        let mut args = vec!["__complete", "--", "ayran"];
        args.extend(words);
        let output = w.run(&args);
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap()
    };
    assert!(complete(&["native", "skill", "disable", "logical"]).contains("logical"));
    assert_eq!(complete(&["native", "skill", "enable", "logical"]), "");
    assert!(complete(&["native", "skill", "disable", "--claude", "--id", "r"]).contains("review"));
    assert_eq!(
        complete(&["native", "skill", "disable", "--claude", "--id", "logical"]),
        ""
    );
    assert_eq!(
        complete(&["native", "skill", "disable", "--copilot", "r"]),
        ""
    );
    w.json(
        &[
            "native", "skill", "disable", "logical", "--claude", "--json",
        ],
        0,
    );
    assert_eq!(complete(&["native", "skill", "disable", "logical"]), "");
    assert!(complete(&["native", "skill", "enable", "logical"]).contains("logical"));
}

#[test]
fn native_skill_dry_run_never_runs_harness_and_old_versions_prevent_writes() {
    let w = Workspace::new();
    for h in ["claude", "codex"] {
        w.write(
            &format!(".{h}/skills/review/SKILL.md"),
            "---\nname: review\ndescription: Review code\n---\n",
        );
    }
    w.write(
        "user.toml",
        "[skills.review]\nclaude='review'\ncodex='review'\ncopilot=false\n",
    );
    w.write(
        "codex",
        "#!/bin/sh\necho called > codex-called\necho '0.159.0'\n",
    );
    let out = w.json(
        &[
            "native",
            "skill",
            "disable",
            "review",
            "--dry-run",
            "--json",
        ],
        0,
    );
    assert_eq!(out["changes"].as_array().unwrap().len(), 2);
    assert!(!w.0.path().join("codex-called").exists());
    let out = w.json(&["native", "skill", "disable", "review", "--json"], 3);
    assert_eq!(out["changes"], serde_json::json!([]));
    assert!(
        out["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "harness-too-old")
    );
    assert!(!w.0.path().join(".claude/settings.json").exists());
    assert!(!w.0.path().join(".codex/config.toml").exists());
}

#[test]
fn native_codex_mcp_off_skips_hides_and_selection_enables_it() {
    let w = Workspace::new();
    let original = "# preferences\nmodel='keep'\n[mcp_servers.'review.server']\ncommand='/bin/true'\nenabled=true # retain\n";
    w.write(".codex/config.toml", original);
    w.write("user.toml", "[mcp.logical]\ncodex='review.server'\n");
    let toggle = ["native", "mcp", "disable", "logical", "--codex", "--json"];
    let changed = w.json(&toggle, 0);
    assert_eq!(changed["changes"][0]["outcome"], "changed");
    let path = w.0.path().join(".codex/config.toml");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        original.replace("enabled=true", "enabled=false")
    );
    assert_eq!(w.json(&toggle, 0)["changes"][0]["outcome"], "unchanged");
    let trace = w.run(&["--codex", "--dry-run"]);
    assert!(trace.status.success(), "{trace:?}");
    assert!(
        String::from_utf8_lossy(&trace.stderr).contains("natively off"),
        "{trace:?}"
    );
    let selected = w.json(&["--codex", "--mcp", "logical", "--dry-run", "--json"], 0);
    assert!(
        selected["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("mcp_servers={\"review.server\"={enabled=true}}")),
        "{selected}"
    );
    let enable = ["native", "mcp", "enable", "logical", "--codex", "--json"];
    assert_eq!(w.json(&enable, 0)["changes"][0]["outcome"], "changed");
    assert_eq!(w.json(&enable, 0)["changes"][0]["outcome"], "unchanged");
}

#[test]
fn native_mcp_completion_filters_state_harness_and_connectors() {
    let w = Workspace::new();
    w.write("user.toml", "[mcp.logical]\ncodex='review'\ncopilot='review'\n[mcp.account]\ncodex={connector='account'}\n");
    w.write(
        ".codex/config.toml",
        "[mcp_servers.review]\ncommand='/bin/true'\nenabled=true\n",
    );
    w.write(
        ".codex/cache/codex_apps_tools/tools.json",
        r#"{"tools":[{"connector_id":"account"}]}"#,
    );
    w.write(
        ".copilot/mcp-config.json",
        r#"{"mcpServers":{"review":{"command":"/bin/true"}}}"#,
    );
    let complete = |words: &[&str]| {
        let mut args = vec!["__complete", "--", "ayran"];
        args.extend(words);
        String::from_utf8(w.run(&args).stdout).unwrap()
    };
    for flag in ["--codex", "--copilot"] {
        assert!(complete(&["native", "mcp", "disable", flag, "logical"]).contains("logical"));
        assert!(complete(&["native", "mcp", "disable", flag, "--id", "r"]).contains("review"));
        assert_eq!(complete(&["native", "mcp", "enable", flag, "logical"]), "");
        assert_eq!(
            complete(&["native", "mcp", "disable", flag, "--id", "logical"]),
            ""
        );
    }
    assert_eq!(complete(&["native", "mcp", "disable", "--claude", "r"]), "");
    assert_eq!(complete(&["native", "mcp", "disable", "--codex", "a"]), "");
    w.json(
        &["native", "mcp", "disable", "logical", "--codex", "--json"],
        0,
    );
    assert_eq!(
        complete(&["native", "mcp", "disable", "--codex", "logical"]),
        ""
    );
    assert!(complete(&["native", "mcp", "enable", "--codex", "logical"]).contains("logical"));
}

impl Workspace {
    fn copilot_mcp(&self) {
        self.write(
            ".copilot/mcp-config.json",
            r#"{"mcpServers":{"review":{"command":"/bin/true"}}}"#,
        );
        self.write(".copilot/settings.json", r#"{"theme":"dark"}"#);
        self.write(
            "copilot",
            r#"#!/usr/bin/python3
import json, os, pathlib, sys
if sys.argv[1:] == ['--version']:
    print('1.0.91')
    sys.exit(0)
root = pathlib.Path(os.environ['HOME'])
home = pathlib.Path(os.environ['COPILOT_HOME'])
assert pathlib.Path.cwd() == home
assert len(sys.argv) == 4 and sys.argv[1] == 'mcp'
assert sys.argv[2] in ['enable', 'disable']
with (root / 'mcp-calls').open('a') as log:
    log.write(json.dumps(sys.argv[1:]) + '\n')
if (root / 'copilot-fail').exists():
    print('write refused', file=sys.stderr)
    sys.exit(1)
path = home / 'settings.json'
settings = json.loads(path.read_text()) if path.exists() else {}
disabled = settings.setdefault('disabledMcpServers', [])
if sys.argv[2] == 'disable' and sys.argv[3] not in disabled:
    disabled.append(sys.argv[3])
if sys.argv[2] == 'enable' and sys.argv[3] in disabled:
    disabled.remove(sys.argv[3])
path.write_text(json.dumps(settings))
"#,
        );
    }
}

#[test]
fn native_copilot_mcp_writes_user_state_idempotently_and_selection_enables_it() {
    let w = Workspace::new();
    w.copilot_mcp();
    w.write("user.toml", "[mcp.logical]\ncopilot='review'\n");
    let disable = ["native", "mcp", "disable", "logical", "--copilot", "--json"];
    assert_eq!(w.json(&disable, 0)["changes"][0]["outcome"], "changed");
    assert_eq!(w.json(&disable, 0)["changes"][0]["outcome"], "unchanged");
    let trace = w.run(&["--copilot", "--dry-run"]);
    assert!(trace.status.success(), "{trace:?}");
    assert!(
        String::from_utf8_lossy(&trace.stderr).contains("natively off"),
        "{trace:?}"
    );
    let selected = w.json(&["--copilot", "--mcp", "logical", "--dry-run", "--json"], 0);
    assert!(
        selected["argv"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|pair| pair[0] == "--enable-mcp-server" && pair[1] == "review"),
        "{selected}"
    );
    let enable = ["native", "mcp", "enable", "logical", "--copilot", "--json"];
    assert_eq!(w.json(&enable, 0)["changes"][0]["outcome"], "changed");
    assert_eq!(w.json(&enable, 0)["changes"][0]["outcome"], "unchanged");
    let calls = fs::read_to_string(w.0.path().join("mcp-calls")).unwrap();
    assert_eq!(calls.lines().count(), 2);
    let settings: serde_json::Value =
        serde_json::from_slice(&fs::read(w.0.path().join(".copilot/settings.json")).unwrap())
            .unwrap();
    assert_eq!(settings["theme"], "dark");
    assert_eq!(settings["disabledMcpServers"], serde_json::json!([]));
    w.write(
        "project/.github/copilot/settings.json",
        r#"{"disabledMcpServers":["review"]}"#,
    );
    let overridden = w.run_in(&w.0.path().join("project"), &enable);
    assert!(overridden.status.success(), "{overridden:?}");
    let result: serde_json::Value = serde_json::from_slice(&overridden.stdout).unwrap();
    assert_eq!(result["changes"][0]["outcome"], "unchanged");
    assert!(
        result["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "native-mcp-overridden"),
        "{result}"
    );
}

#[test]
fn native_mcp_validation_and_unsupported_targets_prevent_all_writes() {
    let w = Workspace::new();
    w.copilot_mcp();
    w.write(
        ".codex/config.toml",
        "[mcp_servers.review]\ncommand='/bin/true'\n",
    );
    w.write("user.toml", "[mcp.logical]\ncodex='review'\ncopilot='review'\n[mcp.mixed]\nclaude='review'\ncodex='review'\n[mcp.account]\ncodex={connector='account'}\n");
    let original = fs::read(w.0.path().join(".codex/config.toml")).unwrap();
    for args in [
        vec![
            "native", "mcp", "disable", "review", "missing", "--codex", "--id", "--json",
        ],
        vec![
            "native",
            "mcp",
            "disable",
            "review",
            "missing",
            "--copilot",
            "--id",
            "--json",
        ],
        vec!["native", "mcp", "disable", "mixed", "--json"],
        vec![
            "native", "mcp", "disable", "logical", "account", "--codex", "--json",
        ],
        vec!["native", "mcp", "disable", "review", "--claude", "--json"],
    ] {
        let result = w.json(&args, 3);
        assert_eq!(result["changes"], serde_json::json!([]), "{result}");
        assert_eq!(
            fs::read(w.0.path().join(".codex/config.toml")).unwrap(),
            original
        );
        assert!(!w.0.path().join("mcp-calls").exists());
    }
    w.write(
        ".codex/cache/codex_apps_tools/tools.json",
        r#"{"tools":[{"connector_id":"account"}]}"#,
    );
    let connector = w.json(
        &[
            "native", "mcp", "disable", "account", "--codex", "--id", "--json",
        ],
        3,
    );
    assert_eq!(connector["diagnostics"][0]["code"], "native-unsupported");
    let usage = w.json(&["native", "mcp", "disable", "review", "--id", "--json"], 2);
    assert_eq!(usage["diagnostics"][0]["code"], "usage");
    w.write(".copilot/settings.json", r#"{"disabledMcpServers":false}"#);
    w.json(&["native", "mcp", "disable", "logical", "--json"], 3);
    assert_eq!(
        fs::read(w.0.path().join(".codex/config.toml")).unwrap(),
        original
    );
    assert!(!w.0.path().join("mcp-calls").exists());
}

#[test]
fn native_mcp_dry_run_and_isolated_home_respect_the_write_boundary() {
    let w = Workspace::new();
    w.write("user.toml", "[harnesses.codex]\nhome='isolated'\n[harnesses.copilot]\nhome='isolated'\n[mcp.project]\ncodex='review'\ncopilot='review'\n[mcp.definition]\ncodex={command='/bin/true'}\n[mcp.absent]\ncodex=false\n");
    w.write(
        "project/.codex/config.toml",
        "[mcp_servers.review]\ncommand='/bin/true'\nenabled=true\n",
    );
    w.write(
        "project/.mcp.json",
        r#"{"mcpServers":{"review":{"command":"/bin/true"}}}"#,
    );
    for h in ["codex", "copilot"] {
        w.write(h, "#!/bin/sh\nexit 99\n");
        let flag = format!("--{h}");
        let output = w.run_in(
            &w.0.path().join("project"),
            &[
                "native",
                "mcp",
                "disable",
                "project",
                &flag,
                "--dry-run",
                "--json",
            ],
        );
        assert!(output.status.success(), "{output:?}");
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["changes"][0]["outcome"], "planned");
    }
    assert!(!w.0.path().join("state").exists());
    assert!(!w.0.path().join("cache").exists());
    let skipped = w.json(
        &[
            "native",
            "mcp",
            "disable",
            "definition",
            "absent",
            "--codex",
            "--dry-run",
            "--json",
        ],
        0,
    );
    assert_eq!(skipped["changes"], serde_json::json!([]));
    assert_eq!(skipped["diagnostics"].as_array().unwrap().len(), 2);
    w.write("codex", "#!/bin/sh\necho 'codex 0.160.0'\n");
    let output = w.run_in(
        &w.0.path().join("project"),
        &["native", "mcp", "disable", "project", "--codex", "--json"],
    );
    assert!(output.status.success(), "{output:?}");
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["changes"][0]["outcome"], "changed");
    assert!(
        result["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "native-mcp-overridden"),
        "{result}"
    );
    let config: toml::Table =
        fs::read_to_string(w.0.path().join("state/ayran/homes/codex/config.toml"))
            .unwrap()
            .parse()
            .unwrap();
    assert_eq!(
        config["mcp_servers"]["review"]["enabled"].as_bool(),
        Some(false)
    );
    assert!(!w.0.path().join(".codex").exists());
}

#[test]
fn native_mcp_write_failure_retains_completed_changes_and_can_be_retried() {
    let w = Workspace::new();
    w.copilot_mcp();
    w.write(
        "user.toml",
        "[mcp.logical]\ncodex='review'\ncopilot='review'\n",
    );
    w.write(
        ".codex/config.toml",
        "[mcp_servers.review]\ncommand='/bin/true'\n",
    );
    w.write("copilot-fail", "");
    let args = ["native", "mcp", "disable", "logical", "--json"];
    let failed = w.json(&args, 3);
    assert_eq!(failed["changes"][0]["outcome"], "changed");
    assert_eq!(failed["changes"][1]["outcome"], "failed");
    assert!(
        failed["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "native-write-failed")
    );
    fs::remove_file(w.0.path().join("copilot-fail")).unwrap();
    let retry = w.json(&args, 0);
    assert_eq!(retry["changes"][0]["outcome"], "unchanged");
    assert_eq!(retry["changes"][1]["outcome"], "changed");
    w.write("copilot", "#!/bin/sh\necho 'copilot 1.0.90'\n");
    let before = fs::read(w.0.path().join(".codex/config.toml")).unwrap();
    let old = w.json(&["native", "mcp", "enable", "logical", "--json"], 3);
    assert!(
        old["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "harness-too-old")
    );
    assert_eq!(
        fs::read(w.0.path().join(".codex/config.toml")).unwrap(),
        before
    );
}

#[test]
fn doctor_suggests_native_mcp_disable_only_for_unbound_user_servers() {
    let w = Workspace::new();
    w.copilot_mcp();
    w.write(
        ".codex/config.toml",
        "[mcp_servers.review]\ncommand='/bin/true'\n",
    );
    for flag in ["--codex", "--copilot"] {
        let doctor = w.json(&["doctor", flag, "--json"], 0);
        assert!(
            doctor["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["code"] == "native-mcp-unbound"
                    && d["message"]
                        .as_str()
                        .unwrap()
                        .contains("native mcp disable review")),
            "{doctor}"
        );
        w.json(
            &["native", "mcp", "disable", "review", flag, "--id", "--json"],
            0,
        );
        let doctor = w.json(&["doctor", flag, "--json"], 0);
        assert!(
            !doctor["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["code"] == "native-mcp-unbound"),
            "{doctor}"
        );
        w.json(
            &["native", "mcp", "enable", "review", flag, "--id", "--json"],
            0,
        );
    }
    w.write(
        "user.toml",
        "[mcp.logical]\ncodex='review'\ncopilot='review'\n",
    );
    for flag in ["--codex", "--copilot"] {
        let doctor = w.json(&["doctor", flag, "--json"], 0);
        assert!(
            !doctor["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["code"] == "native-mcp-unbound"),
            "{doctor}"
        );
    }
}

#[test]
fn native_copilot_mcp_writes_to_the_isolated_launch_home() {
    let w = Workspace::new();
    w.copilot_mcp();
    w.write(
        "user.toml",
        "[harnesses.copilot]\nhome='isolated'\n[mcp.logical]\ncopilot='review'\n",
    );
    w.write(
        "state/ayran/homes/copilot/mcp-config.json",
        r#"{"mcpServers":{"review":{"command":"/bin/true"}}}"#,
    );
    let original = fs::read(w.0.path().join(".copilot/settings.json")).unwrap();
    let changed = w.json(
        &["native", "mcp", "disable", "logical", "--copilot", "--json"],
        0,
    );
    assert_eq!(changed["changes"][0]["outcome"], "changed");
    let settings: serde_json::Value = serde_json::from_slice(
        &fs::read(w.0.path().join("state/ayran/homes/copilot/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        settings["disabledMcpServers"],
        serde_json::json!(["review"])
    );
    assert_eq!(
        fs::read(w.0.path().join(".copilot/settings.json")).unwrap(),
        original
    );
    let selected = w.json(&["--copilot", "--mcp", "logical", "--dry-run", "--json"], 0);
    assert!(
        selected["env"]["COPILOT_HOME"]
            .as_str()
            .unwrap()
            .ends_with("/state/ayran/homes/copilot")
    );
}
