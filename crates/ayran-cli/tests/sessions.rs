use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

struct Home(TempDir);
impl Home {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ayran"));
        command.current_dir(self.0.path());
        for (key, path) in [
            ("HOME", ""),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_CACHE_HOME", "cache"),
            ("CLAUDE_CONFIG_DIR", "claude"),
            ("CODEX_HOME", "codex"),
            ("COPILOT_HOME", "copilot"),
        ] {
            command.env(key, self.0.path().join(path));
        }
        command
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    fn record(&self, id: &str, harness: &str, cwd: &Path, used: &str) -> std::path::PathBuf {
        let directory = self.0.path().join("state/ayran/sessions");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!("{id}.json"));
        fs::write(&path, json!({"version":1,"id":id,"native_id":if harness == "codex" {None} else {Some(id)},"harness":harness,"home":"shared","cwd":cwd,"started_at":"2026-01-01T00:00:00Z","last_used_at":used,"request":{"model":"opus","skills":["x"]},"forked_from":null}).to_string()).unwrap();
        path
    }
}
fn json_output(output: Output) -> Value {
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn list_sessions_filters_sorts_and_skips_corrupt_records_without_touching_use() {
    let home = Home::new();
    let older = home.record(
        "11111111-1111-4111-8111-111111111111",
        "claude",
        home.0.path(),
        "2026-09-30T10:00:00Z",
    );
    home.record(
        "22222222-2222-4222-8222-222222222222",
        "codex",
        home.0.path(),
        "2026-09-30T12:00:00Z",
    );
    home.record(
        "33333333-3333-4333-8333-333333333333",
        "copilot",
        &home.0.path().join("elsewhere"),
        "2026-09-30T13:00:00Z",
    );
    fs::write(older.parent().unwrap().join("corrupt.json"), "broken").unwrap();
    let before = fs::read(&older).unwrap();
    let result = json_output(home.run(&["list", "sessions", "--json"]));
    assert_eq!(result["sessions"].as_array().unwrap().len(), 2);
    assert_eq!(
        result["sessions"][0]["id"],
        "22222222-2222-4222-8222-222222222222"
    );
    assert_eq!(result["sessions"][0]["link"], "unlinked");
    assert_eq!(result["sessions"][1]["link"], "linked");
    assert_eq!(result["sessions"][1]["alias"], Value::Null);
    assert_eq!(result["version"], 1);
    assert_eq!(fs::read(&older).unwrap(), before);
    let all = json_output(home.run(&["list", "sessions", "--all", "--json"]));
    assert_eq!(all["sessions"].as_array().unwrap().len(), 3);
    for flag in ["--claude", "--harness"] {
        let mut args = vec!["list", "sessions", "--all", "--json", flag];
        if flag == "--harness" {
            args.push("claude");
        }
        assert_eq!(
            json_output(home.run(&args))["sessions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    assert_eq!(
        json_output(home.run(&["list", "--json"]))["sessions"],
        json!([])
    );
    let text = home.run(&["list", "sessions", "--all"]);
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.lines().next().unwrap().ends_with("\tcwd"));
    assert!(
        text.contains("22222222\tcodex\t2026-09-30T12:00:00Z\t\t-m opus --skill x\t\t"),
        "{text}"
    );
}

#[cfg(unix)]
#[test]
fn resume_last_filters_directory_and_harness_and_rejects_invalid_selectors() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    let bin = home.0.path().join("bin");
    fs::create_dir(&bin).unwrap();
    for harness in ["claude", "copilot"] {
        fs::write(bin.join(harness), "#!/bin/sh\nprintf '9.0.0\\n'\n").unwrap();
        fs::set_permissions(bin.join(harness), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let other = home.0.path().join("other");
    fs::create_dir(&other).unwrap();
    home.record(
        "11111111-1111-4111-8111-111111111111",
        "claude",
        home.0.path(),
        "2026-09-30T10:00:00Z",
    );
    home.record(
        "22222222-2222-4222-8222-222222222222",
        "claude",
        &other,
        "2026-09-30T13:00:00Z",
    );
    home.record(
        "33333333-3333-4333-8333-333333333333",
        "copilot",
        home.0.path(),
        "2026-09-30T12:00:00Z",
    );
    for (flags, expected) in [
        (vec![], "33333333-3333-4333-8333-333333333333"),
        (vec!["--all"], "22222222-2222-4222-8222-222222222222"),
        (vec!["--claude"], "11111111-1111-4111-8111-111111111111"),
        (
            vec!["--claude", "--all"],
            "22222222-2222-4222-8222-222222222222",
        ),
    ] {
        let mut args = vec!["resume", "--last", "--dry-run", "--json", "--no-skill", "x"];
        args.extend(flags);
        let result = json_output(
            home.command()
                .env("PATH", &bin)
                .args(args)
                .output()
                .unwrap(),
        );
        assert_eq!(result["session_id"], expected);
    }
    let missing = home.run(&["resume", "--last", "--codex"]);
    assert_eq!(missing.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("unknown-session"));
    for args in [
        vec!["resume"],
        vec!["resume", "11111111", "--last"],
        vec!["resume", "11111111", "--all"],
    ] {
        assert_eq!(home.run(&args).status.code(), Some(2));
    }
}

#[test]
fn resume_completes_short_ids_and_descriptions_and_ignores_bad_records() {
    let home = Home::new();
    let path = home.record(
        "abcdef12-1111-4111-8111-111111111111",
        "claude",
        home.0.path(),
        "2026-09-30T10:00:00Z",
    );
    fs::write(path.parent().unwrap().join("bad.json"), "{}").unwrap();
    let output = home.run(&["__complete", "--", "ayran", "resume", "abc"]);
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "abcdef12\n");
    let output = home.run(&[
        "__complete",
        "--descriptions",
        "--",
        "ayran",
        "resume",
        "abc",
    ]);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "abcdef12\tclaude 2026-09-30T10:00:00Z -m opus --skill x\n"
    );
    let empty = home.run(&["__complete", "--", "ayran", "resume", ""]);
    assert!(
        String::from_utf8(empty.stdout)
            .unwrap()
            .lines()
            .any(|line| line == "abcdef12")
    );
    for prefix in ["ABC", "ab-c"] {
        let output = home.run(&["__complete", "--", "ayran", "resume", prefix]);
        assert_eq!(String::from_utf8(output.stdout).unwrap(), "abcdef12\n");
    }
    let after_id = home.run(&["__complete", "--", "ayran", "resume", "abcdef12", "--"]);
    let flags = String::from_utf8(after_id.stdout).unwrap();
    assert!(
        !flags.lines().any(|flag| matches!(flag, "--last" | "--all")),
        "{flags}"
    );
    let output = home.run(&["__complete", "--", "ayran", "list", "sess"]);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "sessions\n");
    assert!(
        home.run(&["__complete", "--", "ayran", "resume", "--last", "abc"])
            .stdout
            .is_empty()
    );
}

#[test]
fn list_links_codex_root_rollout_without_counting_it_as_use() {
    let home = Home::new();
    let id = "11111111-1111-4111-8111-111111111111";
    let native = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let path = home.record(id, "codex", home.0.path(), "2026-09-30T10:00:00Z");
    let directory = home.0.path().join("codex/sessions/2026/01/01");
    fs::create_dir_all(&directory).unwrap();
    for (name, payload) in [
        (
            "main",
            json!({"id":native,"session_id":native,"cwd":home.0.path(),"timestamp":"2026-01-01T00:00:01Z","source":"cli"}),
        ),
        (
            "guardian",
            json!({"id":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","cwd":home.0.path(),"timestamp":"2026-01-01T00:00:01Z","source":{"internal":"guardian"}}),
        ),
        (
            "subagent",
            json!({"id":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","cwd":home.0.path(),"timestamp":"2026-01-01T00:00:01Z","parent_thread_id":native,"source":"cli"}),
        ),
    ] {
        fs::write(
            directory.join(format!("rollout-{name}.jsonl")),
            json!({"type":"session_meta","payload":payload}).to_string(),
        )
        .unwrap();
    }
    let listed = json_output(home.run(&["list", "sessions", "--json"]));
    assert_eq!(listed["sessions"][0]["native_id"], native);
    assert_eq!(listed["sessions"][0]["link"], "linked");
    let record: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(record["native_id"], native);
    assert_eq!(record["last_used_at"], "2026-09-30T10:00:00Z");
}

impl Home {
    fn rollout(&self, name: &str, id: &str, time: &str, extra: Value, message: &str) {
        let directory = self.0.path().join("codex/sessions/2026/01/01");
        fs::create_dir_all(&directory).unwrap();
        let mut meta = json!({"id":id,"session_id":"different-root-interaction","cwd":self.0.path(),"timestamp":time,"source":"cli"});
        meta.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let rows = [
            json!({"type":"session_meta","timestamp":"2099-01-01T00:00:00Z","payload":meta}),
            json!({"type":"response_item","payload":{"role":"user","content":[{"type":"input_text","text":"injected context"}]}}),
            json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"text","text":message}]}}}),
        ];
        fs::write(
            directory.join(format!("rollout-{name}.jsonl")),
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
    }
}

#[test]
fn codex_link_windows_include_forks_and_ignore_already_linked_threads() {
    let home = Home::new();
    let first = "11111111-1111-4111-8111-111111111111";
    let second = "22222222-2222-4222-8222-222222222222";
    let native_first = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let native_second = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    home.record(first, "codex", home.0.path(), "2026-09-30T10:00:00Z");
    let path = home.record(second, "codex", home.0.path(), "2026-09-30T11:00:00Z");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["started_at"] = json!("2026-01-01T00:00:02Z");
    record["forked_from"] = json!(first);
    fs::write(&path, record.to_string()).unwrap();
    home.rollout(
        "first",
        native_first,
        "2026-01-01T01:00:00+01:00",
        json!({}),
        "first",
    );
    home.rollout(
        "fork",
        native_second,
        "2026-01-01T00:00:02Z",
        json!({"forked_from_id":native_first}),
        "fork",
    );
    // An internal source without a parent is still excluded.
    home.rollout(
        "internal",
        "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
        "2026-01-01T00:00:03Z",
        json!({"source":{"subagent":{}}}),
        "internal",
    );
    let listed = json_output(home.run(&["list", "sessions", "--json"]));
    assert_eq!(listed["sessions"][0]["native_id"], native_second);
    assert_eq!(listed["sessions"][1]["native_id"], native_first);
    // A new record whose window includes both cannot adopt already linked threads.
    let third = home.record(
        "33333333-3333-4333-8333-333333333333",
        "codex",
        home.0.path(),
        "2026-09-30T12:00:00Z",
    );
    let listed = json_output(home.run(&["list", "sessions", "--json"]));
    assert_eq!(listed["sessions"][0]["link"], "unlinked");
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(third).unwrap()).unwrap()["native_id"],
        Value::Null
    );
}

#[test]
fn ambiguous_codex_listing_never_prompts_and_resume_requires_a_choice() {
    let home = Home::new();
    let id = "11111111-1111-4111-8111-111111111111";
    let path = home.record(id, "codex", home.0.path(), "2026-09-30T10:00:00Z");
    let before = fs::read(&path).unwrap();
    let missing = home.run(&["resume", id]);
    assert_eq!(missing.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("codex-link-not-found"));
    for (name, native) in [
        ("first", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
        ("second", "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"),
    ] {
        home.rollout(name, native, "2026-01-01T00:00:01Z", json!({}), name);
    }
    let listed = json_output(home.run(&["list", "sessions", "--json"]));
    assert_eq!(listed["sessions"][0]["link"], "ambiguous");
    assert_eq!(listed["sessions"][0]["native_id"], Value::Null);
    let text = home.run(&["list", "sessions"]);
    assert!(String::from_utf8_lossy(&text.stdout).contains('?'));
    let output = home.run(&["resume", id]);
    assert_eq!(output.status.code(), Some(3));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("codex-link-ambiguous"));
    assert!(error.contains("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));
    assert!(error.contains("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"));
    assert_eq!(fs::read(&path).unwrap(), before);
    let claude = "22222222-2222-4222-8222-222222222222";
    home.record(claude, "claude", home.0.path(), "2026-09-30T10:00:00Z");
    let output = home.run(&[
        "resume",
        claude,
        "--native",
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage"));
    let invalid = home.run(&["resume", id, "--native", "invalid"]);
    assert_eq!(invalid.status.code(), Some(2));
}

#[cfg(unix)]
#[test]
fn codex_picker_on_a_terminal_shows_user_messages_and_accepts_or_cancels() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    let id = "11111111-1111-4111-8111-111111111111";
    let path = home.record(id, "codex", home.0.path(), "2026-09-30T10:00:00Z");
    let before = fs::read(&path).unwrap();
    let bin = home.0.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::write(bin.join("codex"), "#!/bin/sh\nprintf '9.0.0\\n'\n").unwrap();
    fs::set_permissions(bin.join("codex"), fs::Permissions::from_mode(0o755)).unwrap();
    home.rollout(
        "first",
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
        "2026-01-01T00:00:01Z",
        json!({}),
        "first prompt\nsecond line",
    );
    home.rollout(
        "second",
        "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
        "2026-01-01T00:00:02Z",
        json!({"forked_from_id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}),
        "second prompt",
    );
    let rollouts = home.0.path().join("codex/sessions/2026/01/01");
    let fork = rollouts.join("rollout-second.jsonl");
    let metadata = fs::read_to_string(&fork)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_owned();
    fs::write(&fork, metadata).unwrap();
    let legacy = rollouts.join("rollout-legacy.jsonl");
    home.rollout(
        "legacy",
        "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
        "2026-01-01T00:00:03Z",
        json!({}),
        "unused",
    );
    let metadata = fs::read_to_string(&legacy)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_owned();
    fs::write(
        &legacy,
        format!(
            "{metadata}\n{}",
            json!({"type":"event_msg","payload":{"type":"user_message","message":"legacy prompt"}})
        ),
    )
    .unwrap();
    let script = r#"
import os, pty, subprocess, sys, json
master, slave = pty.openpty()
p = subprocess.Popen([sys.argv[1], 'resume', sys.argv[2], '--no-skill', 'x', '--dry-run', '--json'], stdin=slave, stderr=slave, stdout=subprocess.PIPE)
os.close(slave)
os.write(master, sys.argv[3].encode())
stdout, _ = p.communicate(timeout=10)
stderr = b''
while True:
    try:
        chunk = os.read(master, 65536)
        if not chunk: break
        stderr += chunk
    except OSError: break
os.close(master)
print(json.dumps({'status':p.returncode,'stdout':stdout.decode(),'stderr':stderr.decode()}))
"#;
    for (answer, expected) in [("2\n", 0), ("\n", 3), ("\u{4}", 3), ("99\n", 3)] {
        let env_command = home.command();
        let result = Command::new("python3")
            .current_dir(home.0.path())
            .envs(
                env_command
                    .get_envs()
                    .filter_map(|(key, value)| value.map(|value| (key, value))),
            )
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .args(["-c", script, env!("CARGO_BIN_EXE_ayran"), id, answer])
            .output()
            .unwrap();
        let result = json_output(result);
        assert_eq!(result["status"], expected, "{result}");
        let stderr = result["stderr"].as_str().unwrap();
        assert!(stderr.contains("1.") && stderr.contains("2."), "{stderr}");
        assert!(stderr.contains("first prompt second line"), "{stderr}");
        assert!(stderr.contains("(no user message)"), "{stderr}");
        assert!(stderr.contains("legacy prompt"), "{stderr}");
        assert!(!stderr.contains("injected context"), "{stderr}");
        if expected == 0 {
            let dry: Value = serde_json::from_str(result["stdout"].as_str().unwrap()).unwrap();
            assert_eq!(dry["argv"][2], "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
        }
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn listing_does_not_link_one_rollout_to_two_sessions_with_equal_start_times() {
    let home = Home::new();
    home.record(
        "11111111-1111-4111-8111-111111111111",
        "codex",
        home.0.path(),
        "2026-09-30T10:00:00Z",
    );
    home.record(
        "22222222-2222-4222-8222-222222222222",
        "codex",
        home.0.path(),
        "2026-09-30T11:00:00Z",
    );
    home.rollout(
        "main",
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
        "2026-01-01T00:00:01Z",
        json!({}),
        "prompt",
    );
    let listed = json_output(home.run(&["list", "sessions", "--json"]));
    assert_eq!(listed["sessions"][0]["link"], "linked");
    assert_eq!(listed["sessions"][1]["link"], "unlinked");
}
