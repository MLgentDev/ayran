#![cfg(unix)]
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

struct Workspace(TempDir);
impl Workspace {
    fn new() -> Self {
        let w = Self(tempfile::tempdir().unwrap());
        w.write("user.toml", "");
        w
    }
    fn write(&self, path: &str, text: &str) {
        let path = self.0.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn harness(&self, name: &str) {
        self.write(&format!("bin/{name}"), r#"#!/usr/bin/python3
import json, os, pathlib, sys
args = sys.argv[1:]
name = pathlib.Path(sys.argv[0]).name
if args == ['--version']:
    print({'claude':'2.1.288','codex':'0.160.0','copilot':'1.0.91'}[name]); sys.exit(0)
home = pathlib.Path(os.environ[{'claude':'CLAUDE_CONFIG_DIR','codex':'CODEX_HOME','copilot':'COPILOT_HOME'}[name]])
assert pathlib.Path.cwd() == home
root = pathlib.Path(os.environ['HOME'])
with (root/'calls').open('a') as f: f.write(json.dumps(args)+'\n')
if (root/'fail').exists() and (root/'fail').read_text() == args[1]:
    print('SECRET fixture error', file=sys.stderr); sys.exit(1)
if args[:2] == ['mcp','disable']:
    path = home/'settings.json'
    value = json.loads(path.read_text()) if path.exists() else {}
    value.setdefault('disabledMcpServers',[]).append(args[2])
    path.write_text(json.dumps(value)); sys.exit(0)
assert args[:2] == ['mcp','add'], args
args = args[2:]
if name == 'claude':
    assert args[:2] == ['--scope','user']; args = args[2:]
transport = 'stdio'
env = {}; headers = {}
while args[0].startswith('--'):
    flag, value = args[:2]; args = args[2:]
    if flag == '--transport': transport = value
    elif flag == '--env':
        key, value = value.split('=',1); env[key] = value
    elif flag == '--header':
        key, value = value.split(':',1); headers[key] = value.strip()
    else: raise AssertionError(flag)
id = args.pop(0)
if name == 'codex':
    path = home/'config.toml'
    text = path.read_text() if path.exists() else ''
    text += '\n[mcp_servers.'+id+']\n'
    if args[0] == '--url':
        text += 'url='+json.dumps(args[1])+'\n'
        if len(args)>2: text += 'bearer_token_env_var='+json.dumps(args[3])+'\n'
    else:
        assert args.pop(0) == '--'
        text += 'command='+json.dumps(args.pop(0))+'\nargs='+json.dumps(args)+'\n'
        if env: text += 'env='+json.dumps(env).replace(':',' =')+'\n'
    path.write_text(text); sys.exit(0)
if transport == 'http': value = {'type':'http','url':args[0],'headers':headers}
else:
    assert args.pop(0) == '--'
    value = {'command':args.pop(0),'args':args,'env':env}
    if name == 'copilot': value['type'] = 'local'
if name == 'copilot': value['tools'] = ['*']
path = home/('.claude.json' if name == 'claude' else 'mcp-config.json')
config = json.loads(path.read_text()) if path.exists() else {}
assert id not in config.get('mcpServers',{})
config.setdefault('mcpServers',{})[id] = value
path.write_text(json.dumps(config))
"#);
        fs::set_permissions(
            self.0.path().join(format!("bin/{name}")),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    fn run_in(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ayran"))
            .current_dir(cwd)
            .args(args)
            .env("HOME", self.0.path())
            .env("USERPROFILE", self.0.path())
            .env("AYRAN_CONFIG", self.0.path().join("user.toml"))
            .env("XDG_STATE_HOME", self.0.path().join("state"))
            .env("XDG_CACHE_HOME", self.0.path().join("cache"))
            .env("PATH", self.0.path().join("bin"))
            .env("CLAUDE_CONFIG_DIR", self.0.path().join(".claude"))
            .env_remove("CODEX_HOME")
            .env_remove("COPILOT_HOME")
            .env("TOKEN", "SECRET")
            .output()
            .unwrap()
    }
    fn run(&self, args: &[&str]) -> Output {
        self.run_in(self.0.path(), args)
    }
    fn json(&self, args: &[&str], code: i32) -> Value {
        let output = self.run(args);
        assert_eq!(output.status.code(), Some(code), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.0.path().join(path)).unwrap()
    }
}

#[test]
fn codex_installs_complete_definition_off_and_repeated_install_is_unchanged() {
    let w = Workspace::new();
    w.harness("codex");
    w.write(
        "user.toml",
        r#"[mcp.api]
codex = { command = "node", args = ["server.js"], env = { MODE = "literal" }, env_vars = ["TOKEN"] }
"#,
    );
    w.write(".codex/config.toml", "# keep\nmodel = 'example'\n");
    let output = w.json(&["install", "--mcp", "api", "--codex", "--json"], 0);
    assert_eq!(output["mcp"][0]["outcome"], "installed");
    let config: toml::Table = w.read(".codex/config.toml").parse().unwrap();
    assert_eq!(
        config["mcp_servers"]["api"]["env_vars"].as_array().unwrap()[0].as_str(),
        Some("TOKEN")
    );
    assert_eq!(
        config["mcp_servers"]["api"]["enabled"].as_bool(),
        Some(false)
    );
    assert!(w.read(".codex/config.toml").contains("# keep"));
    assert!(!output.to_string().contains("SECRET"));
    assert_eq!(
        w.json(&["install", "--mcp", "api", "--codex", "--json"], 0)["mcp"][0]["outcome"],
        "unchanged"
    );
}

#[test]
fn project_definition_trust_tracks_sources_and_preserves_unrelated_edits() {
    let w = Workspace::new();
    w.harness("codex");
    let original =
        "[mcp.api]\ncodex = { url = 'https://example.com/mcp', bearer_token_env = 'TOKEN' }\n";
    w.write("project/ayran.toml", original);
    let cwd = w.0.path().join("project");
    let output = w.run_in(&cwd, &["install", "--mcp", "api", "--codex", "--json"]);
    assert_eq!(output.status.code(), Some(3));
    assert!(!w.0.path().join(".codex").exists());
    assert!(!w.0.path().join("state").exists());
    assert!(w.run_in(&cwd, &["trust"]).status.success());
    w.write(
        "project/ayran.toml",
        &format!("{original}description = 'unrelated'\n[skills.other]\nall = false\n"),
    );
    let output = w.run_in(
        &cwd,
        &["install", "--mcp", "api", "--codex", "--dry-run", "--json"],
    );
    assert!(output.status.success(), "{output:?}");
    assert!(!w.0.path().join(".codex").exists());
    w.write(
        "project/ayran.toml",
        &original.replace("example.com", "different.example"),
    );
    let output = w.run_in(&cwd, &["install", "--mcp", "api", "--codex", "--json"]);
    assert_eq!(
        output.status.code(),
        Some(3),
        "changed definition must revoke Trust: {output:?}"
    );
    assert!(!w.0.path().join(".codex").exists());
}

#[test]
fn installed_definition_selection_overrides_native_off_without_writing() {
    for harness in ["codex", "copilot"] {
        let w = Workspace::new();
        w.harness(harness);
        w.write(
            "user.toml",
            &format!("[mcp.api]\n{harness} = {{ url = 'https://example.com/mcp' }}\n"),
        );
        let flag = format!("--{harness}");
        w.json(&["install", "--mcp", "api", &flag, "--json"], 0);
        let before = w.read(if harness == "codex" {
            ".codex/config.toml"
        } else {
            ".copilot/settings.json"
        });
        let output = w.json(&[&flag, "--mcp", "api", "--dry-run", "--json"], 0);
        let argv = output["argv"].to_string();
        if harness == "codex" {
            assert!(argv.contains("enabled=true"), "{output}");
        } else {
            assert!(argv.contains("--enable-mcp-server"), "{output}");
        }
        assert_eq!(
            w.read(if harness == "codex" {
                ".codex/config.toml"
            } else {
                ".copilot/settings.json"
            }),
            before
        );
        assert!(!w.0.path().join("cache").exists());
    }
}

#[test]
fn stdio_and_http_fields_remain_literal_or_references_on_every_harness() {
    for harness in ["claude", "codex", "copilot"] {
        let w = Workspace::new();
        w.harness(harness);
        w.write("user.toml",r#"[mcp.stdio]
all = { command = "node", args = ["literal;$(TOKEN)", "space arg"], env = { MODE = "LITERAL_PRIVATE" }, env_vars = ["TOKEN"] }
[mcp.http]
all = { url = "https://example.com/mcp", headers = { X-Literal = "HEADER_PRIVATE" }, env_headers = { X-Token = "TOKEN" }, bearer_token_env = "TOKEN" }
"#);
        let flag = format!("--{harness}");
        let output = w.json(&["install", "--mcp", "stdio", "http", &flag, "--json"], 0);
        for secret in ["SECRET", "LITERAL_PRIVATE", "HEADER_PRIVATE"] {
            assert!(!output.to_string().contains(secret));
        }
        let (stdio, http) = if harness == "codex" {
            let value: toml::Table = w.read(".codex/config.toml").parse().unwrap();
            let config = serde_json::to_value(value).unwrap();
            (
                config["mcp_servers"]["stdio"].clone(),
                config["mcp_servers"]["http"].clone(),
            )
        } else {
            let config: Value = serde_json::from_str(&w.read(if harness == "claude" {
                ".claude/.claude.json"
            } else {
                ".copilot/mcp-config.json"
            }))
            .unwrap();
            (
                config["mcpServers"]["stdio"].clone(),
                config["mcpServers"]["http"].clone(),
            )
        };
        assert_eq!(
            stdio["args"],
            serde_json::json!(["literal;$(TOKEN)", "space arg"])
        );
        assert_eq!(stdio["env"]["MODE"], "LITERAL_PRIVATE");
        if harness == "codex" {
            assert_eq!(stdio["env_vars"], serde_json::json!(["TOKEN"]));
            assert_eq!(http["env_http_headers"]["X-Token"], "TOKEN");
            assert_eq!(http["bearer_token_env_var"], "TOKEN");
            assert_eq!(http["http_headers"]["X-Literal"], "HEADER_PRIVATE");
        } else {
            assert_eq!(stdio["env"]["TOKEN"], "${TOKEN}");
            assert_eq!(http["headers"]["X-Token"], "${TOKEN}");
            assert_eq!(http["headers"]["Authorization"], "Bearer ${TOKEN}");
            assert_eq!(http["headers"]["X-Literal"], "HEADER_PRIVATE");
        }
        if harness == "claude" {
            assert!(output["diagnostics"].to_string().contains("mcp-install-on"));
        }
        assert_eq!(
            w.json(&["install", "--mcp", "stdio", "http", &flag, "--json"], 0)["mcp"][0]["outcome"],
            "unchanged"
        );
    }
}

#[test]
fn relative_commands_use_the_declaring_directory_and_isolated_launch_home() {
    for harness in ["claude", "codex", "copilot"] {
        let w = Workspace::new();
        w.harness(harness);
        w.write(
            "user.toml",
            &format!("[harnesses.{harness}]\nhome = 'isolated'\n"),
        );
        w.write(
            "project/ayran.toml",
            "[mcp.api]\nall = { command = './server', args = ['one'] }\n",
        );
        w.write("project/server", "fixture");
        w.write("project/sub/.keep", "");
        let cwd = w.0.path().join("project/sub");
        let flag = format!("--{harness}");
        assert!(w.run_in(&cwd, &["trust"]).status.success());
        let output = w.run_in(
            &cwd,
            &["install", "--mcp", "api", &flag, "--dry-run", "--json"],
        );
        assert!(output.status.success(), "{output:?}");
        assert!(!w.0.path().join("state/ayran/homes").exists());
        let output = w.run_in(&cwd, &["install", "--mcp", "api", &flag, "--json"]);
        assert!(output.status.success(), "{output:?}");
        let home = format!("state/ayran/homes/{harness}");
        let config = w.read(&format!(
            "{home}/{}",
            if harness == "codex" {
                "config.toml"
            } else if harness == "claude" {
                ".claude.json"
            } else {
                "mcp-config.json"
            }
        ));
        assert!(
            config.contains(w.0.path().join("project/server").to_str().unwrap()),
            "{config}"
        );
        assert!(!w.0.path().join(format!(".{harness}")).exists());
    }
}

#[test]
fn conflicts_and_invalid_targets_preflight_all_selected_definitions_before_writes() {
    for harness in ["claude", "codex", "copilot"] {
        let w = Workspace::new();
        w.harness(harness);
        w.write("user.toml","[mcp.api]\nall = { url = 'https://example.com/mcp' }\n[mcp.fresh]\nall = { command = 'node' }\n");
        let file = if harness == "codex" {
            ".codex/config.toml"
        } else if harness == "claude" {
            ".claude/.claude.json"
        } else {
            ".copilot/mcp-config.json"
        };
        let before = if harness == "codex" {
            "# keep\n[mcp_servers.api]\nurl = 'https://different.example/mcp'\nenabled = false\n"
        } else {
            r#"{"unknown":true,"mcpServers":{"api":{"type":"http","url":"https://different.example/mcp"}}}"#
        };
        w.write(file, before);
        let flag = format!("--{harness}");
        let output = w.json(&["install", "--mcp", "fresh", "api", &flag, "--json"], 3);
        assert!(
            output["diagnostics"]
                .to_string()
                .contains("mcp-install-conflict")
        );
        assert_eq!(w.read(file), before);
        assert!(!w.0.path().join("calls").exists());
        w.json(
            &["install", "--mcp", "fresh", "unknown", &flag, "--json"],
            3,
        );
        assert_eq!(w.read(file), before);
        assert!(!w.0.path().join("calls").exists());
        w.write("user.toml","[mcp.fresh]\nall = { url = 'https://example.com', headers = { Authorization = 'HEADER_PRIVATE' }, bearer_token_env = 'TOKEN' }\n");
        let output = w.json(&["install", "--mcp", "fresh", &flag, "--json"], 3);
        assert!(
            output["diagnostics"]
                .to_string()
                .contains("unsupported-binding")
        );
        assert!(!output.to_string().contains("HEADER_PRIVATE"));
        assert_eq!(w.read(file), before);
    }
}

#[test]
fn copilot_disable_failure_retains_added_definition_and_retry_only_disables() {
    let w = Workspace::new();
    w.harness("copilot");
    w.write("user.toml","[mcp.api]\ncopilot = { url = 'https://example.com' }\n[mcp.later]\ncopilot = { command = 'node' }\n");
    w.write("fail", "disable");
    let output = w.json(
        &["install", "--mcp", "api", "later", "--copilot", "--json"],
        3,
    );
    assert_eq!(output["mcp"][0]["add"], "added");
    assert_eq!(output["mcp"][0]["disable"], "failed");
    assert_eq!(output["mcp"][1]["outcome"], "skipped");
    assert!(!output.to_string().contains("SECRET"));
    fs::remove_file(w.0.path().join("fail")).unwrap();
    let output = w.json(&["install", "--mcp", "api", "--copilot", "--json"], 0);
    assert_eq!(output["mcp"][0]["add"], "unchanged");
    assert_eq!(output["mcp"][0]["disable"], "changed");
    let calls: Vec<Value> = w
        .read("calls")
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(calls.iter().filter(|c| c[1] == "add").count(), 1);
    assert_eq!(
        w.json(&["install", "--mcp", "api", "--copilot", "--json"], 0)["mcp"][0]["outcome"],
        "unchanged"
    );
}

#[test]
fn doctor_suggests_definition_install_and_accepts_matching_installed_definition() {
    let w = Workspace::new();
    w.harness("codex");
    w.write(
        "user.toml",
        "[mcp.api]\ncodex = { url = 'https://example.com' }\n",
    );
    let output = w.json(&["doctor", "--codex", "--json"], 0);
    assert!(
        output["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["hint"]
                .as_str()
                .is_some_and(|s| s.contains("install --mcp api"))),
        "{output}"
    );
    w.json(&["install", "--mcp", "api", "--codex", "--json"], 0);
    let output = w.json(&["doctor", "--codex", "--json"], 0);
    assert!(
        !output["diagnostics"]
            .to_string()
            .contains("mcp-definition-collision"),
        "{output}"
    );
}

#[test]
fn install_completion_offers_logical_mcp_definitions_independent_of_launch_defaults() {
    let w = Workspace::new();
    w.write("user.toml","default_harness = 'claude'\n[mcp.api]\ncodex = { url = 'https://example.com' }\n[mcp.absent]\nall = false\n");
    let output = w.run(&["__complete", "--", "ayran", "install", "--mcp", ""]);
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|l| l == "api"),
        "{output:?}"
    );
}

#[test]
fn native_bindings_verify_without_writes_and_absent_or_connector_bindings_skip() {
    let w = Workspace::new();
    w.harness("codex");
    w.write(
        "user.toml",
        r#"[mcp.native]
codex = "existing"
[mcp.absent]
codex = false
[mcp.connector]
codex = { connector = "account" }
[mcp.definition]
codex = { url = "https://example.com" }
"#,
    );
    let before = "[mcp_servers.existing]\ncommand = 'node'\nenabled = true\n";
    w.write(".codex/config.toml", before);
    let output = w.json(
        &[
            "install",
            "--mcp",
            "native",
            "absent",
            "connector",
            "--codex",
            "--json",
        ],
        0,
    );
    assert_eq!(output["mcp"][0]["outcome"], "skipped");
    assert_eq!(output["mcp"][1]["outcome"], "skipped");
    assert_eq!(output["mcp"][2]["outcome"], "unchanged");
    assert_eq!(w.read(".codex/config.toml"), before);
    assert!(!w.0.path().join("calls").exists());
    let output = w.json(&["install", "--codex", "--json"], 0);
    assert_eq!(output["mcp"], serde_json::json!([]));
    assert_eq!(w.read(".codex/config.toml"), before);
}

#[test]
fn project_and_plugin_collisions_and_unknown_user_fields_are_never_adopted() {
    let w = Workspace::new();
    w.harness("codex");
    w.write(
        "user.toml",
        "[mcp.api]\ncodex = { url = 'https://example.com' }\n",
    );
    for (file, contents) in [
        (
            ".codex/config.toml",
            "[mcp_servers.api]\nurl = 'https://example.com'\nstartup_timeout_sec = 20\n",
        ),
        (".codex/config.toml", "[plugins.'test@m']\nenabled = true\n"),
    ] {
        w.write(file, contents);
        if contents.contains("plugins") {
            w.write(
                ".codex/plugins/cache/m/test/1/.codex-plugin/plugin.json",
                "{\"name\":\"test\",\"mcpServers\":\".mcp.json\"}",
            );
            w.write(
                ".codex/plugins/cache/m/test/1/.mcp.json",
                "{\"mcpServers\":{\"api\":{\"url\":\"https://example.com\"}}}",
            );
        }
        let output = w.json(&["install", "--mcp", "api", "--codex", "--json"], 3);
        assert!(
            output["diagnostics"]
                .to_string()
                .contains("mcp-install-conflict"),
            "{output}"
        );
        assert_eq!(w.read(file), contents);
        assert!(!w.0.path().join("calls").exists());
    }
    fs::remove_dir_all(w.0.path().join(".codex/plugins")).unwrap();
    w.write(".codex/config.toml", "");
    w.write("project/.git/.keep", "");
    w.write(
        "project/.codex/config.toml",
        "[mcp_servers.api]\nurl = 'https://example.com'\n",
    );
    let output = w.run_in(
        &w.0.path().join("project"),
        &["install", "--mcp", "api", "--codex", "--json"],
    );
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("mcp-install-conflict"));
    assert_eq!(w.read(".codex/config.toml"), "");
}

#[test]
fn failed_add_reports_its_failed_step_without_echoing_native_output() {
    let w = Workspace::new();
    w.harness("copilot");
    w.write(
        "user.toml",
        "[mcp.api]\ncopilot = { url = 'https://example.com' }\n",
    );
    w.write("fail", "add");
    let output = w.json(&["install", "--mcp", "api", "--copilot", "--json"], 3);
    assert_eq!(output["mcp"][0]["add"], "failed");
    assert!(!output.to_string().contains("SECRET"));
}

#[test]
fn malformed_arguments_fail_before_any_harness_write_even_with_codex_file_mapping() {
    let w = Workspace::new();
    w.harness("codex");
    w.write(
        "user.toml",
        r#"[mcp.good]
codex = { url = "https://example.com" }
[mcp.bad]
codex = { command = "node", args = ["bad\u0000arg"], env_vars = ["TOKEN"] }
"#,
    );
    w.json(&["install", "--mcp", "good", "bad", "--codex", "--json"], 3);
    assert!(!w.0.path().join(".codex").exists());
    assert!(!w.0.path().join("calls").exists());
}

#[test]
fn http_header_names_cannot_overlap_case_insensitively_before_install() {
    for harness in ["claude", "codex", "copilot"] {
        let w = Workspace::new();
        w.harness(harness);
        w.write("user.toml",r#"[mcp.api]
all = { url = "https://example.com", headers = { X-Token = "literal" }, env_headers = { x-token = "TOKEN" } }
"#);
        let flag = format!("--{harness}");
        w.json(&["install", "--mcp", "api", &flag, "--json"], 3);
        assert!(!w.0.path().join(format!(".{harness}")).exists());
        assert!(!w.0.path().join("calls").exists());
    }
}

#[test]
fn unknown_enabled_field_on_claude_or_copilot_is_a_definition_conflict() {
    for harness in ["claude", "copilot"] {
        let w = Workspace::new();
        w.harness(harness);
        w.write(
            "user.toml",
            &format!("[mcp.api]\n{harness} = {{ url = 'https://example.com' }}\n"),
        );
        let path = if harness == "claude" {
            ".claude/.claude.json"
        } else {
            ".copilot/mcp-config.json"
        };
        let original =
            r#"{"mcpServers":{"api":{"type":"http","url":"https://example.com","enabled":false}}}"#;
        w.write(path, original);
        let flag = format!("--{harness}");
        let output = w.json(&["install", "--mcp", "api", &flag, "--json"], 3);
        assert!(
            output["diagnostics"]
                .to_string()
                .contains("mcp-install-conflict")
        );
        assert_eq!(w.read(path), original);
        assert!(!w.0.path().join("calls").exists());
    }
}

#[test]
fn semantic_comparison_ignores_reference_order_and_preserves_unknown_empty_fields() {
    let w = Workspace::new();
    w.harness("codex");
    w.write(
        "user.toml",
        "[mcp.api]\ncodex = { command = 'node', env_vars = ['FIRST','SECOND'] }\n",
    );
    let original =
        "[mcp_servers.api]\ncommand = 'node'\nenv_vars = ['SECOND','FIRST']\nenabled = false\n";
    w.write(".codex/config.toml", original);
    assert_eq!(
        w.json(&["install", "--mcp", "api", "--codex", "--json"], 0)["mcp"][0]["outcome"],
        "unchanged"
    );
    assert_eq!(w.read(".codex/config.toml"), original);
    w.write(".codex/config.toml", &format!("{original}headers = {{}}\n"));
    let output = w.json(&["install", "--mcp", "api", "--codex", "--json"], 3);
    assert!(
        output["diagnostics"]
            .to_string()
            .contains("mcp-install-conflict")
    );
    assert!(!w.0.path().join("calls").exists());
}
