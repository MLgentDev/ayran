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
    fn write(&self, path: &str, contents: &str) {
        let path = self.0.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    fn harness(&self, name: &str, version: &str) {
        self.write(&format!("bin/{name}"), &format!("#!/bin/sh\nif [ \"$1\" = --version ]; then echo '{name} {version}'; else exit 99; fi\n"));
        fs::set_permissions(
            self.0.path().join(format!("bin/{name}")),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    fn claude_writer(&self) {
        self.harness("claude", "2.1.288");
        self.write("bin/claude", r#"#!/usr/bin/python3
import json, os, pathlib, sys
args = sys.argv[1:]
if args == ['--version']:
    print('2.1.288')
    sys.exit(0)
root = pathlib.Path(os.environ['HOME'])
home = pathlib.Path(os.environ['CLAUDE_CONFIG_DIR'])
assert pathlib.Path.cwd() == home
with (root / 'claude-calls.jsonl').open('a') as calls:
    calls.write(json.dumps({'args': args, 'home': str(home)}) + '\n')
action = 'marketplace' if args[:3] == ['plugin', 'marketplace', 'add'] else args[1]
fail = root / 'claude-fail.json'
if fail.exists() and json.loads(fail.read_text()) == action:
    print(json.dumps({'success': False, 'failureCode': 'fixture_refused'}))
    sys.exit(1)
def read(path, default):
    return json.loads(path.read_text()) if path.exists() else default
if action == 'marketplace':
    assert args[4:] == ['--scope', 'user', '--json']
    value = args[3]
    value, sep, ref = value.partition('#')
    if value.startswith('/'):
        source = {'source': 'directory', 'path': value}
        name = read(pathlib.Path(value) / '.claude-plugin/marketplace.json', {'name': 'acme'})['name']
    elif value.startswith(('https://', 'ssh://', 'git@')):
        source = {'source': 'git', 'url': value}
        name = 'acme'
    else:
        source = {'source': 'github', 'repo': value}
        name = 'acme'
    if sep:
        source['ref'] = ref
    path = home / 'plugins/known_marketplaces.json'
    markets = read(path, {})
    markets[name] = {'source': source}
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(markets))
elif action == 'install':
    assert args[3:] == ['--scope', 'user', '--json']
    id = args[2]
    path = home / 'plugins/installed_plugins.json'
    registry = read(path, {'version': 2, 'plugins': {}})
    registry['plugins'][id] = [{'scope': 'user', 'installPath': 'unused'}]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(registry))
    path = home / 'settings.json'
    settings = read(path, {})
    settings.setdefault('enabledPlugins', {})[id] = True
    path.write_text(json.dumps(settings))
elif action == 'disable':
    assert args[2:5] == ['--scope', 'user', '--json']
    assert len(args) == 6
    path = home / 'settings.json'
    settings = read(path, {})
    enabled = settings.setdefault('enabledPlugins', {})
    if enabled.get(args[5]) is False:
        print(json.dumps({'failureCode': 'already_in_goal_state'}))
        sys.exit(1)
    enabled[args[5]] = False
    path.write_text(json.dumps(settings))
else:
    raise AssertionError(args)
print(json.dumps({'success': True}))
"#);
    }
    fn copilot_writer(&self) {
        self.harness("copilot", "1.0.91");
        self.write("bin/copilot", r#"#!/usr/bin/python3
import json, os, pathlib, sys
args = sys.argv[1:]
if args == ['--version']:
    print('1.0.91')
    sys.exit(0)
root = pathlib.Path(os.environ['HOME'])
home = pathlib.Path(os.environ['COPILOT_HOME'])
assert pathlib.Path.cwd() == home
with (root / 'copilot-calls.jsonl').open('a') as calls:
    calls.write(json.dumps({'args': args, 'home': str(home), 'cache': os.environ.get('COPILOT_CACHE_HOME')}) + '\n')
action = 'marketplace' if args[:3] == ['plugin', 'marketplace', 'add'] else args[1]
fail = root / 'copilot-fail.json'
if fail.exists() and json.loads(fail.read_text()) == action:
    print('fixture refused', file=sys.stderr)
    sys.exit(1)
def read(path, default):
    return json.loads(path.read_text()) if path.exists() else default
settings_path = home / 'settings.json'
settings = read(settings_path, {})
if action == 'marketplace':
    assert len(args) == 4
    value, sep, ref = args[3].partition('#')
    name = 'acme'
    if value.startswith('/'):
        source = {'source': 'directory', 'path': value}
        for manifest in ['marketplace.json', '.plugin/marketplace.json', '.github/plugin/marketplace.json', '.claude-plugin/marketplace.json']:
            candidate = pathlib.Path(value) / manifest
            if candidate.exists():
                name = json.loads(candidate.read_text())['name']
                break
    elif value.startswith(('https://', 'ssh://', 'git@')):
        source = {'source': 'git', 'url': value}
    else:
        source = {'source': 'github', 'repo': value}
    if sep:
        source['ref'] = ref
    markets = settings.setdefault('extraKnownMarketplaces', {})
    if name in markets:
        print('Marketplace already registered', file=sys.stderr)
        sys.exit(1)
    markets[name] = {'source': source}
    settings_path.write_text(json.dumps(settings))
elif action == 'install':
    assert len(args) == 3
    id = args[2]
    name, market = id.split('@')
    path = home / 'config.json'
    config = read(path, {})
    source = settings['extraKnownMarketplaces'][market]['source']
    if source['source'] != 'directory':
        cache = home / 'installed-plugins' / market / name
        cache.mkdir(parents=True, exist_ok=True)
        (cache / 'plugin.json').write_text('{}')
        config.setdefault('installedPlugins', []).append({'name': name, 'marketplace': market, 'version': '1.0', 'cache_path': str(cache), 'source_sha': 'fixture', 'enabled': True})
    config.setdefault('enabledPlugins', {})[id] = True
    path.write_text(json.dumps(config))
elif action == 'disable':
    assert len(args) == 3
    path = home / 'config.json'
    config = read(path, {})
    config.setdefault('enabledPlugins', {})[args[2]] = False
    for plugin in config.get('installedPlugins', []):
        if plugin['name'] + '@' + plugin['marketplace'] == args[2]:
            plugin['enabled'] = False
    path.write_text(json.dumps(config))
else:
    raise AssertionError(args)
print('Success')
"#);
    }
    fn copilot_calls(&self) -> Vec<Value> {
        fs::read_to_string(self.0.path().join("copilot-calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn codex_writer(&self) {
        self.harness("codex", "0.160.0");
        self.write("bin/codex", r#"#!/usr/bin/python3
import json, os, pathlib, sys
args = sys.argv[1:]
if args == ['--version']:
    print('codex-cli 0.160.0')
    sys.exit(0)
root = pathlib.Path(os.environ['HOME'])
home = pathlib.Path(os.environ['CODEX_HOME'])
assert pathlib.Path.cwd() == home
with (root / 'codex-calls.jsonl').open('a') as calls:
    calls.write(json.dumps({'args': args, 'home': str(home)}) + '\n')
path = home / 'config.toml'
config = path.read_text() if path.exists() else '# preserved setting\nmodel = "fixture"\n'
if args[:3] == ['plugin', 'marketplace', 'add']:
    assert args[-1] == '--json'
    source = args[3]
    ref = args[5] if args[4:5] == ['--ref'] else None
    name = 'acme'
    if source.startswith('/'):
        kind = 'local'
        for manifest in ['.agents/plugins/marketplace.json', '.agents/plugins/api_marketplace.json', '.claude-plugin/marketplace.json']:
            candidate = pathlib.Path(source) / manifest
            if candidate.exists():
                name = json.loads(candidate.read_text())['name']
                break
    elif source.startswith(('https://', 'ssh://', 'git@')):
        kind = 'git'
    else:
        # Codex 0.160.0 records GitHub shorthand as a git URL.
        kind = 'git'
        source = 'https://github.com/' + source + '.git'
    config += '\n[marketplaces.' + name + ']\nsource_type=' + json.dumps(kind) + '\nsource=' + json.dumps(source) + '\n'
    if ref:
        config += 'ref=' + json.dumps(ref) + '\n'
elif args[:2] == ['plugin', 'add']:
    assert args[3:] == ['--json']
    id = args[2]
    name, market = id.split('@')
    cache = home / 'plugins/cache' / market / name / '1.0'
    cache.mkdir(parents=True, exist_ok=True)
    (cache / 'plugin.json').write_text('{}')
    config += '\n[plugins.' + json.dumps(id) + ']\nenabled=true\n'
else:
    raise AssertionError(args)
path.write_text(config)
print('{}')
"#);
    }
    fn codex_calls(&self) -> Vec<Value> {
        fs::read_to_string(self.0.path().join("codex-calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn claude_calls(&self) -> Vec<Value> {
        fs::read_to_string(self.0.path().join("claude-calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn run(&self, args: &[&str]) -> Output {
        self.run_in(self.0.path(), args)
    }
    fn run_in(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command(cwd).args(args).output().unwrap()
    }
    fn command(&self, cwd: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ayran"));
        command
            .current_dir(cwd)
            .env("HOME", self.0.path())
            .env("USERPROFILE", self.0.path())
            .env("AYRAN_CONFIG", self.0.path().join("user.toml"))
            .env("XDG_STATE_HOME", self.0.path().join("state"))
            .env("XDG_CACHE_HOME", self.0.path().join("cache"))
            .env("PATH", self.0.path().join("bin"))
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env_remove("COPILOT_HOME");
        command
    }
    fn json(&self, args: &[&str], status: i32) -> Value {
        let output = self.run(args);
        assert_eq!(output.status.code(), Some(status), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
fn codes(output: &Value) -> Vec<&str> {
    output["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect()
}

#[test]
fn marketplace_list_resolves_paths_and_per_harness_names() {
    let w = Workspace::new();
    w.write(
        "ayran.toml",
        r#"
[marketplaces.acme]
all = { source = { path = "plugins" }, ref = "v1" }
codex = { source = "github:acme/agents", ref = "v2", name = "acme-agents" }
claude = false
"#,
    );
    let output = w.json(&["list", "marketplaces", "--json"], 0);
    let row = &output["marketplaces"][0];
    assert_eq!(row["name"], "acme");
    assert_eq!(row["trusted"], false);
    assert_eq!(
        row["layer"],
        w.0.path().join("ayran.toml").to_str().unwrap()
    );
    assert_eq!(
        row["bindings"]["copilot"]["source"],
        w.0.path().join("plugins").to_str().unwrap()
    );
    assert_eq!(row["bindings"]["copilot"]["ref"], "v1");
    assert_eq!(row["bindings"]["codex"]["source"], "github:acme/agents");
    assert_eq!(row["bindings"]["codex"]["name"], "acme-agents");
    assert_eq!(row["bindings"]["claude"]["kind"], "absent");
    let output = w.json(&["list", "marketplaces", "--codex", "--json"], 0);
    assert_eq!(
        output["marketplaces"][0]["bindings"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    let text = w.run(&["list", "marketplaces", "--codex"]);
    assert!(text.status.success());
    assert!(String::from_utf8_lossy(&text.stdout).contains("acme-agents"));
}

#[test]
fn nearer_marketplace_replaces_all_bindings_and_invalid_sources_are_rejected() {
    let w = Workspace::new();
    w.write("user.toml", "[marketplaces.acme]\nall={source='github:acme/old',ref='v1'}\ncodex={source='github:acme/codex'}\n");
    w.write(
        "ayran.toml",
        "[marketplaces.acme]\nclaude={source='ssh://git@example.com/acme/agents',ref='main'}\n",
    );
    let output = w.json(&["list", "marketplaces", "--json"], 0);
    assert!(output["marketplaces"][0]["bindings"]["codex"].is_null());
    assert!(output["marketplaces"][0]["bindings"]["copilot"].is_null());
    assert_eq!(
        output["marketplaces"][0]["bindings"]["claude"]["source"],
        "ssh://git@example.com/acme/agents"
    );
    for invalid in [
        "all=true",
        "all={source='github:acme'}",
        "all={source='file:///tmp/plugins'}",
        "all={source='https://'}",
        "all={source='git@'}",
        "all={source='https://example.com/repo',ref=1}",
        "all={source='github:acme/repo',name='alias'}",
        "all={source={path='plugins',extra=true}}",
        "claude={source='github:acme/repo',name='bad@name'}",
    ] {
        w.write("ayran.toml", &format!("[marketplaces.acme]\n{invalid}\n"));
        let output = w.json(&["list", "marketplaces", "--json"], 3);
        assert_eq!(codes(&output), vec!["config-invalid"], "{invalid}");
    }
}

#[test]
fn trust_survives_capability_edits_but_source_ref_and_name_changes_require_trust_again() {
    let w = Workspace::new();
    let config =
        "[marketplaces.acme]\ncodex={source='github:acme/agents',ref='v1',name='acme-agents'}\n";
    w.write("ayran.toml", config);
    assert!(w.run(&["trust"]).status.success());
    let trusted =
        || w.json(&["list", "marketplaces", "--json"], 0)["marketplaces"][0]["trusted"].clone();
    assert_eq!(trusted(), true);
    w.write(
        "ayran.toml",
        &format!("# edited formatting\n{config}\n[plugins.review]\nall='review@acme-agents'\n"),
    );
    assert_eq!(trusted(), true);
    for changed in [
        config.replace("acme/agents", "acme/other"),
        config.replace("v1", "v2"),
        config.replace("acme-agents", "new-name"),
    ] {
        w.write("ayran.toml", &changed);
        assert_eq!(trusted(), false);
        let list = w.run(&["trust", "--list"]);
        assert!(list.status.success());
        assert!(String::from_utf8_lossy(&list.stdout).contains("false"));
    }
    w.write("ayran.toml", config);
    assert_eq!(trusted(), true);
    let path = w.0.path().join("ayran.toml");
    assert!(
        w.run(&["trust", "--revoke", path.to_str().unwrap()])
            .status
            .success()
    );
    assert_eq!(trusted(), false);
    assert!(w.run(&["trust", path.to_str().unwrap()]).status.success());
    assert_eq!(trusted(), true);
    // Revocation must still work after a layer has been deleted.
    fs::remove_file(path).unwrap();
    let output = w.run(&["trust", "--revoke", "ayran.toml"]);
    assert!(output.status.success(), "{output:?}");
    let output = w.run(&["trust", "--list"]);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("ayran.toml"));
}

#[test]
fn dry_run_plans_all_declarations_but_named_install_only_plans_required_marketplaces() {
    let w = Workspace::new();
    w.harness("codex", "0.160.0");
    w.write(
        "user.toml",
        r#"
[harnesses.codex]
home='isolated'
[marketplaces.acme]
codex={source='github:acme/agents',ref='v1',name='acme-agents'}
[marketplaces.other]
all={source='https://example.com/other.git',ref='v2'}
[plugins.review]
all='review@acme-agents'
[plugins.unused]
all='unused@other'
[plugins.direct]
all={path='plugins'}
[plugins.absent]
all=false
[plugins.missing]
claude='only@acme'
"#,
    );
    let output = w.json(&["install", "--dry-run", "--json"], 0);
    assert_eq!(output["marketplaces"].as_array().unwrap().len(), 2);
    assert!(
        output["marketplaces"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["outcome"] == "planned")
    );
    let acme = &output["marketplaces"][0];
    assert_eq!(acme["id"], "acme-agents");
    assert_eq!(acme["logical"], "acme");
    assert_eq!(
        acme["command"],
        "codex plugin marketplace add acme/agents --ref v1 --json"
    );
    let plugins = output["plugins"].as_array().unwrap();
    assert_eq!(plugins.len(), 5);
    assert_eq!(
        plugins.iter().filter(|p| p["outcome"] == "planned").count(),
        2
    );
    assert!(codes(&output).contains(&"harness-not-found"));
    assert!(codes(&output).contains(&"install-skipped"));
    let output = w.json(&["install", "review", "--codex", "--dry-run", "--json"], 0);
    assert_eq!(output["marketplaces"].as_array().unwrap().len(), 1);
    assert_eq!(output["plugins"].as_array().unwrap().len(), 1);
    assert_eq!(output["plugins"][0]["id"], "review@acme-agents");
    assert_eq!(
        output["plugins"][0]["command"],
        "codex plugin add 'review@acme-agents' --json"
    );
    assert!(!w.0.path().join("state").exists());
    assert!(!w.0.path().join("cache").exists());
    let output = w.json(&["install", "review", "--codex", "--json"], 3);
    assert!(codes(&output).contains(&"install-write-failed"));
    assert_eq!(output["marketplaces"][0]["outcome"], "failed");
    assert_eq!(output["plugins"][0]["outcome"], "skipped");
    let output = w.json(&["install", "unknown", "--dry-run", "--json"], 3);
    assert!(codes(&output).contains(&"unknown-plugin"));
    let output = w.run(&["install", "review", "--codex", "--dry-run"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("review@acme-agents"));
}

#[test]
fn codex_git_url_registration_matches_github_shorthand() {
    let w = Workspace::new();
    w.harness("codex", "0.160.0");
    w.write("user.toml", "[marketplaces.acme]\ncodex={source='github:Acme/agents'}\n[plugins.review]\ncodex='review@acme'\n");
    w.write(
        ".codex/config.toml",
        "[marketplaces.acme]\nsource_type='git'\nsource='https://github.com/acme/agents.git'\n",
    );
    let output = w.json(&["install", "--codex", "--dry-run", "--json"], 0);
    assert_eq!(output["marketplaces"][0]["outcome"], "unchanged");
    assert!(!codes(&output).contains(&"marketplace-conflict"));
    w.write(
        ".codex/config.toml",
        "[marketplaces.acme]\nsource_type='git'\nsource='https://github.com/acme/other.git'\n",
    );
    let output = w.json(&["install", "--codex", "--dry-run", "--json"], 3);
    assert!(codes(&output).contains(&"marketplace-conflict"));
}

#[test]
fn adding_project_marketplaces_requires_trust_but_native_registrations_and_private_layers_do_not() {
    let w = Workspace::new();
    w.harness("codex", "0.160.0");
    let declaration = "[marketplaces.acme]\ncodex={source='github:acme/agents',ref='v1'}\n[plugins.review]\nall='review@acme'\n";
    w.write("ayran.toml", declaration);
    let output = w.json(&["install", "--codex", "--dry-run", "--json"], 3);
    assert!(codes(&output).contains(&"untrusted-layer"));
    let diagnostic = output["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "untrusted-layer")
        .unwrap();
    assert!(
        diagnostic["message"]
            .as_str()
            .unwrap()
            .contains("github:acme/agents")
    );
    assert!(diagnostic["hint"].as_str().unwrap().contains("ayran trust"));
    assert!(!w.0.path().join("state").exists());
    assert!(w.run(&["trust"]).status.success());
    let output = w.json(&["install", "--codex", "--dry-run", "--json"], 0);
    assert_eq!(output["plugins"][0]["outcome"], "planned");
    assert!(w.run(&["trust", "--revoke"]).status.success());
    w.write(
        ".codex/config.toml",
        "[marketplaces.acme]\nsource_type='github'\nsource='acme/agents'\nref='v1'\n",
    );
    let output = w.json(&["install", "--codex", "--dry-run", "--json"], 0);
    assert_eq!(output["marketplaces"][0]["outcome"], "unchanged");
    assert!(!codes(&output).contains(&"untrusted-layer"));
    fs::remove_file(w.0.path().join(".codex/config.toml")).unwrap();
    fs::remove_file(w.0.path().join("ayran.toml")).unwrap();
    w.write("ayran.local.toml", declaration);
    let output = w.json(&["install", "--codex", "--dry-run", "--json"], 0);
    assert_eq!(output["marketplaces"][0]["outcome"], "planned");
}

#[test]
fn registered_marketplace_conflicts_and_sha_refs_fail_validation_before_writes() {
    let w = Workspace::new();
    for (h, version) in [
        ("claude", "2.1.288"),
        ("codex", "0.160.0"),
        ("copilot", "1.0.91"),
    ] {
        w.harness(h, version);
    }
    w.write("user.toml", "[marketplaces.acme]\nall={source='github:acme/agents',ref='v1'}\n[plugins.review]\nall='review@acme'\n");
    w.write(
        ".claude/plugins/known_marketplaces.json",
        r#"{"acme":{"source":{"source":"github","repo":"acme/agents","ref":"v0"}}}"#,
    );
    w.write(".copilot/settings.json", "// User settings\n{\"extraKnownMarketplaces\":{\"acme\":{\"source\":{\"source\":\"github\",\"repo\":\"acme/other\",\"ref\":\"v1\"}}},}\n");
    w.write(
        ".codex/config.toml",
        "[marketplaces.acme]\nsource_type='github'\nsource='acme/agents'\nref='v1'\n",
    );
    let output = w.json(&["install", "--json"], 3);
    assert_eq!(
        codes(&output)
            .iter()
            .filter(|c| **c == "marketplace-conflict")
            .count(),
        2
    );
    assert!(!codes(&output).contains(&"install-not-supported"));
    assert!(
        output["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["outcome"] == "planned")
    );
    w.write("user.toml", "[marketplaces.acme]\nall={source='github:acme/agents',ref='0123456789abcdef0123456789abcdef01234567'}\n");
    fs::remove_file(w.0.path().join(".claude/plugins/known_marketplaces.json")).unwrap();
    fs::remove_file(w.0.path().join(".copilot/settings.json")).unwrap();
    fs::remove_file(w.0.path().join(".codex/config.toml")).unwrap();
    let output = w.json(&["install", "--dry-run", "--json"], 3);
    assert_eq!(
        codes(&output)
            .iter()
            .filter(|c| **c == "unsupported-ref")
            .count(),
        2
    );
    let output = w.json(&["install", "--codex", "--dry-run", "--json"], 0);
    assert_eq!(output["marketplaces"][0]["outcome"], "planned");
    w.harness("codex", "0.1.0");
    let output = w.json(&["install", "--codex", "--dry-run", "--json"], 3);
    assert!(codes(&output).contains(&"harness-too-old"));
    assert!(!w.0.path().join("state").exists());
    assert!(!w.0.path().join("cache").exists());
}

#[test]
fn installed_plugins_are_unchanged_even_when_off_and_undeclared_plugins_are_skipped() {
    let w = Workspace::new();
    for (h, version) in [
        ("claude", "2.1.288"),
        ("codex", "0.160.0"),
        ("copilot", "1.0.91"),
    ] {
        w.harness(h, version);
    }
    w.write(
        "user.toml",
        "[plugins.review]\nall='review@acme'\n[plugins.unavailable]\nall='unavailable@missing'\n",
    );
    w.write(
        ".claude/plugins/installed_plugins.json",
        r#"{"version":2,"plugins":{"review@acme":[{"scope":"user","installPath":"unused"}]}}"#,
    );
    w.write(
        ".claude/settings.json",
        r#"{"enabledPlugins":{"review@acme":false}}"#,
    );
    w.write(
        ".codex/config.toml",
        "[plugins.'review@acme']\nenabled=false\n",
    );
    w.write(".codex/plugins/cache/acme/review/1.0/plugin.json", "{}");
    // A live Copilot install only has an enabledPlugins entry; no cache root is needed.
    w.write(
        ".copilot/settings.json",
        r#"{"enabledPlugins":{"review@acme":false}}"#,
    );
    let output = w.json(&["install", "--json"], 0);
    let rows = output["plugins"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .filter(|p| p["logical"] == "review" && p["outcome"] == "unchanged")
            .count(),
        3
    );
    assert_eq!(
        rows.iter()
            .filter(|p| p["logical"] == "unavailable" && p["outcome"] == "skipped")
            .count(),
        3
    );
    assert_eq!(
        codes(&output)
            .iter()
            .filter(|c| **c == "marketplace-undeclared")
            .count(),
        3
    );
    assert!(rows.iter().all(|p| p["command"].is_null()));
    assert!(!w.0.path().join("state").exists());
}

#[test]
fn launch_hint_and_doctor_report_declared_marketplace_install_problems() {
    let w = Workspace::new();
    w.harness("codex", "0.160.0");
    w.write("ayran.toml", "[marketplaces.acme]\ncodex={source='github:acme/agents'}\n[plugins.review]\nall='review@acme'\n");
    let launch = w.run(&["--codex", "--plugin", "review", "--dry-run"]);
    assert_eq!(launch.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&launch.stderr);
    assert!(stderr.contains("native-not-found"));
    assert!(stderr.contains("ayran install review"), "{stderr}");
    let output = w.json(&["doctor", "--codex", "--json"], 1);
    assert!(codes(&output).contains(&"not-installed"));
    assert!(codes(&output).contains(&"untrusted-layer"));
    assert!(codes(&output).contains(&"marketplace-unpinned"));
    assert!(w.run(&["trust"]).status.success());
    let output = w.json(&["doctor", "--codex", "--json"], 0);
    assert!(!codes(&output).contains(&"untrusted-layer"));
    w.write(
        ".codex/config.toml",
        "[marketplaces.acme]\nsource_type='github'\nsource='acme/other'\n",
    );
    let output = w.json(&["doctor", "--codex", "--json"], 1);
    assert!(codes(&output).contains(&"marketplace-conflict"));
}

#[test]
fn conflicting_declarations_for_the_same_native_marketplace_are_rejected() {
    let w = Workspace::new();
    w.harness("codex", "0.160.0");
    w.write("user.toml", "[marketplaces.one]\ncodex={source='github:acme/one',name='shared'}\n[marketplaces.two]\ncodex={source='github:acme/two',name='shared'}\n[plugins.review]\nall='review@shared'\n");
    let output = w.json(&["install", "--dry-run", "--json"], 3);
    assert!(codes(&output).contains(&"marketplace-conflict"));
    let output = w.json(&["install", "review", "--dry-run", "--json"], 3);
    assert!(codes(&output).contains(&"marketplace-conflict"));
}

#[test]
fn planner_reads_registrations_from_launch_homes_and_uses_native_marketplaces_without_declarations()
{
    let w = Workspace::new();
    for (h, version) in [
        ("claude", "2.1.288"),
        ("codex", "0.160.0"),
        ("copilot", "1.0.91"),
    ] {
        w.harness(h, version);
    }
    w.write("user.toml", "[harnesses.claude]\nhome='isolated'\n[harnesses.codex]\nhome='isolated'\n[harnesses.copilot]\nhome='isolated'\n[plugins.review]\nall='review@acme'\n");
    w.write(
        "state/ayran/homes/claude/plugins/known_marketplaces.json",
        r#"{"acme":{"source":{"source":"github","repo":"acme/agents","ref":"v1"}}}"#,
    );
    w.write(
        "state/ayran/homes/codex/config.toml",
        "[marketplaces.acme]\nsource_type='github'\nsource='acme/agents'\nref='v1'\n",
    );
    w.write("state/ayran/homes/copilot/settings.json", r#"{"extraKnownMarketplaces":{"acme":{"source":{"source":"github","repo":"acme/agents","ref":"v1"}}}}"#);
    let output = w.json(&["install", "review", "--dry-run", "--json"], 0);
    assert!(output["marketplaces"].as_array().unwrap().is_empty());
    assert_eq!(output["plugins"].as_array().unwrap().len(), 3);
    assert!(
        output["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["outcome"] == "planned")
    );
    assert!(!codes(&output).contains(&"marketplace-undeclared"));
    w.write("user.toml", "[plugins.review]\nall='review@acme'\n");
    let output = w.json(&["install", "review", "--dry-run", "--json"], 0);
    assert!(
        output["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["outcome"] == "skipped")
    );
    let output = w
        .command(w.0.path())
        .env("CODEX_HOME", w.0.path().join("state/ayran/homes/codex"))
        .args(["install", "review", "--codex", "--dry-run", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["plugins"][0]["outcome"], "planned");
}

#[test]
fn trust_does_not_authorize_different_relative_sources_through_symlinked_layers() {
    let w = Workspace::new();
    w.harness("codex", "0.160.0");
    w.write(
        "source/ayran.toml",
        "[marketplaces.acme]\ncodex={source={path='plugins'}}\n",
    );
    fs::create_dir_all(w.0.path().join("linked")).unwrap();
    std::os::unix::fs::symlink(
        w.0.path().join("source/ayran.toml"),
        w.0.path().join("linked/ayran.toml"),
    )
    .unwrap();
    assert!(w.run(&["trust", "source/ayran.toml"]).status.success());
    let output = w.run_in(
        &w.0.path().join("linked"),
        &["install", "--codex", "--dry-run", "--json"],
    );
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(codes(&output).contains(&"untrusted-layer"));
    assert!(w.run(&["trust", "linked/ayran.toml"]).status.success());
    let list = w.run(&["trust", "--list"]);
    assert!(list.status.success());
    assert!(String::from_utf8_lossy(&list.stdout).contains("true"));
    let output = w.run_in(
        &w.0.path().join("linked"),
        &["install", "--codex", "--dry-run", "--json"],
    );
    assert!(output.status.success(), "{output:?}");
    let output = w.run_in(
        &w.0.path().join("source"),
        &["install", "--codex", "--dry-run", "--json"],
    );
    assert_eq!(output.status.code(), Some(3), "{output:?}");
}

#[test]
fn named_install_includes_a_declared_marketplace_even_when_its_plugin_is_already_installed() {
    let w = Workspace::new();
    w.harness("codex", "0.160.0");
    w.write("user.toml", "[marketplaces.acme]\ncodex={source='github:acme/agents',ref='v1'}\n[plugins.review]\nall='review@acme'\n");
    w.write(
        ".codex/config.toml",
        "[plugins.'review@acme']\nenabled=false\n",
    );
    w.write(".codex/plugins/cache/acme/review/1.0/plugin.json", "{}");
    let output = w.json(&["install", "review", "--codex", "--dry-run", "--json"], 0);
    assert_eq!(output["plugins"][0]["outcome"], "unchanged");
    assert_eq!(output["marketplaces"].as_array().unwrap().len(), 1);
    assert_eq!(output["marketplaces"][0]["outcome"], "planned");
}

#[test]
fn conflicting_native_marketplaces_need_no_trust_because_they_cannot_be_replaced() {
    let w = Workspace::new();
    w.harness("codex", "0.160.0");
    w.write(
        "ayran.toml",
        "[marketplaces.acme]\ncodex={source='github:acme/agents',ref='v1'}\n",
    );
    w.write(
        ".codex/config.toml",
        "[marketplaces.acme]\nsource_type='github'\nsource='acme/other'\nref='v1'\n",
    );
    let output = w.json(&["install", "--codex", "--dry-run", "--json"], 3);
    assert!(codes(&output).contains(&"marketplace-conflict"));
    assert!(!codes(&output).contains(&"untrusted-layer"));
}

#[test]
fn validation_errors_retain_valid_named_targets_without_attempting_writes() {
    let w = Workspace::new();
    w.harness("codex", "0.160.0");
    w.write("user.toml", "[marketplaces.acme]\ncodex={source='github:acme/agents',ref='v1'}\n[plugins.review]\nall='review@acme'\n");
    let output = w.json(&["install", "review", "unknown", "--json"], 3);
    assert!(codes(&output).contains(&"unknown-plugin"));
    assert_eq!(output["plugins"].as_array().unwrap().len(), 1);
    assert_eq!(output["plugins"][0]["id"], "review@acme");
    assert_eq!(output["plugins"][0]["outcome"], "planned");
    assert_eq!(output["marketplaces"][0]["outcome"], "planned");
    assert!(!codes(&output).contains(&"install-not-supported"));
}

#[test]
fn claude_install_adds_a_marketplace_installs_its_plugin_and_leaves_it_natively_off() {
    let w = Workspace::new();
    w.claude_writer();
    w.write("user.toml", "[marketplaces.acme]\nclaude={source='github:acme/agents',ref='v1'}\n[plugins.review]\nclaude='review@acme'\n");
    let output = w.json(&["install", "--claude", "--json"], 0);
    assert_eq!(output["marketplaces"][0]["outcome"], "added");
    assert_eq!(output["plugins"][0]["outcome"], "installed");
    let native = w.json(&["native", "plugin", "list", "--claude", "--json"], 0);
    assert_eq!(native["plugins"][0]["id"], "review@acme");
    assert_eq!(native["plugins"][0]["state"], "off");
    let calls = w.claude_calls();
    assert_eq!(
        calls[0]["args"],
        serde_json::json!([
            "plugin",
            "marketplace",
            "add",
            "acme/agents#v1",
            "--scope",
            "user",
            "--json"
        ])
    );
    assert_eq!(
        calls[1]["args"],
        serde_json::json!([
            "plugin",
            "install",
            "review@acme",
            "--scope",
            "user",
            "--json"
        ])
    );
    assert_eq!(
        calls[2]["args"],
        serde_json::json!([
            "plugin",
            "disable",
            "--scope",
            "user",
            "--json",
            "review@acme"
        ])
    );
    let output = w.json(&["install", "--claude", "--json"], 0);
    assert_eq!(output["marketplaces"][0]["outcome"], "unchanged");
    assert_eq!(output["plugins"][0]["outcome"], "unchanged");
    assert_eq!(w.claude_calls().len(), 3);
}

#[test]
fn claude_install_leaves_existing_enabled_and_disabled_plugins_untouched() {
    for enabled in [true, false] {
        let w = Workspace::new();
        w.claude_writer();
        w.write("user.toml", "[marketplaces.acme]\nclaude={source='github:acme/agents',ref='v1'}\n[plugins.review]\nclaude='review@acme'\n");
        let market = "{\"acme\":{\"source\":{\"source\":\"github\",\"repo\":\"acme/agents\",\"ref\":\"v1\"}}}\n";
        let installs = "{\"version\":2,\"plugins\":{\"review@acme\":[{\"scope\":\"user\",\"installPath\":\"unused\"}]}}\n";
        let settings =
            format!("{{\"theme\":\"dark\",\"enabledPlugins\":{{\"review@acme\":{enabled}}}}}\n");
        w.write(".claude/plugins/known_marketplaces.json", market);
        w.write(".claude/plugins/installed_plugins.json", installs);
        w.write(".claude/settings.json", &settings);
        let output = w.json(&["install", "--claude", "--json"], 0);
        assert_eq!(output["marketplaces"][0]["outcome"], "unchanged");
        assert_eq!(output["plugins"][0]["outcome"], "unchanged");
        assert!(output["plugins"][0]["command"].is_null());
        assert!(w.claude_calls().is_empty());
        for (path, expected) in [
            (".claude/plugins/known_marketplaces.json", market),
            (".claude/plugins/installed_plugins.json", installs),
            (".claude/settings.json", settings.as_str()),
        ] {
            assert_eq!(fs::read_to_string(w.0.path().join(path)).unwrap(), expected);
        }
    }
}

#[test]
fn claude_install_validates_all_sources_before_any_native_write() {
    for (config, registered, expected) in [
        (
            "[marketplaces.acme]\nclaude={source='github:acme/agents',ref='v1'}\n[marketplaces.z]\nclaude={source='github:acme/z'}\n",
            Some(r#"{"z":{"source":{"source":"github","repo":"acme/different"}}}"#),
            "marketplace-conflict",
        ),
        (
            "[marketplaces.acme]\nclaude={source='github:acme/agents',ref='0123456789abcdef0123456789abcdef01234567'}\n",
            None,
            "unsupported-ref",
        ),
    ] {
        let w = Workspace::new();
        w.claude_writer();
        w.write("user.toml", config);
        if let Some(registered) = registered {
            w.write(".claude/plugins/known_marketplaces.json", registered);
        }
        let output = w.json(&["install", "--claude", "--json"], 3);
        assert!(codes(&output).contains(&expected));
        assert!(w.claude_calls().is_empty());
    }
    let w = Workspace::new();
    w.claude_writer();
    w.write(
        "ayran.toml",
        "[marketplaces.acme]\nclaude={source='github:acme/agents',ref='v1'}\n",
    );
    let output = w.json(&["install", "--claude", "--json"], 3);
    assert!(codes(&output).contains(&"untrusted-layer"));
    assert!(w.claude_calls().is_empty());
    assert!(!w.0.path().join(".claude").exists());
}

#[test]
fn claude_install_translates_git_refs_and_absolute_paths_in_the_launch_home() {
    for (home_setting, home_path, source, expected) in [
        (
            "[harnesses.claude]\nhome='isolated'\n",
            "state/ayran/homes/claude",
            "{source='https://example.com/agents.git',ref='release'}",
            "https://example.com/agents.git#release",
        ),
        (
            "",
            "custom-claude",
            "{source={path='local plugins'}}",
            "local plugins",
        ),
    ] {
        let w = Workspace::new();
        w.claude_writer();
        w.write("user.toml", &format!("{home_setting}[marketplaces.acme]\nclaude={source}\n[plugins.review]\nclaude='review@acme'\n"));
        w.write(
            "local plugins/.claude-plugin/marketplace.json",
            r#"{"name":"acme"}"#,
        );
        let mut command = w.command(w.0.path());
        if home_setting.is_empty() {
            command.env("CLAUDE_CONFIG_DIR", w.0.path().join(home_path));
        }
        let output = command
            .args(["install", "--claude", "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let calls = w.claude_calls();
        let source = if expected == "local plugins" {
            w.0.path().join(expected).to_str().unwrap().to_owned()
        } else {
            expected.to_owned()
        };
        assert_eq!(calls[0]["args"][3], source);
        assert_eq!(calls.len(), 3);
        for call in &calls {
            assert_eq!(call["home"], w.0.path().join(home_path).to_str().unwrap());
        }
        assert!(!w.0.path().join(".claude").exists());
        let mut command = w.command(w.0.path());
        if home_setting.is_empty() {
            command.env("CLAUDE_CONFIG_DIR", w.0.path().join(home_path));
        }
        let output = command
            .args(["native", "plugin", "list", "--claude", "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let native: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(native["plugins"][0]["state"], "off");
    }
}

#[test]
fn claude_marketplace_name_mismatch_prevents_plugin_installation() {
    let w = Workspace::new();
    w.claude_writer();
    w.write("user.toml", "[marketplaces.acme]\nclaude={source={path='plugins'}}\n[plugins.review]\nclaude='review@acme'\n");
    w.write(
        "plugins/.claude-plugin/marketplace.json",
        r#"{"name":"different"}"#,
    );
    let output = w.json(&["install", "--claude", "--json"], 3);
    assert!(codes(&output).contains(&"marketplace-name-mismatch"));
    assert_eq!(output["marketplaces"][0]["outcome"], "failed");
    assert_eq!(output["plugins"][0]["outcome"], "skipped");
    assert!(output["plugins"][0]["command"].is_null());
    assert_eq!(w.claude_calls().len(), 1);
    let native = w.json(&["native", "plugin", "list", "--claude", "--json"], 0);
    assert!(native["plugins"].as_array().unwrap().is_empty());
}

#[test]
fn claude_install_failure_preserves_progress_and_retry_resumes_without_readding() {
    for (action, expected_market, call_count) in
        [("marketplace", "failed", 1), ("install", "added", 2)]
    {
        let w = Workspace::new();
        w.claude_writer();
        w.write("user.toml", "[marketplaces.acme]\nclaude={source='github:acme/agents',ref='v1'}\n[plugins.review]\nclaude='review@acme'\n");
        w.write("claude-fail.json", &serde_json::to_string(action).unwrap());
        let output = w.json(&["install", "--claude", "--json"], 3);
        assert!(codes(&output).contains(&"install-write-failed"));
        assert_eq!(output["marketplaces"][0]["outcome"], expected_market);
        assert_eq!(
            output["plugins"][0]["outcome"],
            if action == "install" {
                "failed"
            } else {
                "skipped"
            }
        );
        assert_eq!(w.claude_calls().len(), call_count);
        fs::remove_file(w.0.path().join("claude-fail.json")).unwrap();
        let output = w.json(&["install", "--claude", "--json"], 0);
        assert_eq!(output["plugins"][0]["outcome"], "installed");
        if action == "install" {
            assert_eq!(output["marketplaces"][0]["outcome"], "unchanged");
            assert_eq!(w.claude_calls()[call_count]["args"][1], "install");
        }
        let native = w.json(&["native", "plugin", "list", "--claude", "--json"], 0);
        assert_eq!(native["plugins"][0]["state"], "off");
    }
}

#[test]
fn claude_disable_failure_reports_recovery_and_never_reinstalls_the_plugin() {
    let w = Workspace::new();
    w.claude_writer();
    w.write("user.toml", "[marketplaces.acme]\nclaude={source='github:acme/agents',ref='v1'}\n[plugins.first]\nclaude='first@acme'\n[plugins.later]\nclaude='later@acme'\n");
    w.write("claude-fail.json", "\"disable\"");
    let output = w.json(&["install", "--claude", "--json"], 3);
    assert_eq!(output["marketplaces"][0]["outcome"], "added");
    assert_eq!(output["plugins"][0]["outcome"], "failed");
    assert_eq!(output["plugins"][1]["outcome"], "skipped");
    let diagnostic = output["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "native-write-failed")
        .unwrap();
    assert!(
        diagnostic["hint"]
            .as_str()
            .unwrap()
            .contains("ayran native plugin disable --claude --id 'first@acme'")
    );
    assert_eq!(w.claude_calls().len(), 3);
    fs::remove_file(w.0.path().join("claude-fail.json")).unwrap();
    let output = w.json(&["install", "--claude", "--json"], 0);
    assert_eq!(output["plugins"][0]["outcome"], "unchanged");
    assert_eq!(output["plugins"][1]["outcome"], "installed");
    let calls = w.claude_calls();
    assert_eq!(calls.len(), 5);
    assert_eq!(calls[3]["args"][2], "later@acme");
    let native = w.json(&["native", "plugin", "list", "--claude", "--json"], 0);
    assert_eq!(native["plugins"][0]["state"], "on");
    let output = w.json(
        &[
            "native",
            "plugin",
            "disable",
            "--claude",
            "--id",
            "first@acme",
            "--json",
        ],
        0,
    );
    assert_eq!(output["changes"][0]["after"], "off");
}

#[test]
fn install_deduplicates_native_targets_and_runs_across_harnesses() {
    let w = Workspace::new();
    w.claude_writer();
    w.copilot_writer();
    w.codex_writer();
    w.write("user.toml", "[marketplaces.acme]\nall={source='github:acme/agents',ref='v1'}\n[marketplaces.same]\nclaude={source='github:acme/agents',ref='v1',name='acme'}\n[plugins.review]\nall='review@acme'\n[plugins.same]\nclaude='review@acme'\n");
    let output = w.json(&["install", "--json"], 0);
    assert!(!codes(&output).contains(&"install-not-supported"));
    assert_eq!(w.copilot_calls().len(), 3);
    assert_eq!(w.codex_calls().len(), 2);
    let claude_markets: Vec<_> = output["marketplaces"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["harness"] == "claude")
        .collect();
    assert_eq!(claude_markets[0]["outcome"], "added");
    assert_eq!(claude_markets[1]["outcome"], "unchanged");
    let claude_plugins: Vec<_> = output["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["harness"] == "claude")
        .collect();
    assert_eq!(claude_plugins[0]["outcome"], "installed");
    assert_eq!(claude_plugins[1]["outcome"], "unchanged");
    assert_eq!(w.claude_calls().len(), 3);
    let native = w.json(&["native", "plugin", "list", "--claude", "--json"], 0);
    assert_eq!(native["plugins"][0]["state"], "off");
}

#[test]
fn codex_install_adds_a_sha_pinned_marketplace_and_leaves_new_plugins_natively_off() {
    let w = Workspace::new();
    w.codex_writer();
    w.write("user.toml", "[marketplaces.acme]\ncodex={source='github:acme/agents',ref='0123456789abcdef0123456789abcdef01234567'}\n[plugins.review]\ncodex='review@acme'\n");
    let output = w.json(&["install", "--codex", "--json"], 0);
    assert_eq!(output["marketplaces"][0]["outcome"], "added");
    assert_eq!(output["plugins"][0]["outcome"], "installed");
    let native = w.json(&["native", "plugin", "list", "--codex", "--json"], 0);
    assert_eq!(native["plugins"][0]["state"], "off");
    let calls = w.codex_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0]["args"],
        serde_json::json!([
            "plugin",
            "marketplace",
            "add",
            "acme/agents",
            "--ref",
            "0123456789abcdef0123456789abcdef01234567",
            "--json"
        ])
    );
    assert_eq!(
        calls[1]["args"],
        serde_json::json!(["plugin", "add", "review@acme", "--json"])
    );
    let config = fs::read_to_string(w.0.path().join(".codex/config.toml")).unwrap();
    assert!(config.contains("# preserved setting"));
    assert!(config.contains("model = \"fixture\""));
    let output = w.json(&["install", "--codex", "--json"], 0);
    assert_eq!(output["marketplaces"][0]["outcome"], "unchanged");
    assert_eq!(output["plugins"][0]["outcome"], "unchanged");
    assert_eq!(w.codex_calls().len(), 2);
    assert_eq!(
        fs::read_to_string(w.0.path().join(".codex/config.toml")).unwrap(),
        config
    );
}

#[test]
fn codex_install_leaves_existing_on_and_off_plugins_untouched() {
    for enabled in [true, false] {
        let w = Workspace::new();
        w.codex_writer();
        w.write("user.toml", "[marketplaces.acme]\ncodex={source='github:acme/agents',ref='v1'}\n[plugins.review]\ncodex='review@acme'\n");
        let config = format!(
            "[marketplaces.acme]\nsource_type='github'\nsource='acme/agents'\nref='v1'\n[plugins.'review@acme']\nenabled={enabled}\n"
        );
        w.write(".codex/config.toml", &config);
        w.write(".codex/plugins/cache/acme/review/1.0/plugin.json", "{}");
        let output = w.json(&["install", "--codex", "--json"], 0);
        assert_eq!(output["marketplaces"][0]["outcome"], "unchanged");
        assert_eq!(output["plugins"][0]["outcome"], "unchanged");
        assert!(w.codex_calls().is_empty());
        assert_eq!(
            fs::read_to_string(w.0.path().join(".codex/config.toml")).unwrap(),
            config
        );
    }
}

#[test]
fn codex_install_conflicts_and_untrusted_sources_prevent_all_writes() {
    for (file, config, native, code) in [
        (
            "user.toml",
            "[marketplaces.acme]\ncodex={source='github:acme/agents',ref='v1'}\n[marketplaces.z]\ncodex={source='github:acme/z'}\n",
            "[marketplaces.z]\nsource_type='github'\nsource='acme/other'\n",
            "marketplace-conflict",
        ),
        (
            "user.toml",
            "[marketplaces.acme]\ncodex={source='github:acme/agents',ref='v1'}\n",
            "[marketplaces.acme]\nsource_type='github'\nsource='acme/agents'\nref='v2'\n",
            "marketplace-conflict",
        ),
        (
            "ayran.toml",
            "[marketplaces.acme]\ncodex={source='github:acme/agents',ref='v1'}\n",
            "",
            "untrusted-layer",
        ),
    ] {
        let w = Workspace::new();
        w.codex_writer();
        w.write(file, config);
        w.write(".codex/config.toml", native);
        let output = w.json(&["install", "--codex", "--json"], 3);
        assert!(codes(&output).contains(&code));
        assert!(w.codex_calls().is_empty());
        assert_eq!(
            fs::read_to_string(w.0.path().join(".codex/config.toml")).unwrap(),
            native
        );
    }
}

#[test]
fn codex_install_translates_git_and_path_sources_in_launch_homes() {
    for (setting, home_path, source, expected) in [
        (
            "[harnesses.codex]\nhome='isolated'\n",
            "state/ayran/homes/codex",
            "{source='ssh://example.com/agents.git',ref='release'}",
            "ssh://example.com/agents.git",
        ),
        (
            "",
            "custom-codex",
            "{source={path='local plugins'}}",
            "local plugins",
        ),
    ] {
        let w = Workspace::new();
        w.codex_writer();
        w.write("user.toml", &format!("{setting}[marketplaces.acme]\ncodex={source}\n[plugins.review]\ncodex='review@acme'\n"));
        w.write(
            "local plugins/.agents/plugins/marketplace.json",
            r#"{"name":"acme"}"#,
        );
        w.write(
            "local plugins/.claude-plugin/marketplace.json",
            r#"{"name":"claude-name"}"#,
        );
        let mut command = w.command(w.0.path());
        if setting.is_empty() {
            command.env("CODEX_HOME", w.0.path().join(home_path));
        }
        let output = command
            .args(["install", "--codex", "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let calls = w.codex_calls();
        assert_eq!(calls.len(), 2);
        let expected = if expected == "local plugins" {
            w.0.path().join(expected).display().to_string()
        } else {
            expected.to_owned()
        };
        assert_eq!(calls[0]["args"][3], expected);
        for call in calls {
            assert_eq!(call["home"], w.0.path().join(home_path).to_str().unwrap());
        }
        let config: toml::Value = toml::from_str(
            &fs::read_to_string(w.0.path().join(home_path).join("config.toml")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            config["plugins"]["review@acme"]["enabled"].as_bool(),
            Some(false)
        );
        assert!(!w.0.path().join(".codex").exists());
    }
}

#[test]
fn codex_marketplace_name_mismatch_prevents_plugin_installation() {
    let w = Workspace::new();
    w.codex_writer();
    w.write("user.toml", "[marketplaces.acme]\ncodex={source={path='plugins'}}\n[plugins.review]\ncodex='review@acme'\n");
    w.write(
        "plugins/.agents/plugins/api_marketplace.json",
        r#"{"name":"different"}"#,
    );
    w.write(
        "plugins/.claude-plugin/marketplace.json",
        r#"{"name":"acme"}"#,
    );
    let output = w.json(&["install", "--codex", "--json"], 3);
    assert!(codes(&output).contains(&"marketplace-name-mismatch"));
    assert_eq!(output["marketplaces"][0]["outcome"], "failed");
    assert_eq!(output["plugins"][0]["outcome"], "skipped");
    assert_eq!(w.codex_calls().len(), 1);
}

#[test]
fn copilot_install_adds_a_marketplace_and_leaves_new_plugins_natively_off() {
    let w = Workspace::new();
    w.copilot_writer();
    w.write("user.toml", "[marketplaces.acme]\ncopilot={source='github:acme/agents',ref='v1'}\n[plugins.review]\ncopilot='review@acme'\n");
    let output = w.json(&["install", "--copilot", "--json"], 0);
    assert_eq!(output["marketplaces"][0]["outcome"], "added");
    assert_eq!(output["plugins"][0]["outcome"], "installed");
    let native = w.json(&["native", "plugin", "list", "--copilot", "--json"], 0);
    assert_eq!(native["plugins"][0]["id"], "review@acme");
    assert_eq!(native["plugins"][0]["state"], "off");
    let calls = w.copilot_calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls[0]["args"],
        serde_json::json!(["plugin", "marketplace", "add", "acme/agents#v1"])
    );
    assert_eq!(
        calls[1]["args"],
        serde_json::json!(["plugin", "install", "review@acme"])
    );
    assert_eq!(
        calls[2]["args"],
        serde_json::json!(["plugin", "disable", "review@acme"])
    );
    let output = w.json(&["install", "--copilot", "--json"], 0);
    assert_eq!(output["marketplaces"][0]["outcome"], "unchanged");
    assert_eq!(output["plugins"][0]["outcome"], "unchanged");
    assert_eq!(w.copilot_calls().len(), 3);
}

#[test]
fn copilot_install_preserves_existing_git_and_live_plugins_on_or_off() {
    for live in [false, true] {
        for enabled in [false, true] {
            let w = Workspace::new();
            w.copilot_writer();
            w.write("user.toml", "[marketplaces.acme]\ncopilot={source='github:acme/agents',ref='v1'}\n[plugins.review]\ncopilot='review@acme'\n");
            let settings = r#"{"extraKnownMarketplaces":{"acme":{"source":{"source":"github","repo":"acme/agents","ref":"v1"}}}}"#;
            let config = if live {
                serde_json::json!({"theme":"dark", "enabledPlugins":{"review@acme":enabled}})
            } else {
                // A copied install record alone must prevent reinstall, too.
                serde_json::json!({"theme":"dark", "installedPlugins":[{"name":"review", "marketplace":"acme", "enabled":enabled}]})
            }.to_string();
            w.write(".copilot/settings.json", settings);
            w.write(".copilot/config.json", &config);
            let output = w.json(&["install", "--copilot", "--json"], 0);
            assert_eq!(output["marketplaces"][0]["outcome"], "unchanged");
            assert_eq!(output["plugins"][0]["outcome"], "unchanged");
            assert!(output["plugins"][0]["command"].is_null());
            assert!(w.copilot_calls().is_empty());
            assert_eq!(
                fs::read_to_string(w.0.path().join(".copilot/config.json")).unwrap(),
                config
            );
            assert_eq!(
                fs::read_to_string(w.0.path().join(".copilot/settings.json")).unwrap(),
                settings
            );
        }
    }
}

#[test]
fn copilot_install_validates_all_targets_before_writing() {
    for (file, config, registered, expected) in [
        (
            "user.toml",
            "[marketplaces.acme]\ncopilot={source='github:acme/agents',ref='v1'}\n[marketplaces.z]\ncopilot={source='github:acme/z'}\n",
            r#"{"extraKnownMarketplaces":{"z":{"source":{"source":"github","repo":"acme/other"}}}}"#,
            "marketplace-conflict",
        ),
        (
            "user.toml",
            "[marketplaces.acme]\ncopilot={source='github:acme/agents',ref='v1'}\n",
            r#"{"extraKnownMarketplaces":{"acme":{"source":{"source":"github","repo":"acme/agents","ref":"v2"}}}}"#,
            "marketplace-conflict",
        ),
        (
            "user.toml",
            "[marketplaces.acme]\ncopilot={source='github:acme/agents',ref='0123456789abcdef0123456789abcdef01234567'}\n",
            "{}",
            "unsupported-ref",
        ),
        (
            "ayran.toml",
            "[marketplaces.acme]\ncopilot={source='github:acme/agents',ref='v1'}\n",
            "{}",
            "untrusted-layer",
        ),
    ] {
        let w = Workspace::new();
        w.copilot_writer();
        w.write(file, config);
        w.write(".copilot/settings.json", registered);
        let output = w.json(&["install", "--copilot", "--json"], 3);
        assert!(codes(&output).contains(&expected));
        assert!(w.copilot_calls().is_empty());
        assert_eq!(
            fs::read_to_string(w.0.path().join(".copilot/settings.json")).unwrap(),
            registered
        );
        assert!(!w.0.path().join(".copilot/config.json").exists());
    }
}

#[test]
fn copilot_install_translates_sources_in_launch_homes_and_keeps_the_cache_home() {
    for (setting, home_path, source, expected) in [
        (
            "[harnesses.copilot]\nhome='isolated'\n",
            "state/ayran/homes/copilot",
            "{source='ssh://example.com/agents.git',ref='release'}",
            "ssh://example.com/agents.git#release",
        ),
        (
            "",
            "custom-copilot",
            "{source={path='local plugins'}}",
            "local plugins",
        ),
    ] {
        let w = Workspace::new();
        w.copilot_writer();
        w.write("user.toml", &format!("{setting}[marketplaces.acme]\ncopilot={source}\n[plugins.review]\ncopilot='review@acme'\n"));
        w.write("local plugins/marketplace.json", r#"{"name":"acme"}"#);
        w.write(
            "local plugins/.claude-plugin/marketplace.json",
            r#"{"name":"claude-name"}"#,
        );
        let cache_home = w.0.path().join("shared-cache");
        let mut command = w.command(w.0.path());
        command.env("COPILOT_CACHE_HOME", &cache_home);
        if setting.is_empty() {
            command.env("COPILOT_HOME", w.0.path().join(home_path));
        }
        let output = command
            .args(["install", "--copilot", "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let calls = w.copilot_calls();
        assert_eq!(calls.len(), 3);
        let expected = if expected == "local plugins" {
            w.0.path().join(expected).display().to_string()
        } else {
            expected.to_owned()
        };
        assert_eq!(calls[0]["args"][3], expected);
        for call in calls {
            assert_eq!(call["home"], w.0.path().join(home_path).to_str().unwrap());
            assert_eq!(call["cache"], cache_home.to_str().unwrap());
        }
        let config: Value = serde_json::from_str(
            &fs::read_to_string(w.0.path().join(home_path).join("config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(config["enabledPlugins"]["review@acme"], false);
        if setting.is_empty() {
            assert!(config.get("installedPlugins").is_none());
        } else {
            assert_eq!(config["installedPlugins"][0]["enabled"], false);
        }
        assert!(!w.0.path().join(".copilot").exists());
        // A live install's off entry alone must prevent a second install.
        let mut command = w.command(w.0.path());
        if setting.is_empty() {
            command.env("COPILOT_HOME", w.0.path().join(home_path));
        }
        let output = command
            .args(["install", "--copilot", "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(w.copilot_calls().len(), 3);
    }
}

#[test]
fn copilot_marketplace_name_mismatch_prevents_plugin_installation() {
    for manifest in [
        "marketplace.json",
        ".plugin/marketplace.json",
        ".github/plugin/marketplace.json",
        ".claude-plugin/marketplace.json",
    ] {
        let w = Workspace::new();
        w.copilot_writer();
        w.write("user.toml", "[marketplaces.acme]\ncopilot={source={path='plugins'}}\n[plugins.review]\ncopilot='review@acme'\n");
        w.write(&format!("plugins/{manifest}"), r#"{"name":"different"}"#);
        let output = w.json(&["install", "--copilot", "--json"], 3);
        assert!(codes(&output).contains(&"marketplace-name-mismatch"));
        assert_eq!(output["marketplaces"][0]["outcome"], "failed");
        assert_eq!(output["plugins"][0]["outcome"], "skipped");
        assert_eq!(w.copilot_calls().len(), 1);
        assert!(!w.0.path().join(".copilot/config.json").exists());
    }
}

#[test]
fn copilot_install_failure_preserves_progress_and_retry_resumes() {
    for action in ["marketplace", "install", "disable"] {
        let w = Workspace::new();
        w.copilot_writer();
        w.write("user.toml", "[marketplaces.acme]\ncopilot={source='github:acme/agents',ref='v1'}\n[plugins.first]\ncopilot='first@acme'\n[plugins.later]\ncopilot='later@acme'\n");
        w.write("copilot-fail.json", &serde_json::to_string(action).unwrap());
        let output = w.json(&["install", "--copilot", "--json"], 3);
        assert_eq!(
            output["marketplaces"][0]["outcome"],
            if action == "marketplace" {
                "failed"
            } else {
                "added"
            }
        );
        assert_eq!(
            output["plugins"][0]["outcome"],
            if action == "marketplace" {
                "skipped"
            } else {
                "failed"
            }
        );
        assert_eq!(output["plugins"][1]["outcome"], "skipped");
        if action == "disable" {
            let diagnostic = output["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .find(|d| d["code"] == "native-write-failed")
                .unwrap();
            assert!(
                diagnostic["hint"]
                    .as_str()
                    .unwrap()
                    .contains("ayran native plugin disable --copilot --id 'first@acme'")
            );
        } else {
            assert!(codes(&output).contains(&"install-write-failed"));
        }
        let before = w.copilot_calls().len();
        fs::remove_file(w.0.path().join("copilot-fail.json")).unwrap();
        let output = w.json(&["install", "--copilot", "--json"], 0);
        assert_eq!(output["plugins"][1]["outcome"], "installed");
        assert_eq!(
            output["plugins"][0]["outcome"],
            if action == "disable" {
                "unchanged"
            } else {
                "installed"
            }
        );
        if action != "marketplace" {
            assert_eq!(output["marketplaces"][0]["outcome"], "unchanged");
            assert_eq!(
                w.copilot_calls()[before]["args"],
                serde_json::json!([
                    "plugin",
                    "install",
                    if action == "disable" {
                        "later@acme"
                    } else {
                        "first@acme"
                    }
                ])
            );
        }
        let native = w.json(&["native", "plugin", "list", "--copilot", "--json"], 0);
        assert_eq!(
            native["plugins"][0]["state"],
            if action == "disable" { "on" } else { "off" }
        );
        if action == "disable" {
            let output = w.json(
                &[
                    "native",
                    "plugin",
                    "disable",
                    "--copilot",
                    "--id",
                    "first@acme",
                    "--json",
                ],
                0,
            );
            assert_eq!(output["changes"][0]["after"], "off");
        }
    }
}
