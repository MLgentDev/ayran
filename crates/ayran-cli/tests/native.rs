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
