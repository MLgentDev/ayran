#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

struct TestHome {
    dir: TempDir,
}

#[test]
fn claude_fork_inherits_home_and_request_and_only_touches_parent_use() {
    let home = TestHome::new();
    home.config("[harnesses.claude]\nhome = 'isolated'\n");
    assert!(home.run(&["--claude", "-m", "opus"]).status.success());
    let parent_path = home.session_path();
    let before = fs::read(&parent_path).unwrap();
    let parent: serde_json::Value = serde_json::from_slice(&before).unwrap();
    let id = parent["id"].as_str().unwrap();
    home.config("");
    let dry = home.run(&[
        "resume",
        "--last",
        "--fork",
        "-m",
        "sonnet",
        "--dry-run",
        "--json",
    ]);
    assert!(dry.status.success(), "{dry:?}");
    let dry: serde_json::Value = serde_json::from_slice(&dry.stdout).unwrap();
    assert_ne!(dry["session_id"], parent["id"]);
    assert_eq!(dry["argv"][1], "--resume");
    assert_eq!(dry["argv"][2], id);
    assert_eq!(dry["argv"][3], "--fork-session");
    assert_eq!(dry["argv"][4], "--session-id");
    assert_eq!(dry["argv"][5], dry["session_id"]);
    assert_eq!(fs::read(&parent_path).unwrap(), before);
    assert_eq!(
        fs::read_dir(parent_path.parent().unwrap()).unwrap().count(),
        1
    );
    let output = home.run(&["resume", id, "--fork", "-m", "sonnet"]);
    assert!(output.status.success(), "{output:?}");
    let fork_path = fs::read_dir(parent_path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path != &parent_path)
        .unwrap();
    let fork: serde_json::Value = serde_json::from_slice(&fs::read(fork_path).unwrap()).unwrap();
    assert_eq!(fork["forked_from"], id);
    assert_eq!(fork["native_id"], fork["id"]);
    assert_eq!(fork["request"]["model"], "sonnet");
    for field in ["harness", "home", "cwd"] {
        assert_eq!(fork[field], parent[field]);
    }
    assert_ne!(fork["started_at"], parent["started_at"]);
    let mut after: serde_json::Value =
        serde_json::from_slice(&fs::read(&parent_path).unwrap()).unwrap();
    assert_ne!(after["last_used_at"], parent["last_used_at"]);
    after["last_used_at"] = parent["last_used_at"].clone();
    assert_eq!(after, parent);
    assert!(home.raw_record().starts_with(&format!(
        "--resume\n{id}\n--fork-session\n--session-id\n{}\n",
        fork["id"].as_str().unwrap()
    )));
    let listed = home.run(&["list", "sessions", "--json"]);
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert!(
        listed["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["forked_from"] == id)
    );
    assert!(String::from_utf8_lossy(&home.run(&["list", "sessions"]).stdout).contains(&id[..8]));
}

#[test]
fn codex_fork_replays_overrides_and_links_its_own_rollout() {
    let home = TestHome::new();
    home.config("[harnesses.codex]\nhome = 'isolated'\n");
    assert!(
        home.run(&["--codex", "-m", "gpt-5", "-e", "high"])
            .status
            .success()
    );
    let parent_path = home.session_path();
    let before = fs::read(&parent_path).unwrap();
    let parent: serde_json::Value = serde_json::from_slice(&before).unwrap();
    let id = parent["id"].as_str().unwrap();
    let native = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let directory = home.dir.path().join("state/ayran/homes/codex/sessions");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("rollout-parent.jsonl"), serde_json::json!({"type":"session_meta","payload":{"id":native,"cwd":parent["cwd"],"timestamp":parent["started_at"],"source":"cli"}}).to_string()).unwrap();
    home.config("");
    let dry = home.run(&[
        "resume",
        "--last",
        "--fork",
        "--codex",
        "--dry-run",
        "--json",
    ]);
    assert!(dry.status.success(), "{dry:?}");
    let dry: serde_json::Value = serde_json::from_slice(&dry.stdout).unwrap();
    assert_eq!(dry["argv"][1], "fork");
    assert_eq!(dry["argv"][2], native);
    assert_ne!(dry["session_id"], id);
    assert_eq!(fs::read(&parent_path).unwrap(), before);
    let result = home.run(&["resume", id, "--fork", "-m", "changed", "--", "prompt"]);
    assert!(result.status.success(), "{result:?}");
    let args = home.raw_record();
    assert!(args.starts_with(&format!("fork\n{native}\n")), "{args}");
    assert!(args.contains("changed\n"));
    assert!(args.contains("model_reasoning_effort=\"high\""));
    assert!(args.ends_with("prompt\n"));
    assert!(!args.contains("--session-id") && !args.contains("--fork-session"));
    let fork_path = fs::read_dir(parent_path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path != &parent_path)
        .unwrap();
    let fork: serde_json::Value = serde_json::from_slice(&fs::read(&fork_path).unwrap()).unwrap();
    assert_eq!(fork["forked_from"], id);
    assert_eq!(fork["native_id"], serde_json::Value::Null);
    assert_eq!(fork["home"], "isolated");
    assert_eq!(fork["request"]["model"], "changed");
    let mut after: serde_json::Value =
        serde_json::from_slice(&fs::read(&parent_path).unwrap()).unwrap();
    after["last_used_at"] = parent["last_used_at"].clone();
    assert_eq!(after, parent);
    let fork_native = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    fs::write(directory.join("rollout-fork.jsonl"), serde_json::json!({"type":"session_meta","payload":{"id":fork_native,"cwd":fork["cwd"],"timestamp":fork["started_at"],"source":"cli","forked_from_id":native}}).to_string()).unwrap();
    let listed = home.run(&["list", "sessions", "--json"]);
    assert!(listed.status.success(), "{listed:?}");
    let linked: serde_json::Value = serde_json::from_slice(&fs::read(&fork_path).unwrap()).unwrap();
    assert_eq!(linked["native_id"], fork_native);
    let parent_linked: serde_json::Value =
        serde_json::from_slice(&fs::read(&parent_path).unwrap()).unwrap();
    assert_eq!(parent_linked["native_id"], native);
    let linked_bytes = fs::read(&parent_path).unwrap();
    let resumed = home.run(&["resume", fork["id"].as_str().unwrap()]);
    assert!(resumed.status.success(), "{resumed:?}");
    assert!(
        home.raw_record()
            .starts_with(&format!("resume\n{fork_native}\n"))
    );
    assert_eq!(fs::read(&parent_path).unwrap(), linked_bytes);
}

#[test]
fn copilot_fork_is_unsupported_without_writing_records() {
    let home = TestHome::new();
    assert!(home.run(&["--copilot"]).status.success());
    let path = home.session_path();
    let before = fs::read(&path).unwrap();
    for flags in [vec![], vec!["--dry-run", "--json"]] {
        let mut args = vec!["resume", "--last", "--fork"];
        args.extend(flags);
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("fork-unsupported"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("use /fork inside the session"));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }
}

#[test]
fn failed_fork_preserves_parent_and_removes_child_record() {
    for harness in ["claude", "codex"] {
        let home = TestHome::new();
        assert!(home.run(&[&format!("--{harness}")]).status.success());
        let path = home.session_path();
        let before = fs::read(&path).unwrap();
        let parent: serde_json::Value = serde_json::from_slice(&before).unwrap();
        let id = parent["id"].as_str().unwrap();
        let mut args = vec!["resume", id, "--fork", "-m", "changed"];
        if harness == "codex" {
            args.extend(["--native", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"]);
        }
        // Resolution failure occurs before any records are written.
        args.extend(["--skill", "missing"]);
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert_eq!(fs::read(&path).unwrap(), before);
        args.truncate(args.len() - 2);
        fs::remove_dir_all(home.dir.path().join("cache")).ok();
        fs::write(
            home.dir.path().join(format!("bin/{harness}")),
            "#!/bin/sh\nprintf '9.0.0\\n'\n/bin/rm -- \"$0\"\n",
        )
        .unwrap();
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("harness-not-found"));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }
}

#[test]
fn fork_keeps_an_old_parent_that_would_otherwise_be_pruned() {
    let home = TestHome::new();
    assert!(home.run(&["--claude"]).status.success());
    let path = home.session_path();
    let mut parent: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    parent["last_used_at"] = serde_json::json!("2000-01-01T00:00:00Z");
    fs::write(&path, parent.to_string()).unwrap();
    let output = home.run(&["resume", parent["id"].as_str().unwrap(), "--fork"]);
    assert!(output.status.success(), "{output:?}");
    let after: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_ne!(after["last_used_at"], parent["last_used_at"]);
}

#[test]
fn codex_resume_links_without_dry_run_writes_and_replays_overrides() {
    let home = TestHome::new();
    home.config("[harnesses.codex]\nhome = 'isolated'\n");
    assert!(
        home.run(&["--codex", "-m", "gpt-5", "-e", "high"])
            .status
            .success()
    );
    let path = home.session_path();
    let before = fs::read(&path).unwrap();
    let record: serde_json::Value = serde_json::from_slice(&before).unwrap();
    let id = record["id"].as_str().unwrap();
    let native = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let isolated = home.dir.path().join("state/ayran/homes/codex");
    fs::create_dir_all(isolated.join("sessions/2026/10/01")).unwrap();
    fs::write(isolated.join("sessions/2026/10/01/rollout-main.jsonl"), serde_json::json!({"type":"session_meta","payload":{"id":native,"session_id":native,"timestamp":record["started_at"],"cwd":record["cwd"],"source":"cli"}}).to_string()).unwrap();
    fs::write(
        isolated.join("config.toml"),
        "[mcp_servers.hidden]\ncommand = 'hidden'\n",
    )
    .unwrap();
    home.config("");
    let output = home.run(&["resume", id, "--dry-run", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let dry: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let argv = dry["argv"].as_array().unwrap();
    assert_eq!(
        &argv[1..3],
        &[serde_json::json!("resume"), serde_json::json!(native)]
    );
    assert!(argv.iter().any(|arg| arg == "gpt-5"));
    assert!(
        argv.iter()
            .any(|arg| arg == "model_reasoning_effort=\"high\"")
    );
    assert!(
        argv.iter()
            .any(|arg| arg == "mcp_servers.hidden.enabled=false"),
        "{argv:?}"
    );
    for code in ["home-mode-changed", "codex-resume-override"] {
        assert!(
            dry["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| diagnostic["code"] == code),
            "{dry}"
        );
    }
    assert_eq!(fs::read(&path).unwrap(), before);
    let output = home.run(&["resume", id]);
    assert!(output.status.success(), "{output:?}");
    assert!(
        home.raw_record()
            .starts_with(&format!("resume\n{native}\n"))
    );
    assert_eq!(
        fs::read_to_string(home.dir.path().join("home_record"))
            .unwrap()
            .trim(),
        isolated.to_str().unwrap()
    );
    let after: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(after["native_id"], native);
}

#[test]
fn launch_prunes_by_last_use_and_preserves_recent_or_unageable_corrupt_records() {
    let home = TestHome::new();
    assert!(home.run(&["--claude"]).status.success());
    let old = home.session_path();
    let original = fs::read(&old).unwrap();
    let mut record: serde_json::Value = serde_json::from_slice(&original).unwrap();
    record["last_used_at"] = "2000-01-01T00:00:00Z".into();
    fs::write(&old, serde_json::to_vec(&record).unwrap()).unwrap();
    let directory = old.parent().unwrap();
    let corrupt = directory.join("corrupt.json");
    let aged_corrupt = directory.join("aged-corrupt.json");
    fs::write(&corrupt, "broken").unwrap();
    fs::write(&aged_corrupt, r#"{"last_used_at":"2000-01-01T00:00:00Z"}"#).unwrap();
    let recent = directory.join("11111111-1111-4111-8111-111111111111.json");
    record["id"] = "11111111-1111-4111-8111-111111111111".into();
    record["last_used_at"] =
        serde_json::from_slice::<serde_json::Value>(&original).unwrap()["last_used_at"].clone();
    fs::write(&recent, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(home.run(&["--codex", "--dry-run"]).status.success());
    assert!(old.exists());
    assert!(home.run(&["--codex"]).status.success());
    assert!(!old.exists());
    assert!(!aged_corrupt.exists());
    assert!(recent.exists());
    assert!(corrupt.exists());
}

#[test]
fn codex_launch_can_be_listed_without_native_identity_and_dry_run_writes_nothing() {
    let home = TestHome::new();
    let dry = home.run(&["--codex", "--dry-run", "--json"]);
    assert!(dry.status.success(), "{dry:?}");
    let dry: serde_json::Value = serde_json::from_slice(&dry.stdout).unwrap();
    assert!(dry["session_id"].as_str().is_some());
    assert!(!home.dir.path().join("state/ayran/sessions").exists());
    assert!(home.run(&["--codex", "-m", "gpt-5"]).status.success());
    let output = home.run(&["list", "sessions", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let listed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let record = &listed["sessions"][0];
    assert_eq!(record["harness"], "codex");
    assert_eq!(record["native_id"], serde_json::Value::Null);
    assert_eq!(record["link"], "unlinked");
    assert_eq!(record["request"]["model"], "gpt-5");
    assert!(!home.raw_record().contains("--session-id"));
}

#[test]
fn copilot_launch_records_a_resumable_session_and_dry_run_records_nothing() {
    let home = TestHome::new();
    let output = home.run(&["--copilot", "--dry-run", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let dry: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let id = dry["session_id"].as_str().unwrap();
    assert_eq!(uuid::Uuid::parse_str(id).unwrap().get_version_num(), 4);
    assert_eq!(dry["argv"][1], "--session-id");
    assert_eq!(dry["argv"][2], id);
    assert!(!home.dir.path().join("state/ayran/sessions").exists());

    let output = home.run(&["--copilot", "-m", "gpt-5", "--", "private-prompt"]);
    assert!(output.status.success(), "{output:?}");
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(home.session_path()).unwrap()).unwrap();
    assert_eq!(record["native_id"], record["id"]);
    assert_eq!(record["harness"], "copilot");
    assert_eq!(record["home"], "shared");
    assert_eq!(record["request"]["model"], "gpt-5");
    assert!(record["request"].get("passthrough").is_none());
    assert!(home.raw_record().starts_with(&format!(
        "--session-id\n{}\n",
        record["id"].as_str().unwrap()
    )));
}

#[test]
fn copilot_resume_regenerates_overrides_in_the_recorded_home_and_directory() {
    let home = TestHome::new();
    home.config("[harnesses.copilot]\nhome = 'isolated'\n");
    assert!(home.run(&["--copilot"]).status.success());
    let path = home.session_path();
    let before = fs::read(&path).unwrap();
    let record: serde_json::Value = serde_json::from_slice(&before).unwrap();
    let id = record["id"].as_str().unwrap();
    let isolated = home.dir.path().join("state/ayran/homes/copilot");
    fs::create_dir_all(isolated.join("installed-plugins/market/unselected")).unwrap();
    fs::write(isolated.join("mcp-config.json"), r#"{"mcpServers":{"selected":{"command":"selected","tools":["*"]},"hidden":{"command":"hidden","tools":["*"]}}}"#).unwrap();
    let plugin = home.dir.path().join("tools/plugin");
    fs::create_dir_all(plugin.join(".github/plugin")).unwrap();
    fs::write(
        plugin.join(".github/plugin/plugin.json"),
        r#"{"name":"fixture","version":"1.0.0"}"#,
    )
    .unwrap();
    home.config(&format!("[harnesses.copilot]\nmodel = 'gpt-5'\neffort = 'high'\n[plugins.p]\ncopilot = {{ path = '{}' }}\ndefault = true\n[mcp.native]\ncopilot = 'selected'\ndefault = true\n[mcp.extra]\ncopilot = {{ command = 'extra' }}\ndefault = true\n", plugin.display()));
    let elsewhere = home.dir.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    let output = home
        .command()
        .current_dir(&elsewhere)
        .args(["resume", &id[..8], "--dry-run", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let dry: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(dry["session_id"], id);
    assert_eq!(dry["cwd"], home.dir.path().to_str().unwrap());
    assert_eq!(dry["env"]["COPILOT_HOME"], isolated.to_str().unwrap());
    assert_eq!(fs::read(&path).unwrap(), before);

    let output = home
        .command()
        .current_dir(&elsewhere)
        .args(["resume", id, "-m", "gpt-5.4"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let raw = home.raw_record();
    let args: Vec<_> = raw.lines().collect();
    assert!(
        args.windows(2).any(|pair| pair == ["--resume", id]),
        "{raw}"
    );
    assert!(!args.contains(&"--session-id"));
    for pair in [
        ["--model", "gpt-5.4"],
        ["--reasoning-effort", "high"],
        ["--plugin-dir", plugin.to_str().unwrap()],
        ["--disable-mcp-server", "hidden"],
    ] {
        assert!(args.windows(2).any(|actual| actual == pair), "{raw}");
    }
    let config = args[args
        .iter()
        .position(|arg| *arg == "--additional-mcp-config")
        .unwrap()
        + 1];
    let generated: serde_json::Value =
        serde_json::from_slice(&fs::read(config.strip_prefix('@').unwrap()).unwrap()).unwrap();
    assert_eq!(generated["mcpServers"]["extra"]["command"], "extra");
    assert_eq!(
        fs::read_to_string(home.dir.path().join("plugin_env_record"))
            .unwrap()
            .trim(),
        "true"
    );
    assert_eq!(
        fs::read_to_string(home.dir.path().join("home_record"))
            .unwrap()
            .trim(),
        isolated.to_str().unwrap()
    );
    assert_eq!(
        fs::read_to_string(home.dir.path().join("cwd_record"))
            .unwrap()
            .trim(),
        home.dir.path().to_str().unwrap()
    );
    let updated: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(updated["request"]["model"], "gpt-5.4");
    assert_eq!(updated["started_at"], record["started_at"]);
    assert_ne!(updated["last_used_at"], record["last_used_at"]);
    assert!(home.run(&["resume", id]).status.success());
    assert!(home.raw_record().contains("--model\ngpt-5.4\n"));
}

#[test]
fn resume_ignores_corruption_in_an_unrelated_session_record() {
    let home = TestHome::new();
    assert!(home.run(&["--claude"]).status.success());
    let path = home.session_path();
    let record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let id = record["id"].as_str().unwrap();
    let unrelated = if id.starts_with('0') {
        "11111111-1111-4111-8111-111111111111"
    } else {
        "00000000-0000-4000-8000-000000000000"
    };
    fs::write(path.with_file_name(format!("{unrelated}.json")), "broken").unwrap();
    let output = home.run(&["resume", id, "--dry-run"]);
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn resume_reports_unknown_ambiguous_and_missing_directory_sessions() {
    let home = TestHome::new();
    let output = home.run(&["resume", "unknown"]);
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown-session"));
    assert!(home.run(&["--claude"]).status.success());
    let path = home.session_path();
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let id = record["id"].as_str().unwrap().to_owned();
    let second_id = format!("{}{}", &id[..35], if id.ends_with('0') { '1' } else { '0' });
    record["id"] = second_id.clone().into();
    record["native_id"] = second_id.clone().into();
    let second_path = path.with_file_name(format!("{second_id}.json"));
    fs::write(&second_path, serde_json::to_vec(&record).unwrap()).unwrap();
    let output = home.run(&["resume", &id[..8]]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("ambiguous Session prefix"));
    assert!(String::from_utf8_lossy(&output.stderr).contains(&id[..8]));
    fs::remove_file(second_path).unwrap();
    record["id"] = id.clone().into();
    record["native_id"] = id.clone().into();
    record["cwd"] = home
        .dir
        .path()
        .join("gone")
        .to_string_lossy()
        .into_owned()
        .into();
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    let output = home.run(&["resume", &id]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("cwd-not-found"));
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn resume_validates_the_recorded_alias_and_rejects_alias_or_harness_changes() {
    let home = TestHome::new();
    home.config("[aliases.work]\nharness = 'claude'\nmodel = 'opus'\n");
    assert!(home.run(&["--alias", "work"]).status.success());
    let path = home.session_path();
    let original = fs::read(&path).unwrap();
    let record: serde_json::Value = serde_json::from_slice(&original).unwrap();
    assert_eq!(record["request"]["alias"], "work");
    assert!(record["request"]["model"].is_null());
    let id = record["id"].as_str().unwrap();
    for flags in [
        vec!["--alias", "work"],
        vec!["--codex"],
        vec!["--harness", "copilot"],
    ] {
        let mut args = vec!["resume", id];
        args.extend(flags);
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("usage"));
    }
    assert!(
        home.run(&["resume", id, "--claude", "--dry-run"])
            .status
            .success()
    );
    for (config, code) in [
        ("", "unknown-alias"),
        (
            "[aliases.work]\nharness = 'codex'\n",
            "alias-harness-changed",
        ),
    ] {
        home.config(config);
        let output = home.run(&["resume", id]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        let message = String::from_utf8_lossy(&output.stderr);
        assert!(message.contains(code), "{message}");
        if code == "unknown-alias" {
            assert!(message.contains("restore the Alias"));
        }
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}

#[test]
fn resume_uses_the_recorded_home_and_regenerates_capabilities_from_current_config() {
    let home = TestHome::new();
    home.config("[harnesses.claude]\nhome = 'isolated'\nmodel = 'opus'\n");
    assert!(home.run(&["--claude"]).status.success());
    let path = home.session_path();
    let original = fs::read(&path).unwrap();
    let record: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let id = record["id"].as_str().unwrap();
    home.config(
        "[harnesses.claude]\nmodel = 'sonnet'\n[skills.x]\nclaude = 'personal'\ndefault = true\n",
    );
    home.skill("state/ayran/homes/claude/skills/personal", "personal");
    home.skill("state/ayran/homes/claude/skills/hidden", "hidden");
    let output = home.run(&["resume", id, "--dry-run", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["session_id"], id);
    assert_eq!(
        json["env"]["CLAUDE_CONFIG_DIR"],
        home.dir
            .path()
            .join("state/ayran/homes/claude")
            .to_str()
            .unwrap()
    );
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "home-mode-changed" && d["severity"] == "note")
    );
    let argv = json["argv"].as_array().unwrap();
    assert!(argv.contains(&serde_json::json!("sonnet")));
    let settings: serde_json::Value = serde_json::from_str(
        argv[argv.iter().position(|arg| arg == "--settings").unwrap() + 1]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert!(settings["skillOverrides"].get("personal").is_none());
    assert_eq!(settings["skillOverrides"]["hidden"], "off");
    assert_eq!(fs::read(&path).unwrap(), original);
    let quiet = home.run(&["resume", id, "--dry-run", "-q"]);
    assert!(quiet.status.success());
    assert!(quiet.stderr.is_empty(), "{quiet:?}");
    let output = home.run(&["resume", id]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        fs::read_to_string(home.dir.path().join("home_record"))
            .unwrap()
            .trim(),
        home.dir
            .path()
            .join("state/ayran/homes/claude")
            .to_str()
            .unwrap()
    );
}

#[test]
fn exec_failure_removes_a_new_record_and_restores_a_resumed_record() {
    for harness in ["claude", "copilot", "codex"] {
        let flag = format!("--{harness}");
        for resume in [false, true] {
            let home = TestHome::new();
            let existing = if resume {
                assert!(home.run(&[&flag]).status.success());
                let path = home.session_path();
                let bytes = fs::read(&path).unwrap();
                let record: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                Some((path, bytes, record["id"].as_str().unwrap().to_owned()))
            } else {
                None
            };
            fs::remove_dir_all(home.dir.path().join("cache")).ok();
            // Version probing succeeds, then removes the binary so exec itself fails.
            fs::write(
                home.dir.path().join(format!("bin/{harness}")),
                "#!/bin/sh\nprintf '9.0.0\\n'\n/bin/rm -- \"$0\"\n",
            )
            .unwrap();
            let output = if let Some((_, _, id)) = &existing {
                if harness == "codex" {
                    home.run(&[
                        "resume",
                        id,
                        "-m",
                        "changed",
                        "--native",
                        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
                    ])
                } else {
                    home.run(&["resume", id, "-m", "changed"])
                }
            } else {
                home.run(&[&flag])
            };
            assert_eq!(output.status.code(), Some(3), "{output:?}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("harness-not-found"));
            if let Some((path, bytes, _)) = existing {
                assert_eq!(fs::read(path).unwrap(), bytes);
            } else {
                assert_eq!(
                    fs::read_dir(home.dir.path().join("state/ayran/sessions"))
                        .unwrap()
                        .count(),
                    0
                );
            }
        }
    }
}

#[test]
fn failed_resume_binding_check_and_dry_run_leave_the_record_unchanged() {
    for harness in ["claude", "copilot"] {
        let flag = format!("--{harness}");
        let home = TestHome::new();
        home.config(&format!("[skills.x]\n{harness} = false\n"));
        assert!(home.run(&[&flag]).status.success());
        let path = home.session_path();
        let bytes = fs::read(&path).unwrap();
        let record: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let id = record["id"].as_str().unwrap();
        home.config("[skills.x]\ncodex = 'x'\n");
        let output = home.run(&["resume", id, "--skill", "x"]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("missing-binding"));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let output = home.run(&["resume", id, "-e", "high", "--dry-run"]);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn launch_json_exposes_a_fresh_unrecorded_session() {
    let home = TestHome::new();
    let first = home.run(&["--claude", "--dry-run", "--json"]);
    let second = home.run(&["--claude", "--dry-run", "--json"]);
    let first: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    let second: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    let id = first["session_id"].as_str().unwrap();
    assert_eq!(uuid::Uuid::parse_str(id).unwrap().get_version_num(), 4);
    assert_ne!(first["session_id"], second["session_id"]);
    assert_eq!(first["argv"][1], "--session-id");
    assert_eq!(first["argv"][2], id);
    assert_eq!(first["cwd"], home.dir.path().to_str().unwrap());
    assert!(!home.dir.path().join("state/ayran/sessions").exists());
}

#[test]
fn resume_disables_a_vanished_selection_and_failed_resolution_preserves_bytes() {
    for harness in ["claude", "copilot"] {
        let flag = format!("--{harness}");
        let home = TestHome::new();
        home.skill(&format!(".{harness}/skills/x"), "x");
        home.skill(&format!(".{harness}/skills/y"), "y");
        home.config(&format!(
            "[skills.x]\n{harness} = 'x'\n[skills.y]\n{harness} = 'y'\n"
        ));
        assert!(home.run(&[&flag, "--skill", "x,y,x"]).status.success());
        let path = home.session_path();
        let original = fs::read(&path).unwrap();
        let record: serde_json::Value = serde_json::from_slice(&original).unwrap();
        assert_eq!(record["request"]["skills"], serde_json::json!(["x", "y"]));
        let id = record["id"].as_str().unwrap();
        home.config(&format!("[skills.y]\n{harness} = 'y'\n"));
        let output = home.run(&["resume", id]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("--no-skill x to drop"),
            "{output:?}"
        );
        let output = home.run(&["resume", id, "--no-skill", "x"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let updated: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(updated["request"]["skills"], serde_json::json!(["y"]));
        assert_eq!(updated["request"]["no_skills"], serde_json::json!(["x"]));
        assert!(home.run(&["resume", id]).status.success());
    }
}

#[test]
fn resume_replays_the_request_from_the_recorded_directory() {
    let home = TestHome::new();
    fs::write(
        home.dir.path().join("ayran.toml"),
        "[harnesses.claude]\neffort = 'high'\n",
    )
    .unwrap();
    let output = home.run(&["--claude", "-m", "opus"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let path = fs::read_dir(home.dir.path().join("state/ayran/sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let id = record["id"].as_str().unwrap();
    let other = home.dir.path().join("elsewhere");
    fs::create_dir(&other).unwrap();
    fs::write(
        other.join("ayran.toml"),
        "[harnesses.claude]\neffort = 'low'\n",
    )
    .unwrap();
    let output = home
        .command()
        .current_dir(&other)
        .args(["resume", &id[..8], "-m", "sonnet", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let line = String::from_utf8_lossy(&output.stdout);
    assert!(line.starts_with("cd "), "{line}");
    assert!(line.contains(&format!("--resume {id}")), "{line}");
    assert!(line.contains("--model sonnet"), "{line}");
    assert!(!line.contains("--session-id"), "{line}");
    assert!(line.contains("--effort high"), "{line}");
    let output = home
        .command()
        .current_dir(&other)
        .args(["resume", id, "-m", "sonnet"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(home.record().contains(&format!("--resume\n{id}\n")));
    assert_eq!(
        fs::read_to_string(home.dir.path().join("cwd_record"))
            .unwrap()
            .trim(),
        home.dir.path().to_str().unwrap()
    );
    let updated: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(updated["request"]["model"], "sonnet");
    assert_eq!(updated["started_at"], record["started_at"]);
    assert_ne!(updated["last_used_at"], record["last_used_at"]);
}

#[test]
fn claude_launch_records_a_resumable_session_and_dry_run_records_nothing() {
    let home = TestHome::new();
    let output = home.run(&["--claude", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("--session-id"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("(not recorded)"));
    assert!(!home.dir.path().join("state/ayran/sessions").exists());
    let output = home.run(&["--claude", "-m", "opus", "--", "private-prompt"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let records: Vec<_> = fs::read_dir(home.dir.path().join("state/ayran/sessions"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(records.len(), 1);
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(&records[0]).unwrap()).unwrap();
    assert_eq!(record["version"], 1);
    assert_eq!(record["native_id"], record["id"]);
    assert_eq!(record["harness"], "claude");
    assert_eq!(record["home"], "shared");
    assert_eq!(record["request"]["model"], "opus");
    assert!(record["request"].get("harness").is_none());
    assert!(record["request"].get("passthrough").is_none());
    assert!(home.raw_record().contains(record["id"].as_str().unwrap()));
}

#[test]
fn isolated_copilot_ignores_unrelated_broken_shared_installs() {
    let home = TestHome::new();
    home.config("[harnesses.copilot]\nhome = 'isolated'\n");
    use std::os::unix::ffi::OsStringExt;
    let broken = home
        .dir
        .path()
        .join(".copilot/installed-plugins")
        .join(std::ffi::OsString::from_vec(vec![0xff]));
    fs::create_dir_all(broken).unwrap();
    let output = home.run(&["--copilot"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test]
fn isolated_copilot_native_bindings_require_a_shared_home_install() {
    let home = TestHome::new();
    home.config("[harnesses.copilot]\nhome = 'isolated'\n[plugins.p]\ncopilot = 'p@market'\n");
    fs::create_dir_all(
        home.dir
            .path()
            .join("state/ayran/homes/copilot/installed-plugins/market/p"),
    )
    .unwrap();
    let output = home.run(&["--copilot", "--plugin", "p", "--dry-run"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("native-not-found"),
        "{output:?}"
    );
}

#[test]
fn isolated_enumeration_uses_only_the_session_home_for_skills_and_mcp() {
    for harness in ["claude", "codex", "copilot"] {
        let home = TestHome::new();
        home.config(&format!("[harnesses.{harness}]\nhome = 'isolated'\n[skills.s]\n{harness} = 'personal'\n[mcp.m]\n{harness} = 'personal'\n"));
        home.skill(&format!(".{harness}/skills/personal"), "personal");
        home.skill(".agents/skills/shared", "shared");
        let mcp_path = match harness {
            "claude" => ".claude.json",
            "codex" => ".codex/config.toml",
            _ => ".copilot/mcp-config.json",
        };
        let mcp = if harness == "codex" {
            "[mcp_servers.personal]\ncommand = 'fixture'\n"
        } else {
            r#"{"mcpServers":{"personal":{"command":"fixture"}}}"#
        };
        let path = home.dir.path().join(mcp_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, mcp).unwrap();
        let flag = format!("--{harness}");
        let output = home.run(&[&flag, "--dry-run"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let trace = String::from_utf8_lossy(&output.stderr);
        assert!(
            !trace.contains("Skill personal:")
                && !trace.contains("Skill shared:")
                && !trace.contains("MCP server personal:"),
            "{trace}"
        );
        for selection in ["--skill", "--mcp"] {
            let output = home.run(&[
                &flag,
                selection,
                if selection == "--skill" { "s" } else { "m" },
                "--dry-run",
            ]);
            assert_eq!(output.status.code(), Some(3), "{output:?}");
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("native-not-found"),
                "{output:?}"
            );
        }
        home.skill(
            &format!("state/ayran/homes/{harness}/skills/personal"),
            "personal",
        );
        let isolated = home.dir.path().join("state/ayran/homes").join(harness);
        let target = match harness {
            "claude" => ".claude.json",
            "codex" => "config.toml",
            _ => "mcp-config.json",
        };
        fs::write(isolated.join(target), mcp).unwrap();
        let before = snapshot(&isolated);
        let output = home.run(&[&flag, "--skill", "s", "--mcp", "m"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(snapshot(&isolated), before);
    }
}

#[test]
fn isolated_plugins_ignore_real_installs_and_use_isolated_installs() {
    for harness in ["claude", "codex"] {
        let home = TestHome::new();
        home.config(&format!(
            "[harnesses.{harness}]\nhome = 'isolated'\n[plugins.p]\n{harness} = 'personal@market'\n"
        ));
        let registry = if harness == "claude" {
            r#"{"version":2,"plugins":{"personal@market":[{"scope":"user"}]}}"#
        } else {
            "[plugins.'personal@market']\nenabled = true\n"
        };
        if harness == "claude" {
            home.claude_installs(registry);
        } else {
            home.codex_config(registry);
        }
        let flag = format!("--{harness}");
        let output = home.run(&[&flag, "--dry-run"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("Plugin personal@market: hidden")
        );
        let output = home.run(&[&flag, "--plugin", "p", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("native-not-found"));
        let isolated = home.dir.path().join("state/ayran/homes").join(harness);
        let relative = if harness == "claude" {
            "plugins/installed_plugins.json"
        } else {
            "config.toml"
        };
        let path = isolated.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, registry).unwrap();
        if harness == "codex" {
            fs::create_dir_all(isolated.join("plugins/cache/market/personal/local")).unwrap();
        }
        let output = home.run(&[&flag, "--plugin", "p"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let output = home.run(&[&flag, "--dry-run"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("Plugin personal@market: hidden"));
    }
}

#[test]
fn shared_home_preserves_inherited_home_variables_and_isolated_home_has_a_state_fallback() {
    for (harness, variable) in [
        ("claude", "CLAUDE_CONFIG_DIR"),
        ("codex", "CODEX_HOME"),
        ("copilot", "COPILOT_HOME"),
    ] {
        let home = TestHome::new();
        home.config(&format!("[harnesses.{harness}]\nhome = 'shared'\n"));
        let inherited = home.dir.path().join("custom");
        let flag = format!("--{harness}");
        let output = home
            .command()
            .env(variable, &inherited)
            .arg(&flag)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            fs::read_to_string(home.dir.path().join("home_record"))
                .unwrap()
                .trim(),
            inherited.to_str().unwrap()
        );
        assert!(!inherited.exists());
        home.config(&format!("[harnesses.{harness}]\nhome = 'isolated'\n"));
        let output = home
            .command()
            .env_remove("XDG_STATE_HOME")
            .arg(&flag)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(
            home.dir
                .path()
                .join(".local/state/ayran/homes")
                .join(harness)
                .is_dir()
        );
    }
}

#[test]
fn isolated_copilot_native_plugins_use_shared_install_paths_without_hiding_shared_items() {
    let home = TestHome::new();
    home.config("[harnesses.copilot]\nhome = 'isolated'\n[plugins.p]\ncopilot = 'p@market'\n");
    let native = home.dir.path().join(".copilot/installed-plugins/market/p");
    fs::create_dir_all(&native).unwrap();
    fs::create_dir_all(
        home.dir
            .path()
            .join(".copilot/installed-plugins/_direct/unselected"),
    )
    .unwrap();
    let output = home.run(&["--copilot", "--plugin", "p"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(home.record().contains(native.to_str().unwrap()));
    fs::create_dir_all(
        home.dir
            .path()
            .join("state/ayran/homes/copilot/installed-plugins/market/p"),
    )
    .unwrap();
    let output = home.run(&["--copilot", "--plugin", "p"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(home.record().contains(native.to_str().unwrap()));
    assert_eq!(
        fs::read_to_string(home.dir.path().join("plugin_env_record")).unwrap(),
        "true\n"
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("copilot-direct-plugin"));
    fs::remove_dir_all(
        home.dir
            .path()
            .join("state/ayran/homes/copilot/installed-plugins"),
    )
    .unwrap();
    let output = home.run(&["--copilot"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(!home.record().contains("--plugin-dir"));
    assert_eq!(
        fs::read_to_string(home.dir.path().join("plugin_env_record")).unwrap(),
        "unset\n"
    );
}

#[test]
fn isolated_homes_are_empty_and_override_inherited_harness_homes() {
    for (harness, variable) in [
        ("claude", "CLAUDE_CONFIG_DIR"),
        ("codex", "CODEX_HOME"),
        ("copilot", "COPILOT_HOME"),
    ] {
        let home = TestHome::new();
        home.config(&format!("[harnesses.{harness}]\nhome = 'isolated'\n"));
        let isolated = home.dir.path().join("state/ayran/homes").join(harness);
        let flag = format!("--{harness}");
        let output = home
            .command()
            .env(variable, home.dir.path().join("inherited"))
            .args([&flag, "--dry-run"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains(&format!("{variable}={}", isolated.display())),
            "{output:?}"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("not created yet"),
            "{output:?}"
        );
        assert!(!isolated.exists());
        let output = home
            .command()
            .env(variable, home.dir.path().join("inherited"))
            .arg(&flag)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            fs::read_to_string(home.dir.path().join("home_record"))
                .unwrap()
                .trim(),
            isolated.to_str().unwrap()
        );
        assert_eq!(fs::read_dir(&isolated).unwrap().count(), 0);
        fs::write(isolated.join("sentinel"), "untouched").unwrap();
        assert_eq!(home.run(&[&flag]).status.code(), Some(0));
        assert_eq!(
            fs::read_to_string(isolated.join("sentinel")).unwrap(),
            "untouched"
        );
    }
}

#[test]
fn codex_account_connectors_are_hidden_without_a_selected_native_binding() {
    let home = TestHome::new();
    home.config(
        "[mcp.accounts]\ncodex = 'codex_apps'\n[mcp.other]\ncodex = { command = 'fixture' }\n",
    );
    home.codex_config("[features]\napps = true\n");
    for flags in [vec![], vec!["--mcp", "other"]] {
        let mut args = vec!["--codex"];
        args.extend(flags);
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(home.record().contains("--disable\napps\n"));
        args.push("--dry-run");
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("--disable apps"));
    }
    assert_eq!(
        fs::read_to_string(home.dir.path().join(".codex/config.toml")).unwrap(),
        "[features]\napps = true\n"
    );
}

#[test]
fn codex_account_connectors_can_be_selected_without_an_enumerated_server() {
    for contents in ["", "[features]\napps = false\n"] {
        let home = TestHome::new();
        home.config("[mcp.accounts]\ncodex = 'codex_apps'\n[mcp.same]\ncodex = 'codex_apps'\n");
        home.codex_config(contents);
        let output = home.run(&["--codex", "--mcp", "accounts,same"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let record = home.record();
        assert!(!record.contains("--disable\napps\n"), "{record}");
        assert!(!record.contains("mcp_servers.codex_apps"), "{record}");
        let output = home.run(&["--codex", "--mcp", "accounts", "--dry-run"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("--disable apps"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("native codex_apps"));
        assert_eq!(
            fs::read_to_string(home.dir.path().join(".codex/config.toml")).unwrap(),
            contents
        );
    }
}

#[test]
fn copilot_mcp_plugin_sources_follow_verified_loader_precedence() {
    for source in ["default", "github", "inline", "path", "agent", "precedence"] {
        let home = TestHome::new();
        let plugin = home.dir.path().join("plugin");
        fs::create_dir_all(plugin.join(".plugin")).unwrap();
        let servers = r#"{"b":{"command":"bundled","tools":["*"]}}"#;
        let config = format!("{{\"mcpServers\":{servers}}}");
        let manifest = match source {
            "inline" => format!("{{\"name\":\"p\",\"mcpServers\":{servers}}}"),
            "path" => r#"{"name":"p","mcpServers":"./custom.json"}"#.into(),
            "agent" => r#"{"name":"p","$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json"}"#.into(),
            _ => r#"{"name":"p","mcpServers":{"ignored":{"command":"ignored"}}}"#.into(),
        };
        fs::write(plugin.join(".plugin/plugin.json"), manifest).unwrap();
        let payload = match source {
            "default" | "precedence" => Some(".mcp.json"),
            "github" => Some(".github/mcp.json"),
            "path" => Some("custom.json"),
            "agent" => Some("mcp.json"),
            _ => None,
        };
        if let Some(payload) = payload {
            let path = plugin.join(payload);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, &config).unwrap();
        }
        if source == "precedence" {
            fs::create_dir(plugin.join(".github")).unwrap();
            fs::write(
                plugin.join(".github/mcp.json"),
                "malformed but lower priority",
            )
            .unwrap();
            fs::write(plugin.join("plugin.json"), "malformed but lower priority").unwrap();
        }
        home.config(&format!(
            "[plugins.p]\ncopilot = {{ path = '{}' }}\n",
            plugin.display()
        ));
        let user = home.dir.path().join(".copilot/mcp-config.json");
        fs::create_dir_all(user.parent().unwrap()).unwrap();
        fs::write(
            user,
            r#"{"mcpServers":{"b":{"command":"user"},"ignored":{"command":"user"}}}"#,
        )
        .unwrap();
        let output = home.run(&["--copilot", "--plugin", "p", "--dry-run"]);
        assert_eq!(output.status.code(), Some(0), "{source}: {output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("MCP server b: left visible (plugin-server-shadow)"),
            "{source}: {stderr}"
        );
        assert!(
            stderr.contains("MCP server ignored: hidden"),
            "{source}: {stderr}"
        );
        assert!(home.no_record());
    }
}

#[test]
fn copilot_mcp_unreadable_state_refuses_launch_and_custom_home_is_respected() {
    for contents in [
        "invalid JSON",
        "[]",
        r#"{"mcpServers":[]}"#,
        r#"{"mcpServers":{"x":false}}"#,
    ] {
        let home = TestHome::new();
        let path = home.dir.path().join(".copilot/mcp-config.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        let output = home.run(&["--copilot"]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("enumeration-failed"),
            "{output:?}"
        );
        assert!(home.no_record());
        assert_eq!(fs::read_to_string(path).unwrap(), contents);
    }
    for relative in [
        ".copilot/mcp-config.json",
        ".github/mcp.json",
        "plugin/.mcp.json",
    ] {
        let home = TestHome::new();
        let path = home.dir.path().join(relative);
        fs::create_dir_all(&path).unwrap(); // A directory is unreadable as a config file.
        if relative.starts_with("plugin/") {
            fs::write(
                home.dir.path().join("plugin/plugin.json"),
                r#"{"name":"p"}"#,
            )
            .unwrap();
            home.config(&format!(
                "[plugins.p]\ncopilot = {{ path = '{}' }}\n",
                home.dir.path().join("plugin").display()
            ));
        }
        let args = if relative.starts_with("plugin/") {
            vec!["--copilot", "--plugin", "p"]
        } else {
            vec!["--copilot"]
        };
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(3), "{relative}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("enumeration-failed"),
            "{output:?}"
        );
        assert!(home.no_record());
    }
    let home = TestHome::new();
    home.config("[mcp.x]\ncopilot = 'custom'\n");
    let custom = home.dir.path().join("custom");
    fs::create_dir_all(&custom).unwrap();
    fs::create_dir(home.dir.path().join(".copilot")).unwrap();
    fs::write(
        home.dir.path().join(".copilot/mcp-config.json"),
        "ignored malformed config",
    )
    .unwrap();
    fs::write(
        custom.join("mcp-config.json"),
        r#"{"mcpServers":{"custom":{"command":"custom"}}}"#,
    )
    .unwrap();
    for path in [PathBuf::from("custom"), custom] {
        let output = home
            .command()
            .env("COPILOT_HOME", path)
            .args(["--copilot", "--mcp", "x"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(
            home.record()
                .lines()
                .collect::<Vec<_>>()
                .windows(2)
                .all(|pair| pair != ["--enable-mcp-server", "custom"])
        );
        assert!(!home.record().contains("ignored"));
    }
}

#[test]
fn copilot_mcp_selected_plugin_servers_protect_user_names_and_definitions_warn() {
    for native in [false, true] {
        let home = TestHome::new();
        let plugin = if native {
            home.dir.path().join(".copilot/installed-plugins/market/p")
        } else {
            home.dir.path().join("path-plugin")
        };
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("plugin.json"), r#"{"name":"p"}"#).unwrap();
        fs::write(
            plugin.join(".mcp.json"),
            r#"{"mcpServers":{"b":{"command":"bundled"},"definition":{"command":"bundled"}}}"#,
        )
        .unwrap();
        let binding = if native {
            "'p@market'".into()
        } else {
            format!("{{ path = '{}' }}", plugin.display())
        };
        home.config(&format!("[plugins.p]\ncopilot = {binding}\n[mcp.definition]\ncopilot = {{ command = 'definition' }}\n"));
        let user = home.dir.path().join(".copilot/mcp-config.json");
        fs::create_dir_all(user.parent().unwrap()).unwrap();
        fs::write(
            user,
            r#"{"mcpServers":{"b":{"command":"user"},"other":{"command":"other"}}}"#,
        )
        .unwrap();
        let output = home.run(&["--copilot", "--plugin", "p", "--mcp", "definition"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let record = home.record();
        let args: Vec<_> = record.lines().collect();
        assert!(
            !args
                .windows(2)
                .any(|pair| pair == ["--disable-mcp-server", "b"]),
            "{record}"
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--disable-mcp-server", "other"]),
            "{record}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("plugin-server-shadow:")
                && stderr.contains("warning[plugin-server-shadowed]"),
            "{stderr}"
        );
        let output = home.run(&["--copilot", "--mcp", "definition"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(
            home.record()
                .lines()
                .collect::<Vec<_>>()
                .windows(2)
                .any(|pair| pair == ["--disable-mcp-server", "b"])
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("plugin-server-shadow"));
    }
}

#[test]
fn copilot_mcp_workspace_servers_are_valid_without_trust_and_shared_names_leak() {
    let home = TestHome::new();
    home.config("[mcp.root]\ncopilot = 'root'\n[mcp.nested]\ncopilot = 'nested'\n");
    let user = home.dir.path().join(".copilot/mcp-config.json");
    fs::create_dir_all(user.parent().unwrap()).unwrap();
    fs::write(
        &user,
        r#"{"mcpServers":{"shared":{"command":"user"},"other":{"command":"other"}}}"#,
    )
    .unwrap();
    // Outside the repo boundary must not validate a native Binding.
    fs::write(
        home.dir.path().join(".mcp.json"),
        r#"{"mcpServers":{"outside":{"command":"outside"}}}"#,
    )
    .unwrap();
    let repo = home.dir.path().join("repo");
    let nested = repo.join("nested");
    fs::create_dir_all(repo.join(".github")).unwrap();
    fs::create_dir_all(&nested).unwrap();
    fs::write(repo.join(".git"), "gitdir: fixture").unwrap();
    let original = r#"{"mcpServers":{"root":{"command":"root"},"shared":{"command":"project"}}}"#;
    fs::write(repo.join(".github/mcp.json"), original).unwrap();
    fs::write(
        nested.join(".mcp.json"),
        r#"{"mcpServers":{"nested":{"command":"nested"}}}"#,
    )
    .unwrap();
    let output = home
        .command()
        .current_dir(&nested)
        .args(["--copilot", "--mcp", "root,nested"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let record = home.record();
    let args: Vec<_> = record.lines().collect();
    for name in ["root", "nested"] {
        assert!(
            args.windows(2)
                .all(|pair| pair != ["--enable-mcp-server", name]),
            "{record}"
        );
    }
    assert!(
        args.windows(2)
            .any(|pair| pair == ["--disable-mcp-server", "other"]),
        "{record}"
    );
    assert!(
        !args
            .windows(2)
            .any(|pair| pair == ["--disable-mcp-server", "shared"]),
        "{record}"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("mcp-project-shadow"),
        "{output:?}"
    );
    assert_eq!(
        fs::read_to_string(repo.join(".github/mcp.json")).unwrap(),
        original
    );
    home.config("[mcp.x]\ncopilot = 'outside'\n");
    let output = home
        .command()
        .current_dir(&nested)
        .args(["--copilot", "--mcp", "x", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("native-not-found"));
}

#[test]
fn copilot_mcp_native_selection_enables_selected_and_hides_other_user_servers() {
    let home = TestHome::new();
    home.config("[mcp.selected]\ncopilot = 'a'\n[mcp.same]\ncopilot = 'a'\n");
    let path = home.dir.path().join(".copilot/mcp-config.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let original = r#"{"mcpServers":{"a":{"command":"a","tools":["*"]},"b":{"command":"b","tools":["*"]},"github-mcp-server":{"command":"github","tools":["*"]}}}"#;
    fs::write(&path, original).unwrap();
    fs::write(
        home.dir.path().join(".copilot/settings.json"),
        r#"{"disabledMcpServers":["a"]}"#,
    )
    .unwrap();
    let output = home.run(&["--copilot", "--mcp", "selected,same"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let record = home.record();
    let args: Vec<_> = record.lines().collect();
    assert_eq!(
        args.windows(2)
            .filter(|pair| *pair == ["--enable-mcp-server", "a"])
            .count(),
        1,
        "{record}"
    );
    assert!(
        args.windows(2)
            .any(|pair| pair == ["--disable-mcp-server", "b"]),
        "{record}"
    );
    assert!(!args.contains(&"github-mcp-server"), "{record}");
    assert_eq!(fs::read_to_string(path).unwrap(), original);
    assert_eq!(
        fs::read_to_string(home.dir.path().join(".copilot/settings.json")).unwrap(),
        r#"{"disabledMcpServers":["a"]}"#
    );
}

#[test]
fn codex_mcp_native_selection_enables_selected_and_hides_other_user_servers() {
    let home = TestHome::new();
    home.config("[mcp.selected]\ncodex = 'a'\n[mcp.same]\ncodex = 'a'\n");
    let original =
        "[mcp_servers.a]\ncommand = 'a'\nenabled = false\n[mcp_servers.b]\ncommand = 'b'\n";
    home.codex_config(original);
    let output = home.run(&["--codex", "--mcp", "selected,same"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let record = home.record();
    assert_eq!(
        record
            .lines()
            .filter(|arg| *arg == "mcp_servers.a.enabled=true")
            .count(),
        1,
        "{record}"
    );
    assert!(
        record
            .lines()
            .any(|arg| arg == "mcp_servers.b.enabled=false"),
        "{record}"
    );
    assert_eq!(
        fs::read_to_string(home.dir.path().join(".codex/config.toml")).unwrap(),
        original
    );
}

#[test]
fn codex_mcp_project_servers_are_valid_without_trust_and_shared_names_leak() {
    let home = TestHome::new();
    home.codex_config(
        "[mcp_servers.shared]\ncommand = 'user'\n[mcp_servers.other]\ncommand = 'other'\n",
    );
    home.config("[mcp.project]\ncodex = 'project'\n");
    let repo = home.dir.path().join("repo");
    let nested = repo.join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir(repo.join(".git")).unwrap();
    fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main").unwrap();
    fs::create_dir(repo.join(".codex")).unwrap();
    let path = repo.join(".codex/config.toml");
    let original =
        "[mcp_servers.project]\ncommand = 'project'\n[mcp_servers.shared]\ncommand = 'project'\n";
    fs::write(&path, original).unwrap();
    let output = home
        .command()
        .current_dir(&nested)
        .args(["--codex", "--mcp", "project"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let record = home.record();
    assert!(
        record.contains("mcp_servers.project.enabled=true"),
        "{record}"
    );
    assert!(
        record.contains("mcp_servers.other.enabled=false"),
        "{record}"
    );
    assert!(
        !record.contains("mcp_servers.shared.enabled=false"),
        "{record}"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("mcp-project-shadow"),
        "{output:?}"
    );
    assert_eq!(fs::read_to_string(path).unwrap(), original);
}

#[test]
fn codex_mcp_definitions_cannot_merge_with_user_or_untrusted_project_servers() {
    for project in [false, true] {
        let home = TestHome::new();
        home.config("[mcp.a]\ncodex = { command = 'definition' }\n");
        let original = "[mcp_servers.a]\ncommand = 'configured'\nargs = ['private']\n";
        let cwd = if project {
            let repo = home.dir.path().join("repo");
            fs::create_dir_all(repo.join(".codex")).unwrap();
            fs::create_dir(repo.join(".git")).unwrap();
            fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main").unwrap();
            fs::write(repo.join(".codex/config.toml"), original).unwrap();
            repo
        } else {
            home.codex_config(original);
            home.dir.path().to_path_buf()
        };
        let output = home
            .command()
            .current_dir(cwd)
            .args(["--codex", "--mcp", "a", "--dry-run"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("mcp-definition-collision") && stderr.contains("codex = \"a\""),
            "{stderr}"
        );
        assert!(home.no_record());
        assert!(!home.dir.path().join("cache/ayran/mcp").exists());
    }
}

#[test]
fn codex_mcp_errors_fail_before_launch_and_custom_home_is_respected() {
    for contents in [
        "not valid TOML",
        "mcp_servers = []",
        "[mcp_servers]\na = false",
        "[mcp_servers.a]\nenabled = 'false'",
    ] {
        let home = TestHome::new();
        home.codex_config(contents);
        let output = home.run(&["--codex"]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("enumeration-failed"),
            "{output:?}"
        );
        assert!(home.no_record());
        assert_eq!(
            fs::read_to_string(home.dir.path().join(".codex/config.toml")).unwrap(),
            contents
        );
    }
    let home = TestHome::new();
    home.config("[mcp.x]\ncodex = 'missing'\n");
    let output = home.run(&["--codex", "--mcp", "x"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("native-not-found"));
    assert!(home.no_record());

    home.config("[mcp.x]\ncodex = 'custom'\n");
    home.codex_config("[mcp_servers.ignored]\ncommand = 'ignored'\n");
    let custom = home.dir.path().join("custom");
    fs::create_dir(&custom).unwrap();
    fs::write(
        custom.join("config.toml"),
        "[mcp_servers.custom]\ncommand = 'custom'\nenabled = false\n",
    )
    .unwrap();
    for path in [PathBuf::from("custom"), custom] {
        let output = home
            .command()
            .env("CODEX_HOME", path)
            .args(["--codex", "--mcp", "x"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let record = home.record();
        assert!(
            record.contains("mcp_servers.custom.enabled=true"),
            "{record}"
        );
        assert!(!record.contains("ignored"), "{record}");
    }
}

#[test]
fn codex_mcp_selected_plugin_servers_protect_user_names_and_definitions_warn() {
    let home = TestHome::new();
    home.codex_config("[plugins.'p@market']\nenabled = false\n[mcp_servers.b]\ncommand = 'user'\n[mcp_servers.other]\ncommand = 'other'\n");
    home.config(
        "[plugins.p]\ncodex = 'p@market'\n[mcp.definition]\ncodex = { command = 'definition' }\n",
    );
    let plugin = home.dir.path().join(".codex/plugins/cache/market/p/local");
    fs::create_dir_all(plugin.join(".codex-plugin")).unwrap();
    fs::write(plugin.join(".codex-plugin/plugin.json"), r#"{"name":"p"}"#).unwrap();
    fs::write(
        plugin.join(".mcp.json"),
        r#"{"mcpServers":{"b":{"command":"bundled"},"definition":{"command":"bundled"}}}"#,
    )
    .unwrap();
    let output = home.run(&["--codex", "--plugin", "p", "--mcp", "definition"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let record = home.record();
    assert!(!record.contains("mcp_servers.b.enabled=false"), "{record}");
    assert!(
        record.contains("mcp_servers.other.enabled=false"),
        "{record}"
    );
    assert!(record.contains("mcp_servers.definition={"), "{record}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("plugin-server-shadow")
            && stderr.contains("warning[plugin-server-shadowed]"),
        "{stderr}"
    );
    let output = home.run(&["--codex", "--mcp", "definition"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(home.record().contains("mcp_servers.b.enabled=false"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("plugin-server-shadow"));
}

#[test]
fn codex_mcp_native_names_with_dots_are_overridden_as_literal_keys() {
    let home = TestHome::new();
    home.config("[mcp.x]\ncodex = 'a.b'\n");
    home.codex_config(
        "[mcp_servers.'a.b']\ncommand = 'selected'\n[mcp_servers.'c.d']\ncommand = 'hidden'\n",
    );
    let output = home.run(&[
        "--codex",
        "--mcp",
        "x",
        "--",
        "-c",
        "mcp_servers.a.b.enabled=false",
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let record = home.record();
    let tables: Vec<toml::Table> = record
        .lines()
        .filter(|arg| arg.starts_with("mcp_servers={"))
        .map(|arg| arg.parse().unwrap())
        .collect();
    assert!(
        tables.iter().any(|table| table["mcp_servers"]
            .get("a.b")
            .is_some_and(|server| server["enabled"].as_bool() == Some(true))),
        "{record}"
    );
    assert!(
        tables.iter().any(|table| table["mcp_servers"]
            .get("c.d")
            .is_some_and(|server| server["enabled"].as_bool() == Some(false))),
        "{record}"
    );
    assert!(
        record.ends_with("-c\nmcp_servers.a.b.enabled=false\n"),
        "{record}"
    );
}

#[test]
fn codex_mcp_agent_plugins_use_mcp_json_and_reject_symlinked_payloads() {
    let home = TestHome::new();
    home.codex_config("[plugins.'p@market']\nenabled = true\n[mcp_servers.b]\ncommand = 'user'\n");
    home.config("[plugins.p]\ncodex = 'p@market'\ndefault = true\n");
    let root = home.dir.path().join(".codex/plugins/cache/market/p/local");
    fs::write(root.join("plugin.json"), r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"p","version":"1.0.0"}"#).unwrap();
    fs::write(
        root.join("mcp.json"),
        r#"{"mcpServers":{"b":{"command":"fixture"}}}"#,
    )
    .unwrap();
    let output = home.run(&["--codex"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(!home.record().contains("mcp_servers.b.enabled=false"));
    fs::remove_file(root.join("mcp.json")).unwrap();
    fs::write(
        home.dir.path().join("outside.json"),
        r#"{"mcpServers":{"b":{"command":"fixture"}}}"#,
    )
    .unwrap();
    std::os::unix::fs::symlink(home.dir.path().join("outside.json"), root.join("mcp.json"))
        .unwrap();
    let output = home.run(&["--codex"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        home.record().contains("mcp_servers.b.enabled=false"),
        "{}",
        home.record()
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("plugin-server-shadow"));
}

#[test]
fn codex_mcp_plugin_manifest_sources_and_active_versions_match_the_harness() {
    for manifest in [
        r#"{"name":"p","mcpServers":{"b":{"command":"fixture"}}}"#,
        r#"{"name":"p","mcpServers":"./servers.json"}"#,
    ] {
        let home = TestHome::new();
        home.codex_config("[plugins.'p@market']\nenabled = true\n[mcp_servers.b]\ncommand = 'user'\n[mcp_servers.old]\ncommand = 'user'\n");
        home.config("[plugins.p]\ncodex = 'p@market'\n[profiles.work]\nplugins = ['p']\n[aliases.work]\nharness = 'codex'\nprofiles = ['work']\n");
        let cache = home.dir.path().join(".codex/plugins/cache/market/p");
        fs::remove_dir(cache.join("local")).unwrap();
        for version in ["1.9.0", "1.10.0"] {
            let root = cache.join(version);
            fs::create_dir_all(root.join(".claude-plugin")).unwrap();
            fs::write(
                root.join(".claude-plugin/plugin.json"),
                if version == "1.10.0" {
                    manifest
                } else {
                    r#"{"name":"p"}"#
                },
            )
            .unwrap();
            fs::write(root.join("servers.json"), r#"{"b":{"command":"fixture"}}"#).unwrap();
            fs::write(
                root.join(".mcp.json"),
                r#"{"mcpServers":{"old":{"command":"fixture"}}}"#,
            )
            .unwrap();
        }
        let output = home.run(&["--alias", "work"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(!home.record().contains("mcp_servers.b.enabled=false"));
        assert!(home.record().contains("mcp_servers.old.enabled=false"));
        let local = cache.join("local");
        fs::create_dir_all(local.join(".cursor-plugin")).unwrap();
        fs::write(local.join(".cursor-plugin/plugin.json"), r#"{"name":"p"}"#).unwrap();
        fs::write(
            local.join(".mcp.json"),
            r#"{"mcpServers":{"old":{"command":"fixture"}}}"#,
        )
        .unwrap();
        let output = home.run(&["--alias", "work"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(home.record().contains("mcp_servers.b.enabled=false"));
        assert!(!home.record().contains("mcp_servers.old.enabled=false"));
        fs::write(local.join(".mcp.json"), "broken").unwrap();
        let output = home.run(&["--alias", "work", "--no-plugin", "p"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(home.record().contains("mcp_servers.old.enabled=false"));
        let output = home.run(&["--alias", "work", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("enumeration-failed"));
    }
}

#[test]
fn claude_mcp_account_connectors_are_hidden_in_the_shared_settings() {
    let home = TestHome::new();
    let output = home.run(&["--claude"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(home.claude_settings()["disableClaudeAiConnectors"], true);
    assert_eq!(
        home.record()
            .lines()
            .filter(|arg| *arg == "--settings")
            .count(),
        1
    );
}

#[test]
fn claude_mcp_native_selection_hides_only_unselected_user_servers() {
    let home = TestHome::new();
    home.config("[mcp.selected]\nclaude = 'a'\n");
    let path = home.dir.path().join(".claude.json");
    let original = r#"{"mcpServers":{"a":{"command":"a"},"b":{"command":"b"},"plugin:p:s":{"command":"plugin"}},"unrelated":"preserved"}"#;
    fs::write(&path, original).unwrap();
    let output = home.run(&["--claude", "--mcp", "selected"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.claude_settings()["deniedMcpServers"],
        serde_json::json!([{"serverName":"b"}])
    );
    assert_eq!(fs::read_to_string(path).unwrap(), original);
}

#[test]
fn claude_mcp_local_scope_uses_git_root_and_respects_the_config_home() {
    let home = TestHome::new();
    let repo = home.dir.path().join("repo");
    let child = repo.join("child");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir(repo.join(".git")).unwrap();
    fs::write(repo.join(".git").join("HEAD"), "ref: refs/heads/main").unwrap();
    let custom = home.dir.path().join("custom");
    fs::create_dir(&custom).unwrap();
    let state = serde_json::json!({"mcpServers":{"a":{"command":"a"},"b":{"command":"b"}},"projects": {
        repo.to_string_lossy(): {"mcpServers":{"c":{"command":"c"}}},
        child.to_string_lossy(): {"mcpServers":{"wrong":{"command":"wrong"}}},
        home.dir.path().to_string_lossy(): {"mcpServers":{"ancestor":{"command":"ancestor"}}}
    }});
    fs::write(custom.join(".claude.json"), state.to_string()).unwrap();
    fs::write(
        home.dir.path().join(".claude.json"),
        "malformed ignored home",
    )
    .unwrap();
    home.config("[mcp.a]\nclaude = 'a'\n[mcp.c]\nclaude = 'c'\n");
    for config_home in [PathBuf::from("../../custom"), custom] {
        let output = home
            .command()
            .current_dir(&child)
            .env("CLAUDE_CONFIG_DIR", config_home)
            .args(["--claude", "--mcp", "a"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            home.claude_settings()["deniedMcpServers"],
            serde_json::json!([{"serverName":"b"},{"serverName":"c"}])
        );
    }
    let output = home
        .command()
        .current_dir(&child)
        .env("CLAUDE_CONFIG_DIR", home.dir.path().join("custom"))
        .args(["--claude", "--mcp", "c"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.claude_settings()["deniedMcpServers"],
        serde_json::json!([{"serverName":"a"},{"serverName":"b"}])
    );
}

#[test]
fn claude_mcp_project_servers_across_ancestors_are_valid_and_never_denied() {
    let home = TestHome::new();
    let repo = home.dir.path().join("repo");
    let child = repo.join("child");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir(repo.join(".git")).unwrap();
    fs::write(repo.join(".git").join("HEAD"), "ref: refs/heads/main").unwrap();
    for (directory, name) in [
        (home.dir.path(), "outside"),
        (repo.as_path(), "root"),
        (child.as_path(), "nested"),
    ] {
        fs::write(
            directory.join(".mcp.json"),
            serde_json::json!({"mcpServers": { name: {"command":"fixture"} }}).to_string(),
        )
        .unwrap();
    }
    fs::write(home.dir.path().join(".claude.json"), r#"{"mcpServers":{"root":{"command":"personal"},"outside":{"command":"personal"},"unselected":{"command":"personal"}}}"#).unwrap();
    home.config("[mcp.nested]\nclaude = 'nested'\n[mcp.root]\nclaude = 'root'\n[mcp.outside]\nclaude = 'outside'\n");
    let output = home
        .command()
        .current_dir(&child)
        .args(["--claude", "--mcp", "nested"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.claude_settings()["deniedMcpServers"],
        serde_json::json!([{"serverName":"unselected"}])
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("warning[leak]: mcp-project-shadow")
            && stderr.contains("root")
            && stderr.contains("outside"),
        "{stderr}"
    );
    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.contains("warning[leak]: mcp-project-shadow"))
            .count(),
        1,
        "{stderr}"
    );
    for selected in ["root", "outside"] {
        let output = home
            .command()
            .current_dir(&child)
            .args(["--claude", "--mcp", selected, "--dry-run"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }
}

#[test]
fn claude_mcp_definitions_shadow_user_servers_without_denying_the_copy() {
    let home = TestHome::new();
    fs::write(
        home.dir.path().join(".claude.json"),
        r#"{"mcpServers":{"x":{"command":"native"},"y":{"command":"native"}}}"#,
    )
    .unwrap();
    home.config("[mcp.x]\nclaude = {command = 'definition'}\n");
    let output = home.run(&["--claude", "--mcp", "x"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.claude_settings()["deniedMcpServers"],
        serde_json::json!([{"serverName":"y"}])
    );
    let record = home.record();
    let args: Vec<_> = record.lines().collect();
    let index = args.iter().position(|arg| *arg == "--mcp-config").unwrap();
    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(args[index + 1]).unwrap()).unwrap();
    assert_eq!(config["mcpServers"]["x"]["command"], "definition");
}

#[test]
fn claude_mcp_native_validation_and_disabled_notes_apply_to_dry_run_and_quiet() {
    let home = TestHome::new();
    fs::create_dir(home.dir.path().join(".git")).unwrap();
    fs::write(
        home.dir.path().join(".git").join("HEAD"),
        "ref: refs/heads/main",
    )
    .unwrap();
    let state = serde_json::json!({"mcpServers":{"off":{"command":"fixture"}},"projects":{
        home.dir.path().to_string_lossy(): {"disabledMcpServers":["off"]}
    }})
    .to_string();
    fs::write(home.dir.path().join(".claude.json"), &state).unwrap();
    home.config("[mcp.off]\nclaude = 'off'\n[mcp.unknown]\nclaude = 'absent'\n[mcp.account]\nclaude = 'claude.ai Linear'\n");
    for args in [
        vec!["--claude", "--mcp", "off"],
        vec!["--claude", "--mcp", "off", "--dry-run"],
    ] {
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("note[mcp-disabled]") && stderr.contains("/mcp"),
            "{stderr}"
        );
    }
    assert!(home.claude_settings().get("deniedMcpServers").is_none());
    let output = home.run(&["--claude", "--mcp", "off", "--quiet"]);
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{output:?}"
    );
    for name in ["unknown", "account"] {
        let output = home.run(&["--claude", "--mcp", name, "--dry-run", "--quiet"]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("error[native-not-found]"), "{stderr}");
        if name == "account" {
            assert!(
                stderr.contains("Account connectors require a connector Binding"),
                "{stderr}"
            );
        }
    }
    assert_eq!(
        fs::read_to_string(home.dir.path().join(".claude.json")).unwrap(),
        state
    );
}

#[test]
fn claude_mcp_malformed_or_unreadable_state_refuses_launch() {
    for (file, contents) in [
        (".claude.json", "{broken"),
        (".claude.json", "[]"),
        (".claude.json", r#"{"mcpServers":[]}"#),
        (".claude.json", r#"{"mcpServers":{"a":false}}"#),
        (".claude.json", r#"{"projects":[]}"#),
        (".mcp.json", "{broken"),
        (".mcp.json", r#"{"mcpServers":false}"#),
    ] {
        let home = TestHome::new();
        let path = home.dir.path().join(file);
        fs::write(&path, contents).unwrap();
        let output = home.run(&["--claude"]);
        assert_eq!(output.status.code(), Some(3), "{file}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("error[enumeration-failed]"),
            "{output:?}"
        );
        assert!(home.no_record());
        assert_eq!(fs::read_to_string(path).unwrap(), contents);
    }
    let home = TestHome::new();
    fs::create_dir(home.dir.path().join(".claude.json")).unwrap();
    let output = home.run(&["--claude"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("error[enumeration-failed]"));
    assert!(home.no_record());
}

#[test]
fn codex_repaired_yaml_name_excludes_the_comment() {
    let home = TestHome::new();
    home.skill(".agents/skills/comment", "placeholder");
    fs::write(
        home.dir.path().join(".agents/skills/comment/SKILL.md"),
        "---\nname: [foo # note\ndescription: Example\n---\nInstructions",
    )
    .unwrap();
    home.codex_config("[[skills.config]]\nname = '[foo'\nenabled = false\n");
    home.config("[skills.x]\ncodex = '[foo'\n");
    let output = home.run(&["--codex", "--skill", "x"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.codex_skill_rules(),
        BTreeMap::from([(
            home.dir.path().join(".agents/skills/comment/SKILL.md"),
            true
        )])
    );
}

#[test]
fn codex_native_skills_accept_repairable_yaml_descriptions() {
    let home = TestHome::new();
    home.skill(".agents/skills/repaired", "repaired");
    fs::write(
        home.dir.path().join(".agents/skills/repaired/SKILL.md"),
        "---\nname: repaired\ndescription: Build for AWS: ECS\n---\nInstructions",
    )
    .unwrap();
    home.codex_config("[[skills.config]]\nname = 'repaired'\nenabled = false\n");
    home.config("[skills.x]\ncodex = 'repaired'\n");
    let output = home.run(&["--codex", "--skill", "x"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.codex_skill_rules(),
        BTreeMap::from([(
            home.dir.path().join(".agents/skills/repaired/SKILL.md"),
            true
        )])
    );
}

#[test]
fn codex_personal_symlink_stays_personal_when_its_target_is_also_a_project_skill() {
    let home = TestHome::new();
    fs::create_dir_all(home.dir.path().join("project/.git")).unwrap();
    fs::write(
        home.dir.path().join("project/.git").join("HEAD"),
        "ref: refs/heads/main",
    )
    .unwrap();
    home.skill("project/.agents/skills/shared", "shared");
    let personal = home.dir.path().join(".agents/skills");
    fs::create_dir_all(&personal).unwrap();
    std::os::unix::fs::symlink(
        home.dir.path().join("project/.agents/skills/shared"),
        personal.join("shared"),
    )
    .unwrap();
    let output = home
        .command()
        .current_dir(home.dir.path().join("project"))
        .args(["--codex"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.codex_skill_rules(),
        BTreeMap::from([(
            home.dir
                .path()
                .join("project/.agents/skills/shared/SKILL.md"),
            false
        )])
    );
}

#[test]
fn codex_native_lookup_rejects_missing_frontmatter_description_and_plugin_skills() {
    for contents in ["Instructions only", "---\nname: invalid\n---\nInstructions"] {
        let home = TestHome::new();
        home.skill(".agents/skills/invalid", "invalid");
        fs::write(
            home.dir.path().join(".agents/skills/invalid/SKILL.md"),
            contents,
        )
        .unwrap();
        home.config("[skills.x]\ncodex = 'invalid'\n");
        let output = home.run(&["--codex", "--skill", "x"]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("error[native-not-found]")
        );
    }
    let home = TestHome::new();
    home.skill("source/plugin/skills/nested", "nested");
    fs::create_dir_all(home.dir.path().join("source/plugin/.codex-plugin")).unwrap();
    fs::write(
        home.dir
            .path()
            .join("source/plugin/.codex-plugin/plugin.json"),
        r#"{"name":"plugin"}"#,
    )
    .unwrap();
    let personal = home.dir.path().join(".agents/skills");
    fs::create_dir_all(&personal).unwrap();
    std::os::unix::fs::symlink(
        home.dir.path().join("source/plugin/skills/nested"),
        personal.join("linked"),
    )
    .unwrap();
    home.config("[skills.x]\ncodex = 'nested'\n");
    let output = home.run(&["--codex", "--skill", "x"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[native-not-found]")
    );
}

#[test]
fn codex_custom_project_root_markers_include_ancestor_skills() {
    let home = TestHome::new();
    fs::create_dir_all(home.dir.path().join("project/subdir")).unwrap();
    fs::write(home.dir.path().join("project/custom-root"), "").unwrap();
    home.skill("project/.agents/skills/team", "team");
    home.skill(".codex/skills/.system/ignored", "ignored");
    home.config("[skills.x]\ncodex = 'team'\n");
    home.codex_config("project_root_markers = ['custom-root']\n[[skills.config]]\nname = 'team'\nenabled = false\n");
    let output = home
        .command()
        .current_dir(home.dir.path().join("project/subdir"))
        .args(["--codex", "--skill", "x"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.codex_skill_rules(),
        BTreeMap::from([(
            home.dir.path().join("project/.agents/skills/team/SKILL.md"),
            true
        )])
    );
    home.codex_config("project_root_markers = []\n");
    let output = home
        .command()
        .current_dir(home.dir.path().join("project/subdir"))
        .args(["--codex", "--skill", "x"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[native-not-found]")
    );
}

#[test]
fn codex_discovery_ignores_symlinked_files_and_bundled_directory_symlinks() {
    let home = TestHome::new();
    home.skill("source/real", "real");
    let linked_file = home.dir.path().join(".agents/skills/file-link");
    fs::create_dir_all(&linked_file).unwrap();
    std::os::unix::fs::symlink(
        home.dir.path().join("source/real/SKILL.md"),
        linked_file.join("SKILL.md"),
    )
    .unwrap();
    let bundled = home.dir.path().join(".codex/skills/.system");
    fs::create_dir_all(&bundled).unwrap();
    std::os::unix::fs::symlink(
        home.dir.path().join("source/real"),
        bundled.join("dir-link"),
    )
    .unwrap();
    home.config("[skills.x]\ncodex = 'real'\n");
    let output = home.run(&["--codex", "--skill", "x"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[native-not-found]")
    );
    assert!(home.no_record());
}

#[test]
fn codex_skill_errors_fail_before_launch_including_all_path_bindings() {
    for (config, name, code) in [
        (
            "[skills.x]\ncodex = {path = 'missing'}",
            "x",
            "unsupported-binding",
        ),
        (
            "[skills.x]\nall = {path = 'missing'}",
            "x",
            "unsupported-binding",
        ),
        ("[skills.x]\ncodex = 'missing'", "x", "native-not-found"),
        ("[skills.x]\ncodex = 'directory'", "x", "native-not-found"),
        ("[skills.x]\nclaude = 'actual'", "x", "missing-binding"),
        ("[skills.x]\ncodex = false", "x", "binding-absent"),
        ("", "x", "unknown-skill"),
    ] {
        let home = TestHome::new();
        home.skill(".agents/skills/directory", "actual");
        home.config(config);
        for dry in [false, true] {
            let mut args = vec!["--codex", "--skill", name];
            if dry {
                args.push("--dry-run");
            }
            let output = home.run(&args);
            assert_eq!(output.status.code(), Some(3), "{output:?}");
            assert!(
                String::from_utf8(output.stderr)
                    .unwrap()
                    .contains(&format!("error[{code}]"))
            );
            assert!(home.no_record());
        }
    }
}

#[test]
fn codex_unreadable_skill_state_fails_even_without_a_selection() {
    for root in [".agents/skills", ".codex/skills", ".codex/skills/.system"] {
        let home = TestHome::new();
        let path = home.dir.path().join(root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "not a directory").unwrap();
        let output = home.run(&["--codex", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("error[enumeration-failed]")
        );
        assert!(home.no_record());
    }
    let home = TestHome::new();
    home.skill(".agents/skills/malformed", "[invalid");
    home.config("[skills.x]\ncodex = 'malformed'\n");
    let output = home.run(&["--codex", "--skill", "x"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[native-not-found]")
    );
    assert!(home.no_record());
}

#[test]
fn codex_skill_dry_run_shows_one_array_and_passthrough_stays_last() {
    let home = TestHome::new();
    home.skill(".agents/skills/quote's \"directory", "tdd");
    home.codex_config("[[skills.config]]\nname = 'tdd'\nenabled = false\n");
    home.config("[skills.x]\nall = 'tdd'\n[skills.y]\ncodex = 'tdd'\n");
    let args = [
        "--codex",
        "--skill",
        "x,y,x",
        "--dry-run",
        "--",
        "-c",
        "skills.config=[]",
    ];
    let dry = home.run(&args);
    assert_eq!(dry.status.code(), Some(0), "{dry:?}");
    let stdout = String::from_utf8(dry.stdout).unwrap();
    assert!(stdout.contains("skills.config=[{path="), "{stdout}");
    assert!(stdout.contains("enabled=true"), "{stdout}");
    assert!(stdout.ends_with(" -c 'skills.config=[]'\n"), "{stdout}");
    assert!(home.no_record());
    assert!(!home.dir.path().join("cache").exists());
    let trace = String::from_utf8(dry.stderr).unwrap();
    assert_eq!(trace.matches("Skill x:").count(), 1);
    assert_eq!(trace.matches("Skill y:").count(), 1);
    let real = home.run(&["--codex", "--skill", "x,y", "--", "-c", "skills.config=[]"]);
    assert_eq!(real.status.code(), Some(0), "{real:?}");
    let record = home.record();
    let args: Vec<_> = record.lines().collect();
    assert_eq!(args[args.len() - 2..], ["-c", "skills.config=[]"]);
    let rules: toml::Table = args[1].parse().unwrap();
    assert_eq!(rules["skills"]["config"].as_array().unwrap().len(), 1);
    assert_eq!(
        rules["skills"]["config"][0]["path"].as_str().unwrap(),
        home.dir
            .path()
            .join(".agents/skills/quote's \"directory/SKILL.md")
            .to_str()
            .unwrap()
    );
}

#[test]
fn codex_empty_frontmatter_name_falls_back_to_directory_name() {
    let home = TestHome::new();
    home.skill(".agents/skills/fallback", "''");
    home.codex_config("[[skills.config]]\nname = 'fallback'\nenabled = false\n");
    home.config("[skills.x]\ncodex = 'fallback'\n");
    let output = home.run(&["--codex", "--skill", "x"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.codex_skill_rules(),
        BTreeMap::from([(
            home.dir.path().join(".agents/skills/fallback/SKILL.md"),
            true
        )])
    );
}

#[test]
fn codex_leaves_project_and_bundled_skills_alone_even_in_a_home_dotfiles_repo() {
    let home = TestHome::new();
    fs::create_dir(home.dir.path().join(".git")).unwrap();
    fs::write(
        home.dir.path().join(".git").join("HEAD"),
        "ref: refs/heads/main",
    )
    .unwrap();
    home.skill(".agents/skills/personal", "shared");
    home.skill(".codex/skills/personal", "codex-personal");
    home.skill(".codex/skills/.system/bundled", "bundled");
    home.skill("project/.agents/skills/repo", "shared");
    home.skill("project/nested/.agents/skills/local", "local");
    home.config("[skills.repo]\ncodex = 'local'\n[skills.bundled]\ncodex = 'bundled'\n");
    let cwd = home.dir.path().join("project/nested");
    let output = home
        .command()
        .current_dir(&cwd)
        .args(["--codex"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.codex_skill_rules(),
        BTreeMap::from([
            (
                home.dir.path().join(".agents/skills/personal/SKILL.md"),
                false
            ),
            (
                home.dir.path().join(".codex/skills/personal/SKILL.md"),
                false
            ),
        ])
    );
    let output = home
        .command()
        .current_dir(cwd)
        .args(["--codex", "--skill", "repo,bundled"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let rules = home.codex_skill_rules();
    assert!(
        !rules.contains_key(
            &home
                .dir
                .path()
                .join("project/nested/.agents/skills/local/SKILL.md")
        )
    );
    assert!(
        !rules.contains_key(
            &home
                .dir
                .path()
                .join(".codex/skills/.system/bundled/SKILL.md")
        )
    );
    assert!(!rules.contains_key(&home.dir.path().join("project/.agents/skills/repo/SKILL.md")));
}

#[test]
fn codex_discovers_project_codex_skills_and_uses_custom_home() {
    let home = TestHome::new();
    fs::create_dir_all(home.dir.path().join("project/.git")).unwrap();
    fs::write(
        home.dir.path().join("project/.git").join("HEAD"),
        "ref: refs/heads/main",
    )
    .unwrap();
    fs::create_dir_all(home.dir.path().join("project/subdir")).unwrap();
    home.skill("project/.codex/skills/team", "team");
    home.skill("custom/skills/selected", "chosen");
    home.skill("custom/skills/.system/bundled", "bundled");
    home.skill(".codex/skills/ignored", "ignored");
    fs::write(home.dir.path().join("custom/config.toml"), "[[skills.config]]\nname = 'team'\nenabled = false\n[[skills.config]]\nname = 'chosen'\nenabled = false\n").unwrap();
    home.config("[skills.team]\ncodex = 'team'\n[skills.personal]\ncodex = 'chosen'\n");
    let output = home
        .command()
        .current_dir(home.dir.path().join("project/subdir"))
        .env("CODEX_HOME", home.dir.path().join("custom"))
        .args(["--codex", "--skill", "team,personal"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.codex_skill_rules(),
        BTreeMap::from([
            (
                home.dir.path().join("custom/skills/selected/SKILL.md"),
                true
            ),
            (
                home.dir.path().join("project/.codex/skills/team/SKILL.md"),
                true
            ),
        ])
    );
}

#[test]
fn codex_symlink_rules_use_the_target_and_directory_fallback_uses_target_name() {
    let home = TestHome::new();
    let target = home.dir.path().join("source/target-name");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("SKILL.md"),
        "---\ndescription: Example\n---\nInstructions",
    )
    .unwrap();
    let root = home.dir.path().join(".agents/skills");
    fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(&target, root.join("alias")).unwrap();
    home.config("[skills.selected]\ncodex = 'target-name'\n");
    let hidden = home.run(&["--codex"]);
    assert_eq!(hidden.status.code(), Some(0), "{hidden:?}");
    assert_eq!(
        home.codex_skill_rules(),
        BTreeMap::from([(target.join("SKILL.md"), false)])
    );
    home.codex_config("[[skills.config]]\nname = 'target-name'\nenabled = false\n");
    let selected = home.run(&["--codex", "--skill", "selected"]);
    assert_eq!(selected.status.code(), Some(0), "{selected:?}");
    assert_eq!(
        home.codex_skill_rules(),
        BTreeMap::from([(target.join("SKILL.md"), true)])
    );
}

#[test]
fn codex_hides_personal_skills_and_force_enables_selected_names_in_one_array() {
    let home = TestHome::new();
    home.skill(".agents/skills/directory", "selected-name");
    home.skill(".codex/skills/other", "other");
    home.config("[skills.team]\nall = 'selected-name'\n");
    let original = "[[skills.config]]\npath = 'ignored'\nenabled = false\n[[skills.config]]\nname = 'selected-name'\nenabled = false\n";
    home.codex_config(original);
    let output = home.run(&["--codex", "--skill", "team"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.codex_skill_rules(),
        BTreeMap::from([
            (
                home.dir.path().join(".agents/skills/directory/SKILL.md"),
                true
            ),
            (home.dir.path().join(".codex/skills/other/SKILL.md"), false),
        ])
    );
    assert_eq!(
        fs::read_to_string(home.dir.path().join(".codex/config.toml")).unwrap(),
        original
    );
    assert!(!home.dir.path().join("cache/ayran/codex").exists());
}

#[test]
fn claude_native_directory_alias_enables_the_effective_skill_name_once() {
    let home = TestHome::new();
    home.skill(".claude/skills/directory-alias", "effective-name");
    fs::write(
        home.dir.path().join(".claude/settings.json"),
        r#"{"skillOverrides":{"effective-name":"off"}}"#,
    )
    .unwrap();
    home.config(
        "[skills.alias]\nclaude = 'directory-alias'\n[skills.name]\nclaude = 'effective-name'\n",
    );
    let output = home.run(&["--claude", "--skill", "alias,name"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings()["skillOverrides"],
        serde_json::json!({"effective-name":"on"})
    );
}

#[test]
fn claude_never_hides_a_bundled_skill_when_a_personal_skill_shares_its_name() {
    let home = TestHome::new();
    home.skill(".claude/skills/doctor", "doctor");
    let output = home.run(&["--claude"]);
    assert!(output.status.success(), "{output:?}");
    assert!(home.claude_settings().get("skillOverrides").is_none());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("warning[leak]: claude-bundled-shadow"),
        "{stderr}"
    );
}

#[test]
fn claude_leaves_project_collisions_visible_and_reports_path_shadowing() {
    let home = TestHome::new();
    home.skill(".claude/skills/personal-directory", "shared");
    home.skill("project/.claude/skills/project-directory", "shared");
    home.skill("project/path-skill", "shared");
    fs::create_dir(home.dir.path().join("project/.git")).unwrap();
    fs::write(
        home.dir.path().join("project/.git").join("HEAD"),
        "ref: refs/heads/main",
    )
    .unwrap();
    let cwd = home.dir.path().join("project/src/nested");
    fs::create_dir_all(&cwd).unwrap();
    home.config("[skills.path]\nclaude = {path = '~/project/path-skill'}\n");
    let output = home
        .command()
        .current_dir(&cwd)
        .args(["--claude", "--skill", "path"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(home.claude_settings().get("skillOverrides").is_none());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("warning[leak]: claude-project-shadow: personal Skill shared"),
        "{stderr}"
    );
    assert!(stderr.contains("note[skill-shadowed]"), "{stderr}");
    let quiet = home
        .command()
        .current_dir(cwd)
        .args(["--claude", "--skill", "path", "--quiet"])
        .output()
        .unwrap();
    assert!(quiet.status.success(), "{quiet:?}");
    assert!(quiet.stderr.is_empty(), "{quiet:?}");
}

#[test]
fn claude_dotfiles_repo_keeps_personal_skills_hidden_and_accepts_project_native_skills() {
    let home = TestHome::new();
    home.skill(".claude/skills/renamed", "personal-name");
    home.skill("project/.claude/skills/renamed", "project-name");
    fs::create_dir(home.dir.path().join(".git")).unwrap();
    fs::write(
        home.dir.path().join(".git").join("HEAD"),
        "ref: refs/heads/main",
    )
    .unwrap();
    home.config("[skills.team]\nclaude = 'project-name'\n");
    let output = home
        .command()
        .current_dir(home.dir.path().join("project"))
        .args(["--claude", "--skill", "team"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings()["skillOverrides"],
        serde_json::json!({
            "personal-name": "off"
        })
    );
    assert!(
        !String::from_utf8(output.stderr)
            .unwrap()
            .contains("claude-project-shadow")
    );
}

#[test]
fn claude_path_skill_collision_with_personal_skill_is_not_hidden() {
    let home = TestHome::new();
    home.skill(".claude/skills/shared", "shared");
    home.skill("path-skill", "shared");
    home.config("[skills.path]\nclaude = {path = '~/path-skill'}\n");
    let output = home.run(&["--claude", "--skill", "path"]);
    assert!(output.status.success(), "{output:?}");
    assert!(home.claude_settings().get("skillOverrides").is_none());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("note[skill-shadowed]"), "{stderr}");
    assert!(!stderr.contains("claude-project-shadow"), "{stderr}");
}

#[test]
fn claude_excludes_synced_and_skills_dir_plugins_from_standalone_skills() {
    let home = TestHome::new();
    home.skill(".claude/skills/synced", "account-skill");
    home.skill(".claude/skills/SYNCED", "account-skill-uppercase");
    home.skill(".claude/skills/plugin", "plugin-skill");
    let manifest = home.dir.path().join(".claude/skills/plugin/.claude-plugin");
    fs::create_dir(&manifest).unwrap();
    fs::write(manifest.join("plugin.json"), r#"{"name":"plugin"}"#).unwrap();
    for name in [
        "account-skill",
        "account-skill-uppercase",
        "plugin-skill",
        "missing",
    ] {
        home.config(&format!("[skills.x]\nclaude = '{name}'\n"));
        let output = home.run(&["--claude", "--skill", "x", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{name}: {output:?}");
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("error[native-not-found]")
        );
    }
    let output = home.run(&["--claude"]);
    assert!(output.status.success(), "{output:?}");
    assert!(home.claude_settings().get("skillOverrides").is_none());
}

#[test]
fn claude_bundled_skills_are_valid_native_bindings_and_never_hidden() {
    let home = TestHome::new();
    home.config("[skills.api]\nclaude = 'claude-api'\n");
    let output = home.run(&["--claude", "--skill", "api"]);
    assert!(output.status.success(), "{output:?}");
    assert!(home.claude_settings().get("skillOverrides").is_none());
}

#[test]
fn claude_uses_config_dir_and_fails_on_unreadable_skill_state() {
    let home = TestHome::new();
    home.skill(".claude/skills/ignored", "ignored");
    home.skill("custom/skills/renamed", "effective");
    let custom = home.dir.path().join("custom");
    let output = home
        .command()
        .env("CLAUDE_CONFIG_DIR", &custom)
        .args(["--claude"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings()["skillOverrides"],
        serde_json::json!({"effective":"off"})
    );
    let root = custom.join("skills");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o000)).unwrap();
    let output = home
        .command()
        .env("CLAUDE_CONFIG_DIR", &custom)
        .args(["--claude", "--dry-run"])
        .output()
        .unwrap();
    fs::set_permissions(root, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[enumeration-failed]")
    );
}

#[test]
fn claude_enables_selected_native_skills_and_hides_other_personal_skills() {
    let home = TestHome::new();
    for name in ["a", "b"] {
        let path = home.dir.path().join(".claude/skills").join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Fixture skill\n---\nInstructions"),
        )
        .unwrap();
    }
    home.config("[skills.a]\nall = 'a'\n[plugins.review]\nclaude = 'review@m'\n");
    home.claude_installs(r#"{"version":2,"plugins":{"review@m":[{"scope":"user"}]}}"#);
    let settings_path = home.dir.path().join(".claude/settings.json");
    let original = r#"{"skillOverrides":{"a":"off"},"syncClaudeAiSkills":true}"#;
    fs::write(&settings_path, original).unwrap();
    let output = home.run(&["--claude", "--skill", "a", "--plugin", "review"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let record = home.record();
    let args: Vec<_> = record.lines().collect();
    assert_eq!(args.len(), 2, "{record}");
    assert_eq!(args[0], "--settings");
    let settings: serde_json::Value = serde_json::from_str(args[1]).unwrap();
    assert_eq!(
        settings,
        serde_json::json!({
            "enabledPlugins": {"review@m": true},
            "skillOverrides": {"a": "on", "b": "off"},
            "disableClaudeAiConnectors": true,
            "syncClaudeAiSkills": false
        })
    );
    assert_eq!(fs::read_to_string(settings_path).unwrap(), original);
}

#[test]
fn path_skill_loads_under_its_own_name_and_dry_run_creates_nothing() {
    let home = TestHome::new();
    let skill = home.dir.path().join("tools/lint");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: team-lint\ndescription: Lint\n---\nInstructions\n",
    )
    .unwrap();
    fs::write(
        home.dir.path().join("ayran.toml"),
        "[skills.lint]\nall = { path = 'tools/lint' }\n",
    )
    .unwrap();
    let dry = home.run(&[
        "--claude",
        "--skill",
        "lint",
        "--dry-run",
        "--",
        "--native",
        "last",
    ]);
    assert_eq!(dry.status.code(), Some(0), "{dry:?}");
    let command = String::from_utf8(dry.stdout).unwrap();
    assert!(command.contains(" --add-dir "), "{command}");
    assert!(command.ends_with(" --native last\n"), "{command}");
    let trace = String::from_utf8(dry.stderr).unwrap();
    assert!(
        trace.contains("Skill lint: explicit (--skill) → team-lint"),
        "{trace}"
    );
    assert!(!home.dir.path().join("cache").exists());
    let real = home.run(&["--claude", "--skill", "lint", "--", "--native", "last"]);
    assert_eq!(real.status.code(), Some(0), "{real:?}");
    let record = home.record();
    let args: Vec<_> = record.lines().collect();
    assert_eq!(args[2], "--add-dir");
    let cache = Path::new(args[3]);
    assert!(cache.starts_with(home.dir.path().join("cache/ayran/claude")));
    assert_eq!(
        fs::read_link(cache.join(".claude/skills/team-lint")).unwrap(),
        skill
    );
    assert_eq!(&args[4..], ["--native", "last"]);
}

impl TestHome {
    fn session_path(&self) -> PathBuf {
        fs::read_dir(self.dir.path().join("state/ayran/sessions"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
    }

    fn codex_skill_rules(&self) -> BTreeMap<PathBuf, bool> {
        let record = self.record();
        let arrays: Vec<_> = record
            .lines()
            .filter(|arg| arg.starts_with("skills.config="))
            .collect();
        assert_eq!(arrays.len(), 1, "{record}");
        let settings: toml::Table = arrays[0].parse().unwrap();
        let rules = settings["skills"]["config"].as_array().unwrap();
        let mut result = BTreeMap::new();
        for rule in rules {
            assert!(rule.get("name").is_none());
            assert!(
                result
                    .insert(
                        PathBuf::from(rule["path"].as_str().unwrap()),
                        rule["enabled"].as_bool().unwrap()
                    )
                    .is_none()
            );
        }
        result
    }

    fn skill(&self, relative: &str, name: &str) {
        let path = self.dir.path().join(relative);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Fixture skill\n---\nInstructions"),
        )
        .unwrap();
    }

    fn claude_settings(&self) -> serde_json::Value {
        let record = self.record();
        let args: Vec<_> = record.lines().collect();
        let index = args.iter().position(|arg| *arg == "--settings").unwrap();
        serde_json::from_str(args[index + 1]).unwrap()
    }

    fn new() -> Self {
        let home = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        for name in ["claude", "codex", "copilot"] {
            home.fake_harness(name, 0);
        }
        home
    }

    fn fake_harness(&self, name: &str, exit: i32) {
        let bin = self.dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let script = bin.join(name);
        fs::write(
            &script,
            format!("#!/bin/sh\nif [ \"$1\" = --version ]; then\n  printf '%s\\n' \"${{HARNESS_VERSION-9.0.0}}\"\n  exit \"${{VERSION_EXIT-0}}\"\nfi\npwd > \"$HOME/cwd_record\"\nprintf '%s\\n' \"$@\" > \"$RECORD\"\nprintf '%s\\n' \"${{CLAUDE_CODE_EFFORT_LEVEL-unset}}\" > \"$ENV_RECORD\"\nprintf '%s\\n' \"${{COPILOT_PLUGIN_DIR_ONLY-unset}}\" > \"$PLUGIN_ENV_RECORD\"\nprintf '%s\\n' \"${{{}-unset}}\" > \"$HOME/home_record\"\nexit {exit}\n", match name { "claude" => "CLAUDE_CONFIG_DIR", "codex" => "CODEX_HOME", _ => "COPILOT_HOME" }),
        )
        .unwrap();
        fs::set_permissions(script, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn bump_harness_mtime(&self, name: &str) {
        let binary = fs::File::open(self.dir.path().join("bin").join(name)).unwrap();
        let modified = binary.metadata().unwrap().modified().unwrap();
        binary
            .set_times(
                fs::FileTimes::new().set_modified(modified + std::time::Duration::from_secs(1)),
            )
            .unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with_effort_env(args, None)
    }

    fn run_with_effort_env(&self, args: &[&str], level: Option<&str>) -> Output {
        let mut command = self.command();
        command.args(args);
        if let Some(level) = level {
            command.env("CLAUDE_CODE_EFFORT_LEVEL", level);
        } else {
            command.env_remove("CLAUDE_CODE_EFFORT_LEVEL");
        }
        command.output().unwrap()
    }

    fn command(&self) -> Command {
        let path = self.dir.path().join("bin");
        let mut command = Command::new(env!("CARGO_BIN_EXE_ayran"));
        command
            .current_dir(self.dir.path())
            .env("HOME", self.dir.path())
            .env("CLAUDE_CODE_DISABLE_POLICY_SKILLS", "1")
            .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
            .env("XDG_CACHE_HOME", self.dir.path().join("cache"))
            .env("XDG_STATE_HOME", self.dir.path().join("state"))
            .env("PATH", path)
            .env("RECORD", self.dir.path().join("record"))
            .env("ENV_RECORD", self.dir.path().join("env_record"))
            .env(
                "PLUGIN_ENV_RECORD",
                self.dir.path().join("plugin_env_record"),
            )
            .env_remove("HARNESS_VERSION")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env_remove("COPILOT_HOME")
            .env_remove("COPILOT_PLUGIN_DIR_ONLY")
            .env_remove("VERSION_EXIT");
        command
    }

    fn raw_record(&self) -> String {
        fs::read_to_string(self.dir.path().join("record")).unwrap()
    }

    fn record(&self) -> String {
        without_session_id(&self.raw_record())
    }

    fn env_record(&self) -> String {
        fs::read_to_string(self.dir.path().join("env_record")).unwrap()
    }

    fn no_record(&self) -> bool {
        !Path::new(&self.dir.path().join("record")).exists()
    }

    fn config(&self, contents: &str) {
        let path = self.dir.path().join("config/ayran/ayran.toml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn codex_config(&self, contents: &str) {
        let path = self.dir.path().join(".codex/config.toml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
        if let Ok(config) = contents.parse::<toml::Table>()
            && let Some(plugins) = config.get("plugins").and_then(toml::Value::as_table)
        {
            for id in plugins.keys() {
                if let Some((name, marketplace)) = id.split_once('@') {
                    fs::create_dir_all(
                        self.dir
                            .path()
                            .join(".codex/plugins/cache")
                            .join(marketplace)
                            .join(name)
                            .join("local"),
                    )
                    .unwrap();
                }
            }
        }
    }

    fn claude_installs(&self, contents: &str) {
        let path = self
            .dir
            .path()
            .join(".claude/plugins/installed_plugins.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}

fn snapshot(directory: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut contents = BTreeMap::new();
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            contents.extend(snapshot(&path));
        } else {
            contents.insert(path.clone(), fs::read(path).unwrap());
        }
    }
    contents
}

#[test]
fn plugin_defaults_activate_and_cli_disables_hide_them() {
    let home = TestHome::new();
    home.config("[plugins.a]\nall = 'a@m'\ndefault = true\n");
    home.claude_installs(r#"{"version":2,"plugins":{"a@m":[{"scope":"user"}]}}"#);
    for (flags, enabled, reason) in [
        (vec![], true, "Default"),
        (vec!["--no-defaults"], false, "--no-defaults"),
        (vec!["--no-plugin", "a"], false, "--no-plugin"),
        (
            vec!["--plugin", "a", "--no-plugin", "a"],
            false,
            "--no-plugin",
        ),
        (vec!["--no-defaults", "--plugin", "a"], true, "explicit"),
    ] {
        let mut args = vec!["--claude", "--dry-run"];
        args.extend(flags);
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let command = String::from_utf8(output.stdout).unwrap();
        assert!(command.contains(&format!("\"a@m\":{enabled}")), "{command}");
        let trace = String::from_utf8(output.stderr).unwrap();
        assert!(
            trace.contains("Plugin a: ") && trace.contains(reason),
            "{trace}"
        );
    }
}

#[test]
fn config_disables_union_and_reset_only_farther_defaults() {
    let home = TestHome::new();
    home.config("[plugins.far]\nall = 'far@m'\ndefault = true\n[plugins.replaced]\nall = 'old@m'\ndefault = true\n[disable]\nplugins = ['same']\n");
    let project = home.dir.path().join("ayran.toml");
    fs::write(&project, "[disable]\ndefaults = true\nplugins = ['near']\n[plugins.same]\nall = 'same@m'\ndefault = true\n[plugins.replaced]\nall = 'new@m'\ndefault = true\n").unwrap();
    fs::write(home.dir.path().join("ayran.local.toml"), "[disable]\ndefaults = false\nplugins = []\n[plugins.near]\nall = 'near@m'\ndefault = true\n[plugins.local]\nall = 'local@m'\ndefault = true\n").unwrap();
    home.claude_installs(r#"{"version":2,"plugins":{"far@m":[{"scope":"user"}],"same@m":[{"scope":"user"}],"new@m":[{"scope":"user"}],"near@m":[{"scope":"user"}],"local@m":[{"scope":"user"}]}}"#);
    let output = home.run(&["--claude", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let command = String::from_utf8(output.stdout).unwrap();
    assert!(
        command.contains(
            r#"{"far@m":false,"local@m":true,"near@m":false,"new@m":true,"same@m":false}"#
        ),
        "{command}"
    );
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(
        trace.contains(&format!(
            "Plugin far: disabled by layer {}",
            project.display()
        )),
        "{trace}"
    );
    let output = home.run(&["--claude", "--plugin", "far,same,near", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains("@m\":false")
    );
}

#[test]
fn default_false_bindings_are_notes_but_explicit_and_missing_bindings_fail() {
    let home = TestHome::new();
    home.config("[plugins.review]\nall = 'review@acme'\ncodex = false\ndefault = true\n");
    let output = home.run(&["--codex", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(
        trace.contains("note[binding-skipped]") && trace.contains("Plugin review: skipped"),
        "{trace}"
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "codex --disable apps\n"
    );
    let output = home.run(&["--codex", "-q"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty());
    let output = home.run(&["--codex", "--plugin", "review", "-q"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[binding-absent]")
    );
    home.config("[plugins.review]\nclaude = 'review@acme'\ndefault = true\n");
    let output = home.run(&["--codex", "--dry-run"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[missing-binding]")
    );
}

#[test]
fn alias_plugin_lists_are_explicit_and_alias_disables_remove_only_defaults() {
    let home = TestHome::new();
    home.config("[plugins.a]\nall = 'a@m'\ndefault = true\n[plugins.b]\nall = 'b@m'\ndefault = true\n[disable]\nplugins = ['a']\n[aliases.work]\nharness = 'claude'\nplugins = ['a']\ndisable = { plugins = ['a', 'b'] }\n[aliases.clean]\nharness = 'claude'\nplugins = ['a']\ndefaults = false\n");
    home.claude_installs(
        r#"{"version":2,"plugins":{"a@m":[{"scope":"user"}],"b@m":[{"scope":"user"}]}}"#,
    );
    for alias in ["work", "clean"] {
        let output = home.run(&["--alias", alias, "--dry-run"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .contains(r#"{"a@m":true,"b@m":false}"#)
        );
        let trace = String::from_utf8(output.stderr).unwrap();
        assert!(
            trace.contains(&format!("Plugin a: explicit (Alias {alias})")),
            "{trace}"
        );
        let output = home.run(&["--alias", alias, "--no-plugin", "a", "--dry-run"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .contains(r#"{"a@m":false,"b@m":false}"#)
        );
    }
    let output = home.run(&["--alias", "work", "--plugin", "b", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains("@m\":false")
    );
}

#[test]
fn worked_example_skill_and_plugin_rows_resolve_across_user_project_and_local_layers() {
    let home = TestHome::new();
    home.config("default_harness = 'claude'\n[harnesses.claude]\nmodel = 'sonnet'\neffort = 'medium'\n[skills.tdd]\nall = 'tdd'\ndefault = true\n[plugins.review]\nall = 'review@acme'\ncodex = false\n[profiles.base]\nskills = ['tdd']\n[aliases.cr]\nharness = 'claude'\nmodel = 'opus'\neffort = 'high'\nprofiles = ['team']\n");
    home.skill(".claude/skills/tdd", "tdd");
    home.skill("tools/lint-skill", "lint");
    fs::write(
        home.dir.path().join("ayran.toml"),
        "[skills.lint]\nall = { path = 'tools/lint-skill' }\ncodex = false\n[profiles.team]\nprofiles = ['base']\nplugins = ['review']\nskills = ['lint']\ndefault = true\n[disable]\nskills = ['tdd']\n",
    )
    .unwrap();
    fs::write(
        home.dir.path().join("ayran.local.toml"),
        "[harnesses.claude]\neffort = 'low'\n",
    )
    .unwrap();
    home.claude_installs(r#"{"version":2,"plugins":{"review@acme":[{"scope":"user"}]}}"#);
    for (flags, model, effort) in [
        (vec![], "sonnet", "low"),
        (vec!["--alias", "cr", "--skill", "tdd"], "opus", "high"),
    ] {
        let mut args = flags;
        args.push("--dry-run");
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let command = String::from_utf8(output.stdout).unwrap();
        assert!(
            command.contains(&format!("--model {model} --effort {effort}")),
            "{command}"
        );
        assert!(command.contains(r#""review@acme":true"#), "{command}");
        assert!(command.contains("--add-dir"), "{command}");
        let trace = String::from_utf8(output.stderr).unwrap();
        assert!(trace.contains("Skill lint:"), "{trace}");
        if model == "opus" {
            assert!(
                trace.contains("Skill tdd: selected (natively on"),
                "{trace}"
            );
            assert!(trace.contains("Skill tdd: explicit (--skill)"), "{trace}");
        } else {
            assert!(command.contains(r#""tdd":"off""#), "{command}");
            assert!(trace.contains("Skill tdd: disabled by layer"), "{trace}");
        }
    }
    let output = home.run(&["--codex", "--no-profile", "base", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "codex --disable apps\n"
    );
    let notes = String::from_utf8(output.stderr).unwrap();
    assert_eq!(notes.matches("note[binding-skipped]").count(), 2, "{notes}");
    assert!(!home.dir.path().join("cache").exists());
}

#[test]
fn copilot_default_and_alias_paths_recognize_selected_direct_installs() {
    let home = TestHome::new();
    let direct = home
        .dir
        .path()
        .join(".copilot/installed-plugins/_direct/source-id");
    fs::create_dir_all(&direct).unwrap();
    let link = home.dir.path().join("working-plugin");
    std::os::unix::fs::symlink(&direct, &link).unwrap();
    home.config("[plugins.direct]\ncopilot = { path = '../../working-plugin' }\ndefault = true\n[aliases.work]\nharness = 'copilot'\nplugins = ['direct']\ndefaults = false\n");
    for args in [
        vec!["--copilot", "--dry-run"],
        vec!["--alias", "work", "--dry-run"],
    ] {
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let command = without_session_id(&String::from_utf8(output.stdout).unwrap());
        assert!(
            command.starts_with("env -u COPILOT_SKILLS_DIRS copilot --plugin-dir "),
            "{command}"
        );
        assert!(
            !String::from_utf8(output.stderr)
                .unwrap()
                .contains("hidden (unselected direct install)")
        );
    }
}

#[test]
fn claude_reports_synced_plugin_leaks_without_guessing_names_or_changing_state() {
    let home = TestHome::new();
    let claude_home = home.dir.path().join("custom claude");
    // Cached payloads are not a complete account inventory: another Plugin can
    // arrive asynchronously after launch. Do not invent native install records.
    fs::create_dir_all(claude_home.join("plugins/synced/cached")).unwrap();
    fs::write(claude_home.join("plugins/synced/cached/payload"), "cached").unwrap();
    let before = snapshot(&claude_home);
    for dry_run in [false, true] {
        let mut command = home.command();
        command
            .env("CLAUDE_CONFIG_DIR", &claude_home)
            .arg("--claude");
        if dry_run {
            command.arg("--dry-run");
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("warning[leak]"), "{stderr}");
        assert!(stderr.contains("claude-synced-plugin"), "{stderr}");
        assert!(!stderr.contains("codex-remote-plugin"), "{stderr}");
        let argv = if dry_run {
            String::from_utf8(output.stdout).unwrap()
        } else {
            home.record()
        };
        assert!(!argv.contains("@synced"), "{argv}");
    }
    assert_eq!(snapshot(&claude_home), before);
    for flag in ["--claude", "--codex"] {
        let output = home.run(&[flag, "--dry-run"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("warning[leak]")
        );
        let output = home.run(&[flag, "-q"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
    }
    let output = home.run(&["--copilot", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        !String::from_utf8(output.stderr)
            .unwrap()
            .contains("warning[leak]")
    );
}

#[test]
fn codex_reports_remote_plugin_leaks_while_hiding_local_installs() {
    let home = TestHome::new();
    home.config("[plugins.selected]\ncodex = 'selected@m'\n");
    home.codex_config(
        "[plugins.\"selected@m\"]\nenabled = false\n[plugins.\"other@m\"]\nenabled = true\n",
    );
    let codex_home = home.dir.path().join(".codex");
    let before = snapshot(&codex_home);
    for dry_run in [false, true] {
        let mut args = vec!["--codex", "--plugin", "selected"];
        if dry_run {
            args.push("--dry-run");
        }
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("warning[leak]"), "{stderr}");
        assert!(stderr.contains("codex-remote-plugin"), "{stderr}");
        let command = if dry_run {
            String::from_utf8(output.stdout).unwrap()
        } else {
            home.record()
        };
        assert!(command.contains("other@m.enabled=false"), "{command}");
        assert!(command.contains("selected@m.enabled=true"), "{command}");
    }
    assert_eq!(snapshot(&codex_home), before);
    let output = home.run(&["--codex", "-q"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn claude_hides_personal_skills_dir_plugins_without_a_registry() {
    let home = TestHome::new();
    home.skill(".claude/skills/plugin", "plugin-skill");
    let manifest = home.dir.path().join(".claude/skills/plugin/.claude-plugin");
    fs::create_dir(&manifest).unwrap();
    fs::write(manifest.join("plugin.json"), r#"{"name":"plugin"}"#).unwrap();
    let before = snapshot(&home.dir.path().join(".claude"));
    let output = home.run(&["--claude", "--dry-run"]);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(r#""plugin@skills-dir":false"#), "{stdout}");
    assert!(home.no_record());
    let output = home.run(&["--claude"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings(),
        serde_json::json!({
            "enabledPlugins": {"plugin@skills-dir": false},
            "disableClaudeAiConnectors": true,
            "syncClaudeAiSkills": false
        })
    );
    assert_eq!(snapshot(&home.dir.path().join(".claude")), before);
}

#[test]
fn claude_selects_skills_dir_plugins_by_manifest_name_in_merged_settings() {
    let home = TestHome::new();
    home.skill(".claude/skills/folder", "plugin-skill");
    home.skill(".claude/skills/personal", "personal");
    let manifest = home.dir.path().join(".claude/skills/folder/.claude-plugin");
    fs::create_dir(&manifest).unwrap();
    fs::write(manifest.join("plugin.json"), r#"{"name":"manifest-name"}"#).unwrap();
    home.claude_installs(r#"{"version":2,"plugins":{"other@m":[{"scope":"user"}]}}"#);
    fs::write(
        home.dir.path().join(".claude/settings.json"),
        r#"{"enabledPlugins":{"manifest-name@skills-dir":false}}"#,
    )
    .unwrap();
    home.config("[plugins.x]\nclaude = 'manifest-name@skills-dir'\n");
    let before = snapshot(&home.dir.path().join(".claude"));
    let output = home.run(&["--claude", "--plugin", "x"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings(),
        serde_json::json!({
            "enabledPlugins": {"manifest-name@skills-dir": true, "other@m": false},
            "skillOverrides": {"personal": "off"},
            "disableClaudeAiConnectors": true,
            "syncClaudeAiSkills": false
        })
    );
    assert_eq!(
        home.record()
            .lines()
            .filter(|arg| *arg == "--settings")
            .count(),
        1
    );
    assert_eq!(snapshot(&home.dir.path().join(".claude")), before);
}

#[test]
fn claude_leaves_project_skills_dir_plugins_and_shared_names_alone() {
    let home = TestHome::new();
    fs::create_dir_all(home.dir.path().join("project/subdir")).unwrap();
    fs::create_dir_all(home.dir.path().join("project/.git")).unwrap();
    fs::write(
        home.dir.path().join("project/.git").join("HEAD"),
        "ref: refs/heads/main",
    )
    .unwrap();
    for (relative, name) in [
        (".claude/skills/personal", "shared"),
        ("project/.claude/skills/project", "shared"),
        ("project/.claude/skills/team", "team"),
    ] {
        let manifest = home.dir.path().join(relative).join(".claude-plugin");
        fs::create_dir_all(&manifest).unwrap();
        fs::write(
            manifest.join("plugin.json"),
            format!(r#"{{"name":"{name}"}}"#),
        )
        .unwrap();
    }
    home.config("[plugins.team]\nclaude = 'team@skills-dir'\n");
    let output = home
        .command()
        .current_dir(home.dir.path().join("project/subdir"))
        .args(["--claude"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(home.claude_settings().get("enabledPlugins").is_none());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("warning[leak]")
            && stderr.contains("claude-project-plugin-shadow")
            && stderr.contains("shared@skills-dir"),
        "{stderr}"
    );
    let output = home
        .command()
        .current_dir(home.dir.path().join("project/subdir"))
        .args(["--claude", "--plugin", "team"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings()["enabledPlugins"],
        serde_json::json!({"team@skills-dir": true})
    );
    let output = home
        .command()
        .current_dir(home.dir.path().join("project/subdir"))
        .args(["--claude", "--dry-run"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("claude-project-plugin-shadow")
    );
    let output = home
        .command()
        .current_dir(home.dir.path().join("project/subdir"))
        .args(["--claude", "--quiet"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    home.config("[plugins.shared]\nclaude = 'shared@skills-dir'\n");
    let output = home
        .command()
        .current_dir(home.dir.path().join("project/subdir"))
        .args(["--claude", "--plugin", "shared"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings()["enabledPlugins"],
        serde_json::json!({"shared@skills-dir": true})
    );
    assert!(
        !String::from_utf8(output.stderr)
            .unwrap()
            .contains("claude-project-plugin-shadow")
    );
}

#[test]
fn claude_skills_dir_plugins_honor_custom_home_and_skip_reserved_folders() {
    let home = TestHome::new();
    for (relative, name) in [
        ("custom/skills/folder", "custom-plugin"),
        ("custom/skills/.hidden", "hidden"),
        ("custom/skills/SYNCED", "synced"),
        (".claude/skills/ignored", "ignored"),
    ] {
        let manifest = home.dir.path().join(relative).join(".claude-plugin");
        fs::create_dir_all(&manifest).unwrap();
        fs::write(
            manifest.join("plugin.json"),
            format!(r#"{{"name":"{name}"}}"#),
        )
        .unwrap();
    }
    // A dotfiles repository must not reclassify personal Plugins as project Plugins.
    fs::create_dir(home.dir.path().join(".git")).unwrap();
    fs::write(
        home.dir.path().join(".git").join("HEAD"),
        "ref: refs/heads/main",
    )
    .unwrap();
    let output = home
        .command()
        .env("CLAUDE_CONFIG_DIR", "custom")
        .args(["--claude"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings()["enabledPlugins"],
        serde_json::json!({"custom-plugin@skills-dir": false})
    );
}

#[test]
fn claude_refuses_malformed_skills_dir_plugin_manifests_before_launch() {
    for contents in [
        "not JSON",
        "[]",
        "{}",
        r#"{"name":false}"#,
        r#"{"name":""}"#,
        r#"{"name":"bad name"}"#,
        r#"{"name":"bad@name"}"#,
        r#"{"name":"bad:name"}"#,
        r#"{"name":"bad/name"}"#,
        r#"{"name":"bad\\name"}"#,
        r#"{"name":"bad\u0000name"}"#,
        r#"{"name":"bad\u202ename"}"#,
    ] {
        let home = TestHome::new();
        let manifest = home.dir.path().join(".claude/skills/plugin/.claude-plugin");
        fs::create_dir_all(&manifest).unwrap();
        fs::write(manifest.join("plugin.json"), contents).unwrap();
        let before = snapshot(&home.dir.path().join(".claude"));
        for args in [vec!["--claude"], vec!["--claude", "--dry-run"]] {
            let output = home.run(&args);
            assert_eq!(output.status.code(), Some(3), "{contents}: {output:?}");
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(stderr.contains("error[enumeration-failed]"), "{stderr}");
            assert!(stderr.contains("plugin.json"), "{stderr}");
            assert!(home.no_record());
        }
        assert_eq!(snapshot(&home.dir.path().join(".claude")), before);
    }
}

#[test]
fn claude_hides_skills_dir_plugins_linked_from_a_working_copy() {
    let home = TestHome::new();
    let source = home.dir.path().join("source/.claude-plugin");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("plugin.json"), r#"{"name":"linked"}"#).unwrap();
    let personal = home.dir.path().join(".claude/skills");
    fs::create_dir_all(&personal).unwrap();
    std::os::unix::fs::symlink(home.dir.path().join("source"), personal.join("link")).unwrap();
    let output = home.run(&["--claude"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings()["enabledPlugins"],
        serde_json::json!({"linked@skills-dir": false})
    );
}

#[test]
fn claude_hides_only_unselected_installed_user_plugins() {
    let home = TestHome::new();
    home.config("[plugins.a]\nall = 'a@m'\n");
    home.claude_installs(
        r#"{"version":2,"plugins":{"a@m":[{"scope":"user"}],"b@m":[{"scope":"user"}]}}"#,
    );
    let output = home.run(&["--claude", "--plugin", "a"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.record(),
        "--settings\n{\"disableClaudeAiConnectors\":true,\"enabledPlugins\":{\"a@m\":true,\"b@m\":false},\"syncClaudeAiSkills\":false}\n"
    );
}

#[test]
fn claude_hides_user_installs_without_changing_its_home_or_project_plugins() {
    let home = TestHome::new();
    let root = home.dir.path();
    let registry = serde_json::json!({
        "version": 2,
        "plugins": {
            "a@m": [{"scope": "user"}, {"scope": "user"}],
            "b@m": [{"scope": "user"}],
            "project@m": [{"scope": "project", "projectPath": root}],
            "local@m": [{"scope": "local", "projectPath": root}],
            "shared@m": [{"scope": "user"}, {"scope": "project", "projectPath": root}],
            "elsewhere@m": [{"scope": "user"}, {"scope": "project", "projectPath": root.join("other")}]
        }
    });
    home.claude_installs(&registry.to_string());
    let claude_home = root.join(".claude");
    fs::write(
        claude_home.join("settings.json"),
        r#"{"enabledPlugins":{"a@m":false,"b@m":true}}"#,
    )
    .unwrap();
    fs::create_dir_all(claude_home.join("plugins/cache/m/a/1.0")).unwrap();
    fs::write(
        claude_home.join("plugins/cache/m/a/1.0/payload"),
        b"plugin contents",
    )
    .unwrap();
    home.config("[plugins.not_installed]\nall = 'never-guess@m'\n");
    let before = snapshot(&claude_home);
    let output = home.run(&["--claude", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "claude --settings '{\"disableClaudeAiConnectors\":true,\"enabledPlugins\":{\"b@m\":false,\"elsewhere@m\":false},\"syncClaudeAiSkills\":false}'\n"
    );
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(
        trace.contains("Plugin a@m: hidden (natively off"),
        "{trace}"
    );
    for id in ["b@m", "elsewhere@m"] {
        assert!(
            trace.contains(&format!("Plugin {id}: hidden (unselected user install)")),
            "{trace}"
        );
    }
    assert_eq!(snapshot(&claude_home), before);
    assert!(home.no_record());
    let output = home.run(&["--claude", "--", "--settings", "passthrough"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.record(),
        "--settings\n{\"disableClaudeAiConnectors\":true,\"enabledPlugins\":{\"a@m\":false,\"b@m\":false,\"elsewhere@m\":false},\"syncClaudeAiSkills\":false}\n--settings\npassthrough\n"
    );
    assert_eq!(snapshot(&claude_home), before);
}

#[test]
fn claude_reads_only_its_selected_home_for_flags_aliases_and_default_harness() {
    let home = TestHome::new();
    home.config("default_harness = 'claude'\n[aliases.review]\nharness = 'claude'\n[plugins.selected]\nall = 'custom@m'\n");
    home.claude_installs("corrupt unused home");
    let custom = home.dir.path().join("custom claude");
    fs::create_dir_all(custom.join("plugins")).unwrap();
    fs::write(
        custom.join("plugins/installed_plugins.json"),
        r#"{"version":2,"plugins":{"custom@m":[{"scope":"user"}]}}"#,
    )
    .unwrap();
    let before = snapshot(&custom);
    for args in [
        vec!["--claude", "--plugin", "selected", "--dry-run"],
        vec!["--alias", "review", "--plugin", "selected", "--dry-run"],
        vec!["--plugin", "selected", "--dry-run"],
    ] {
        let output = home
            .command()
            .env("CLAUDE_CONFIG_DIR", &custom)
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            without_session_id(&String::from_utf8(output.stdout).unwrap()),
            "claude --settings '{\"disableClaudeAiConnectors\":true,\"enabledPlugins\":{\"custom@m\":true},\"syncClaudeAiSkills\":false}'\n"
        );
    }
    assert_eq!(snapshot(&custom), before);
    for args in [
        vec!["--codex", "--dry-run"],
        vec!["--copilot", "--dry-run"],
        vec!["activate", "bash"],
    ] {
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }
    let output = home
        .command()
        .env("CLAUDE_CONFIG_DIR", &custom)
        .args(["--claude"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.record(),
        "--settings\n{\"disableClaudeAiConnectors\":true,\"enabledPlugins\":{\"custom@m\":false},\"syncClaudeAiSkills\":false}\n"
    );
    assert_eq!(snapshot(&custom), before);
}

#[test]
fn claude_missing_or_empty_plugin_registry_still_disables_synced_skills() {
    for contents in [None, Some(r#"{"version":2,"plugins":{}}"#)] {
        let home = TestHome::new();
        if let Some(contents) = contents {
            home.claude_installs(contents);
        }
        let output = home.run(&["--claude", "--dry-run"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            without_session_id(&String::from_utf8(output.stdout).unwrap()),
            "claude --settings '{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}'\n"
        );
        assert!(home.no_record());
    }
}

#[test]
fn claude_rejects_native_bindings_that_are_not_installed() {
    let home = TestHome::new();
    home.config("[plugins.missing]\nall = 'missing@m'\n");
    for dry_run in [false, true] {
        let mut command = home.command();
        command.args(["--claude", "--plugin", "missing"]);
        if dry_run {
            command.arg("--dry-run");
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("error[native-not-found]"), "{stderr}");
        assert!(stderr.contains("missing@m"), "{stderr}");
        assert!(output.stdout.is_empty());
        assert!(home.no_record());
    }
}

#[test]
fn claude_refuses_unreadable_or_invalid_installed_plugin_state() {
    for contents in [
        "not JSON",
        r#"{}"#,
        r#"{"plugins":{}}"#,
        r#"{"version":3,"plugins":{}}"#,
        r#"{"version":2,"plugins":[]}"#,
        r#"{"version":2,"plugins":{"a@m":{}}}"#,
        r#"{"version":2,"plugins":{"a@m":[{}]}}"#,
        r#"{"version":2,"plugins":{"a@m":[{"scope":"unknown"}]}}"#,
        r#"{"version":2,"plugins":{"a@m":[{"scope":"project"}]}}"#,
    ] {
        let home = TestHome::new();
        home.claude_installs(contents);
        for args in [vec!["--claude"], vec!["--claude", "--dry-run"]] {
            let output = home.run(&args);
            assert_eq!(output.status.code(), Some(3), "{contents}: {output:?}");
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(stderr.contains("error[enumeration-failed]"), "{stderr}");
            assert!(stderr.contains("installed_plugins.json"), "{stderr}");
            assert!(output.stdout.is_empty());
            assert!(home.no_record());
        }
    }
    let home = TestHome::new();
    // A directory at the file's location causes a read failure even under root.
    fs::create_dir_all(
        home.dir
            .path()
            .join(".claude/plugins/installed_plugins.json"),
    )
    .unwrap();
    let output = home.run(&["--claude"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[enumeration-failed]")
    );
    assert!(home.no_record());
}

#[test]
fn selected_native_plugin_is_enabled_on_claude_and_explained_in_dry_run() {
    let home = TestHome::new();
    home.claude_installs(
        r#"{"version":2,"plugins":{"dotnet-msbuild@dotnet-agent-skills":[{"scope":"user"}]}}"#,
    );
    home.config("[plugins.msbuild]\nall = 'dotnet-msbuild@dotnet-agent-skills'\ndescription = 'Build .NET'\n");
    let output = home.run(&["--claude", "--plugin", "msbuild", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "claude --settings '{\"disableClaudeAiConnectors\":true,\"enabledPlugins\":{\"dotnet-msbuild@dotnet-agent-skills\":true},\"syncClaudeAiSkills\":false}'\n"
    );
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(
        trace.contains(
            "Plugin settings: {\"enabledPlugins\":{\"dotnet-msbuild@dotnet-agent-skills\":true}}"
        ),
        "{trace}"
    );
    assert!(
        trace.contains("Plugin msbuild: explicit (--plugin)"),
        "{trace}"
    );
    assert!(
        trace.contains("dotnet-msbuild@dotnet-agent-skills"),
        "{trace}"
    );
    assert!(trace.contains("config/ayran/ayran.toml"), "{trace}");
    assert!(home.no_record());
    let output = home.run(&["--claude", "--plugin", "msbuild"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.record(),
        "--settings\n{\"disableClaudeAiConnectors\":true,\"enabledPlugins\":{\"dotnet-msbuild@dotnet-agent-skills\":true},\"syncClaudeAiSkills\":false}\n"
    );
}

#[test]
fn plugin_harness_binding_overrides_all_and_native_items_are_deduplicated() {
    let home = TestHome::new();
    home.claude_installs(r#"{"version":2,"plugins":{"review@m":[{"scope":"user"}]}}"#);
    home.config("[plugins.review]\nall = 'fallback@m'\nclaude = 'review@m'\ncodex = false\ncopilot = 'other@m'\n[plugins.second]\nall = 'review@m'\n");
    let output = home.run(&[
        "--claude",
        "--plugin",
        "review,second",
        "--plugin",
        "review",
        "--",
        "--settings",
        "user override",
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.record(),
        "--settings\n{\"disableClaudeAiConnectors\":true,\"enabledPlugins\":{\"review@m\":true},\"syncClaudeAiSkills\":false}\n--settings\nuser override\n"
    );
    let output = home.run(&["--claude", "--plugin", "review,second", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let trace = String::from_utf8(output.stderr).unwrap();
    for name in ["review", "second"] {
        assert!(
            trace.contains(&format!("Plugin {name}: explicit (--plugin) → review@m")),
            "{trace}"
        );
    }
}

#[test]
fn plugin_paths_are_relative_to_the_declaring_layer_and_expand_home() {
    let home = TestHome::new();
    let user_plugin = home.dir.path().join("config/ayran/user plugin");
    let home_plugin = home.dir.path().join("dev/plugin");
    let project = home.dir.path().join("project");
    let local_plugin = project.join("local plugin");
    for path in [&user_plugin, &home_plugin, &local_plugin] {
        fs::create_dir_all(path).unwrap();
    }
    home.config("[plugins.user]\nall = { path = 'user plugin' }\n[plugins.home]\nclaude = { path = '~/dev/plugin' }\n[plugins.local]\nall = 'farther@m'\n");
    fs::write(
        project.join("ayran.toml"),
        "[plugins.local]\nall = 'project@m'\n",
    )
    .unwrap();
    fs::write(
        project.join("ayran.local.toml"),
        "[plugins.local]\nclaude = { path = 'local plugin' }\n",
    )
    .unwrap();
    let output = home
        .command()
        .current_dir(&project)
        .args(["--claude", "--plugin", "user,home,local"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.record(),
        format!(
            "--settings\n{{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}}\n--plugin-dir\n{}\n--plugin-dir\n{}\n--plugin-dir\n{}\n",
            user_plugin.display(),
            home_plugin.display(),
            local_plugin.display()
        )
    );
    let output = home
        .command()
        .current_dir(&project)
        .args(["--claude", "--plugin", "local", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(
        trace.contains(&local_plugin.display().to_string()),
        "{trace}"
    );
    assert!(trace.contains("ayran.local.toml"), "{trace}");
}

#[test]
fn invalid_plugin_selections_fail_without_starting_a_session() {
    for (config, name, code) in [
        ("", "unknown", "unknown-plugin"),
        ("[plugins.x]\ncodex = 'x@m'\n", "x", "missing-binding"),
        (
            "[plugins.x]\nall = 'x@m'\nclaude = false\n",
            "x",
            "binding-absent",
        ),
        ("[plugins.x]\nall = false\n", "x", "binding-absent"),
        (
            "[plugins.x]\nall = { path = 'missing' }\n",
            "x",
            "path-not-found",
        ),
    ] {
        let home = TestHome::new();
        home.config(config);
        for dry_run in [false, true] {
            let mut command = home.command();
            command.args(["--claude", "--plugin", name]);
            if dry_run {
                command.arg("--dry-run");
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(3), "{config}: {output:?}");
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(stderr.contains(&format!("error[{code}]")), "{stderr}");
            assert!(output.stdout.is_empty());
            assert!(home.no_record());
        }
    }
}

#[test]
fn codex_hides_unselected_plugins_and_force_enables_a_disabled_selection() {
    let home = TestHome::new();
    home.config("[plugins.selected]\nall = 'a@m'\n[plugins.same]\ncodex = 'a@m'\n");
    home.codex_config("model = 'user-model'\n[plugins.'a@m']\nenabled = false\n[plugins.'b@m']\nenabled = true\n[plugins.'c.d@m']\nenabled = false\n");
    let codex_home = home.dir.path().join(".codex");
    let before = snapshot(&codex_home);
    let output = home.run(&["--codex", "--plugin", "selected,same", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "codex -c 'plugins.a@m.enabled=true' -c 'plugins.b@m.enabled=false' --disable apps\n"
    );
    let trace = String::from_utf8(output.stderr).unwrap();
    for id in ["b@m", "c.d@m"] {
        assert!(
            trace.contains(&format!("Plugin {id}: hidden (unselected user install)")),
            "{trace}"
        );
    }
    assert!(home.no_record());
    let output = home.run(&[
        "--codex",
        "--plugin",
        "selected,same",
        "--",
        "-c",
        "plugins.x.enabled=true",
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.record(),
        "-c\nplugins.a@m.enabled=true\n-c\nplugins.b@m.enabled=false\n--disable\napps\n-c\nplugins.x.enabled=true\n"
    );
    assert_eq!(snapshot(&codex_home), before);
}

#[test]
fn codex_native_bindings_must_name_an_installed_plugin() {
    for contents in [None, Some("[plugins.'other@m']\nenabled = true\n")] {
        let home = TestHome::new();
        home.config("[plugins.missing]\nall = 'missing@m'\n");
        if let Some(contents) = contents {
            home.codex_config(contents);
        }
        for dry_run in [false, true] {
            let mut command = home.command();
            command.args(["--codex", "--plugin", "missing"]);
            if dry_run {
                command.arg("--dry-run");
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(3), "{output:?}");
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(stderr.contains("error[native-not-found]"), "{stderr}");
            assert!(
                stderr.contains("missing@m") && stderr.contains("codex"),
                "{stderr}"
            );
            assert!(home.no_record());
        }
    }
}

#[test]
fn codex_stale_registration_is_not_an_installed_native_plugin() {
    let home = TestHome::new();
    home.config("[plugins.stale]\nall = 'stale@m'\n");
    home.codex_config("[plugins.'stale@m']\nenabled = false\n");
    fs::remove_dir_all(home.dir.path().join(".codex/plugins/cache/m/stale")).unwrap();
    let output = home.run(&["--codex", "--plugin", "stale", "--dry-run"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[native-not-found]")
    );
    assert!(home.no_record());
}

#[test]
fn codex_refuses_unreadable_or_invalid_plugin_state() {
    for contents in [
        "[broken",
        "plugins = []",
        "[plugins]\nx = false",
        "[plugins.'x@m']\nenabled = 'yes'",
        "[plugins.'../x@m']\nenabled = true",
        "[plugins.'x@../m']\nenabled = true",
    ] {
        let home = TestHome::new();
        home.codex_config(contents);
        for dry_run in [false, true] {
            let mut command = home.command();
            command.arg("--codex");
            if dry_run {
                command.arg("--dry-run");
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(3), "{contents}: {output:?}");
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(
                stderr.contains("error[enumeration-failed]") && stderr.contains("config.toml"),
                "{stderr}"
            );
            assert!(home.no_record());
        }
    }
    let home = TestHome::new();
    fs::create_dir_all(home.dir.path().join(".codex/config.toml")).unwrap();
    let output = home.run(&["--codex"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[enumeration-failed]")
    );
    assert!(home.no_record());

    let home = TestHome::new();
    home.codex_config("[plugins.'x@m']\nenabled = true");
    let cache = home.dir.path().join(".codex/plugins/cache/m/x");
    fs::remove_dir_all(&cache).unwrap();
    fs::write(cache, "not a directory").unwrap();
    let output = home.run(&["--codex", "--dry-run"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[enumeration-failed]")
    );
}

#[test]
fn codex_missing_or_empty_plugin_state_needs_no_overrides() {
    for contents in [None, Some("model = 'user-model'"), Some("[plugins]")] {
        let home = TestHome::new();
        if let Some(contents) = contents {
            home.codex_config(contents);
        }
        let output = home.run(&["--codex", "--dry-run"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "codex --disable apps\n"
        );
    }
}

#[test]
fn codex_reads_only_the_selected_home_for_flags_aliases_and_default_harness() {
    let home = TestHome::new();
    home.config("default_harness = 'codex'\n[aliases.review]\nharness = 'codex'\n[plugins.selected]\nall = 'custom@m'\n");
    home.codex_config("corrupt unused home");
    let custom = home.dir.path().join("custom codex");
    fs::create_dir_all(custom.join("plugins/cache/m/custom/1.2.3")).unwrap();
    fs::write(
        custom.join("config.toml"),
        "[plugins.'custom@m']\nenabled = false",
    )
    .unwrap();
    let before = snapshot(&custom);
    let project = home.dir.path().join("project");
    fs::create_dir(&project).unwrap();
    for args in [
        vec!["--codex", "--plugin", "selected", "--dry-run"],
        vec!["--alias", "review", "--plugin", "selected", "--dry-run"],
        vec!["--plugin", "selected", "--dry-run"],
    ] {
        let output = home
            .command()
            .current_dir(&project)
            .env("CODEX_HOME", &custom)
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "codex -c 'plugins.custom@m.enabled=true' --disable apps\n"
        );
    }
    let output = home
        .command()
        .current_dir(&project)
        .env("CODEX_HOME", &custom)
        .args(["--codex"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(home.record(), "--disable\napps\n");
    assert_eq!(snapshot(&custom), before);
    for args in [
        vec!["--claude", "--dry-run"],
        vec!["--copilot", "--dry-run"],
        vec!["activate", "bash"],
    ] {
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }
}

#[test]
fn selected_native_plugins_are_force_enabled_on_codex() {
    let home = TestHome::new();
    home.codex_config("[plugins.'dotnet-msbuild@dotnet-agent-skills']\nenabled = false\n");
    home.config("[plugins.msbuild]\nall = 'dotnet-msbuild@dotnet-agent-skills'\n");
    let output = home.run(&["--codex", "--plugin", "msbuild", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "codex -c 'plugins.dotnet-msbuild@dotnet-agent-skills.enabled=true' --disable apps\n"
    );
    assert!(home.no_record());
    let output = home.run(&[
        "--codex",
        "--plugin",
        "msbuild",
        "--",
        "-c",
        "plugins.x.enabled=false",
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.record(),
        "-c\nplugins.dotnet-msbuild@dotnet-agent-skills.enabled=true\n--disable\napps\n-c\nplugins.x.enabled=false\n"
    );
}

#[test]
fn nearer_plugin_definitions_replace_the_whole_table_and_unused_gaps_are_lazy() {
    let home = TestHome::new();
    home.config("[plugins.x]\nall = 'farther@m'\nclaude = 'old@m'\n");
    fs::write(
        home.dir.path().join("ayran.toml"),
        "[plugins.x]\ncodex = 'nearer@m'\n",
    )
    .unwrap();
    let output = home.run(&["--claude", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let output = home.run(&["--claude", "--plugin", "x", "--dry-run"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[missing-binding]")
    );
    assert!(home.no_record());
}

#[test]
fn plugin_config_rejects_malformed_bindings_and_future_selection_keys() {
    for (config, reason) in [
        ("plugins = []", "plugins must be a table"),
        ("[plugins]\nx = 'x@m'", "plugins.x must be a table"),
        ("[plugins.'x,y']\nall = 'x@m'", "invalid-name"),
        ("[plugins.x]\nall = true", "config-invalid"),
        ("[plugins.x]\nclaude = 42", "config-invalid"),
        ("[plugins.x]\nall = { path = 42 }", "path must be a string"),
        (
            "[plugins.x]\nall = { path = '.', extra = 'bad' }",
            "config-invalid",
        ),
        ("[plugins.x]\nall = { command = 'bad' }", "config-invalid"),
        (
            "[plugins.x]\ndescription = false",
            "description must be a string",
        ),
        ("[plugins.x]\nextra = 'bad'", "unknown key plugins.x.extra"),
        ("[plugins.x]\ndefault = 'true'", "default must be a boolean"),
        (
            "[disable]\nplugins = 'a'",
            "plugins must be an array of strings",
        ),
        (
            "[disable]\nplugins = [1]",
            "plugins must be an array of strings",
        ),
        ("[disable]\ndefaults = []", "defaults must be a boolean"),
        ("[disable]\nextra = []", "unknown key disable.extra"),
        ("[disable]\nskills = 42", "skills must be an array"),
        (
            "[aliases.cr]\nharness = 'claude'\nplugins = [false]",
            "plugins must be an array of strings",
        ),
        (
            "[aliases.cr]\nharness = 'claude'\ndefaults = 'false'",
            "defaults must be a boolean",
        ),
        (
            "[aliases.cr]\nharness = 'claude'\ndisable = []",
            "disable must be a table",
        ),
        (
            "[aliases.cr]\nharness = 'claude'\ndisable = { mcp = [1] }",
            "mcp must be an array of strings",
        ),
    ] {
        let home = TestHome::new();
        home.config(config);
        let output = home.run(&["--claude", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{config}: {output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(reason), "{stderr}");
        assert!(output.stdout.is_empty());
        assert!(home.no_record());
    }
}

#[test]
fn activate_prints_dispatchers_for_each_shell() {
    let home = TestHome::new();
    home.config(
        "[aliases.cr]\nharness = 'claude'\nmodel = 'opus'\n[aliases.my-task]\nharness = 'codex'\n",
    );
    for shell in ["bash", "zsh"] {
        let output = home.run(&["activate", shell]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(String::from_utf8(output.stdout).unwrap().starts_with(
            "cr() { ayran --alias cr \"$@\"; }\nmy-task() { ayran --alias my-task \"$@\"; }\n"
        ));
        assert!(output.stderr.is_empty());
    }
    let output = home.run(&["activate", "pwsh"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let activation = String::from_utf8(output.stdout).unwrap();
    assert!(activation.starts_with(
        "function cr { ayran --alias cr @args }\nfunction my-task { ayran --alias my-task @args }\n"
    ));
    assert!(activation.contains("Register-ArgumentCompleter -Native -CommandName 'ayran'"));
    for name in ["cr", "my-task"] {
        assert!(activation.contains(&format!("Register-ArgumentCompleter -CommandName '{name}'")));
    }
}

#[test]
fn activate_reads_only_user_config_and_handles_empty_aliases() {
    let home = TestHome::new();
    let pwsh = home.run(&["activate", "pwsh"]);
    assert!(pwsh.status.success() && pwsh.stderr.is_empty(), "{pwsh:?}");
    assert!(
        String::from_utf8(pwsh.stdout)
            .unwrap()
            .contains("Register-ArgumentCompleter -Native -CommandName 'ayran'")
    );
    let empty = home.run(&["activate", "bash"]);
    assert_eq!(empty.status.code(), Some(0), "{empty:?}");
    assert!(
        String::from_utf8(empty.stdout)
            .unwrap()
            .contains("complete -F _ayran_complete ayran")
    );
    home.config("[aliases.cr]\nharness = 'claude'\n");
    let project = home.dir.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(
        project.join("ayran.toml"),
        "[aliases.bad]\nharness = 'codex'\n",
    )
    .unwrap();
    let output = home
        .command()
        .current_dir(project)
        .args(["activate", "bash"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let activation = String::from_utf8(output.stdout).unwrap();
    assert!(activation.starts_with("cr() { ayran --alias cr \"$@\"; }\n"));
    assert!(!activation.contains("bad()"));
}

#[test]
fn activate_rejects_reserved_aliases_without_partial_output() {
    for name in ["if", "cd", "echo", "ayran", "foreach"] {
        let home = TestHome::new();
        home.config(&format!(
            "[aliases.cr]\nharness = 'claude'\n[aliases.{name}]\nharness = 'codex'\n"
        ));
        let output = home.run(&["activate", "bash"]);
        assert_eq!(output.status.code(), Some(3), "{name}: {output:?}");
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("error[alias-reserved-name]")
        );
    }
}

#[test]
fn activate_rejects_unknown_shell() {
    let output = TestHome::new().run(&["activate", "fish"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[usage]")
    );
}

#[test]
fn activated_bash_and_zsh_functions_forward_arguments() {
    let home = TestHome::new();
    home.config("[aliases.cr]\nharness = 'claude'\n");
    for shell in ["bash", "zsh"] {
        let activation = home.run(&["activate", shell]);
        let output = Command::new(shell)
            .arg("-c")
            .arg("ayran() { printf '<%s>\\n' \"$@\"; }; eval \"$ACTIVATION\"; cr 'two words' --dry-run")
            .env("ACTIVATION", String::from_utf8(activation.stdout).unwrap())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{shell}: {output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "<--alias>\n<cr>\n<two words>\n<--dry-run>\n"
        );
    }
}

#[test]
fn alias_launches_with_its_settings_and_reports_sources() {
    let home = TestHome::new();
    home.fake_harness("claude", 0);
    home.config("default_harness = 'codex'\n[harnesses.claude]\nmodel = 'sonnet'\neffort = 'medium'\n[aliases.cr]\nharness = 'claude'\nmodel = 'opus'\neffort = 'high'\ndescription = 'Review'\n");
    fs::write(
        home.dir.path().join("ayran.local.toml"),
        "[harnesses.claude]\neffort = 'low'\n",
    )
    .unwrap();

    let output = home.run(&["--alias", "cr"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.record(),
        "--model\nopus\n--effort\nhigh\n--settings\n{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}\n"
    );

    let output = home.run(&["--alias", "cr", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "env -u CLAUDE_CODE_EFFORT_LEVEL claude --model opus --effort high --settings '{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}'\n"
    );
    let trace = String::from_utf8(output.stderr).unwrap();
    for expected in [
        "Harness: claude (Alias cr)",
        "Model: opus (Alias cr)",
        "Effort: high (Alias cr)",
    ] {
        assert!(trace.contains(expected), "{trace}");
    }
}

#[test]
fn alias_flags_override_individual_values_and_matching_harness_is_allowed() {
    let home = TestHome::new();
    home.config("[harnesses.claude]\nmodel = 'sonnet'\neffort = 'medium'\n[aliases.cr]\nharness = 'claude'\nmodel = 'opus'\neffort = 'high'\n");
    let output = home.run(&["--alias", "cr", "--claude", "-e", "low", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "env -u CLAUDE_CODE_EFFORT_LEVEL claude --model opus --effort low --settings '{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}'\n"
    );
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(trace.contains("Harness: claude (flag)"), "{trace}");
    assert!(trace.contains("Model: opus (Alias cr)"), "{trace}");
    assert!(trace.contains("Effort: low (flag)"), "{trace}");

    let conflict = home.run(&["--alias", "cr", "--codex", "--dry-run"]);
    assert_eq!(conflict.status.code(), Some(2), "{conflict:?}");
    assert!(
        String::from_utf8(conflict.stderr)
            .unwrap()
            .contains("error[usage]")
    );
}

#[test]
fn alias_errors_have_their_own_diagnostic_codes() {
    let home = TestHome::new();
    let unknown = home.run(&["--alias", "nope", "--dry-run"]);
    assert_eq!(unknown.status.code(), Some(3), "{unknown:?}");
    assert!(
        String::from_utf8(unknown.stderr)
            .unwrap()
            .contains("error[unknown-alias]")
    );

    for (contents, code, message) in [
        (
            "[aliases.cr]\npreset = 42\n",
            "config-invalid",
            "preset must be a string",
        ),
        (
            "[aliases.cr]\nharness = 'wrong'\n",
            "config-invalid",
            "harness",
        ),
        (
            "[aliases.cr]\nharness = 'claude'\nmodel = 42\n",
            "config-invalid",
            "model must be a string",
        ),
        (
            "[aliases.cr]\nharness = 'claude'\neffort = 'ultra'\n",
            "config-invalid",
            "effort must be",
        ),
        (
            "[aliases.cr]\nharness = 'claude'\ndescription = 42\n",
            "config-invalid",
            "description must be a string",
        ),
        (
            "[aliases.cr]\nharness = 'claude'\nskills = 42\n",
            "config-invalid",
            "skills must be an array",
        ),
        (
            "[aliases.cr]\nharness = 'claude'\nextra = true\n",
            "config-invalid",
            "unknown key",
        ),
        (
            "[aliases.\"bad.name\"]\nharness = 'claude'\n",
            "invalid-name",
            "bad.name",
        ),
    ] {
        let home = TestHome::new();
        home.config(contents);
        let output = home.run(&["--claude", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{contents}: {output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(&format!("error[{code}]")), "{stderr}");
        assert!(stderr.contains(message), "{stderr}");
    }
}

#[test]
fn user_config_supplies_bare_harness_and_independent_settings() {
    let home = TestHome::new();
    home.fake_harness("claude", 0);
    home.config(
        "default_harness = 'claude'\n[harnesses.claude]\nmodel = 'opus'\neffort = 'high'\n",
    );

    let output = home.run(&[]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.record(),
        "--model\nopus\n--effort\nhigh\n--settings\n{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}\n"
    );

    let output = home.run(&["-m", "sonnet", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "env -u CLAUDE_CODE_EFFORT_LEVEL claude --model sonnet --effort high --settings '{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}'\n"
    );
    let trace = String::from_utf8(output.stderr).unwrap();
    let path = home.dir.path().join("config/ayran/ayran.toml");
    assert!(
        trace.contains(&format!("Harness: claude ({})", path.display())),
        "{trace}"
    );
    assert!(trace.contains("Model: sonnet (flag)"), "{trace}");
    assert!(
        trace.contains(&format!("Effort: high ({})", path.display())),
        "{trace}"
    );

    let output = home.run(&["-e", "low", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "env -u CLAUDE_CODE_EFFORT_LEVEL claude --model opus --effort low --settings '{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}'\n"
    );
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(
        trace.contains(&format!("Model: opus ({})", path.display())),
        "{trace}"
    );
    assert!(trace.contains("Effort: low (flag)"), "{trace}");
}

#[test]
fn project_default_harness_beats_user_config() {
    let home = TestHome::new();
    home.config("default_harness = 'claude'\n");
    let project = home.dir.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("ayran.toml"), "default_harness = 'codex'\n").unwrap();

    let output = home
        .command()
        .current_dir(&project)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "codex --disable apps\n"
    );
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(
        trace.contains(&format!(
            "Harness: codex ({})",
            project.join("ayran.toml").display()
        )),
        "{trace}"
    );
}

#[test]
fn project_layers_merge_harness_settings_by_key() {
    let home = TestHome::new();
    home.config(
        "default_harness = 'codex'\n[harnesses.claude]\nmodel = 'sonnet'\neffort = 'high'\n",
    );
    let project = home.dir.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("ayran.toml"), "default_harness = 'claude'\n").unwrap();
    fs::write(
        project.join("ayran.local.toml"),
        "[harnesses.claude]\neffort = 'low'\n",
    )
    .unwrap();

    let output = home
        .command()
        .current_dir(&project)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "env -u CLAUDE_CODE_EFFORT_LEVEL claude --model sonnet --effort low --settings '{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}'\n"
    );
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(
        trace.contains(&format!(
            "Harness: claude ({})",
            project.join("ayran.toml").display()
        )),
        "{trace}"
    );
    assert!(
        trace.contains(&format!(
            "Model: sonnet ({})",
            home.dir.path().join("config/ayran/ayran.toml").display()
        )),
        "{trace}"
    );
    assert!(
        trace.contains(&format!(
            "Effort: low ({})",
            project.join("ayran.local.toml").display()
        )),
        "{trace}"
    );
}

#[test]
fn local_file_and_nearer_directory_win() {
    let home = TestHome::new();
    let parent = home.dir.path().join("parent");
    let child = parent.join("child");
    fs::create_dir_all(&child).unwrap();
    fs::write(
        parent.join("ayran.toml"),
        "default_harness = 'claude'\n[harnesses.claude]\nmodel = 'parent'\neffort = 'high'\n",
    )
    .unwrap();
    fs::write(
        parent.join("ayran.local.toml"),
        "[harnesses.claude]\nmodel = 'local'\n",
    )
    .unwrap();
    fs::write(
        child.join("ayran.toml"),
        "[harnesses.claude]\neffort = 'medium'\n",
    )
    .unwrap();
    let output = home
        .command()
        .current_dir(&child)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "env -u CLAUDE_CODE_EFFORT_LEVEL claude --model local --effort medium --settings '{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}'\n"
    );
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(
        trace.contains(&format!(
            "Model: local ({})",
            parent.join("ayran.local.toml").display()
        )),
        "{trace}"
    );
    assert!(
        trace.contains(&format!(
            "Effort: medium ({})",
            child.join("ayran.toml").display()
        )),
        "{trace}"
    );
}

#[test]
fn user_config_inside_project_tree_is_loaded_once() {
    let home = TestHome::new();
    let parent = home.dir.path().join("parent");
    let child = parent.join("child");
    fs::create_dir_all(&child).unwrap();
    fs::write(parent.join("ayran.toml"), "default_harness = 'codex'\n").unwrap();
    let user_path = child.join("ayran.toml");
    fs::write(&user_path, "default_harness = 'claude'\n").unwrap();

    let output = home
        .command()
        .current_dir(&child)
        .env("AYRAN_CONFIG", &user_path)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "codex --disable apps\n"
    );

    let xdg = parent.join("xdg");
    let xdg_config = xdg.join("ayran");
    let nested = xdg_config.join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(
        xdg_config.join("ayran.toml"),
        "default_harness = 'claude'\n",
    )
    .unwrap();
    fs::write(xdg.join("ayran.toml"), "default_harness = 'copilot'\n").unwrap();
    let output = home
        .command()
        .current_dir(&nested)
        .env("XDG_CONFIG_HOME", &xdg)
        .env_remove("AYRAN_CONFIG")
        .arg("--dry-run")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "env -u COPILOT_SKILLS_DIRS copilot\n"
    );
}

#[test]
fn project_aliases_and_home_are_user_level_only() {
    for (contents, key) in [
        ("[aliases.cr]\nharness = 'claude'\n", "aliases"),
        ("[harnesses.claude]\nhome = 'isolated'\n", "home"),
    ] {
        let home = TestHome::new();
        let path = home.dir.path().join("ayran.toml");
        fs::write(&path, contents).unwrap();
        let output = home.run(&["--claude", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("ayran: error[user-level-only]"), "{stderr}");
        assert!(stderr.contains(&path.display().to_string()), "{stderr}");
        assert!(stderr.contains(key), "{stderr}");
    }
}

#[test]
fn user_config_location_follows_override_xdg_then_home() {
    let home = TestHome::new();
    home.config("default_harness = 'claude'\n");
    let fallback = home.dir.path().join(".config/ayran/ayran.toml");
    fs::create_dir_all(fallback.parent().unwrap()).unwrap();
    fs::write(&fallback, "default_harness = 'copilot'\n").unwrap();
    let override_path = home.dir.path().join("custom.toml");
    fs::write(&override_path, "default_harness = 'codex'\n").unwrap();

    let xdg = home.run(&["--dry-run"]);
    assert_eq!(
        without_session_id(&String::from_utf8(xdg.stdout).unwrap()),
        "claude --settings '{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}'\n"
    );

    let override_output = home
        .command()
        .args(["--dry-run"])
        .env("AYRAN_CONFIG", &override_path)
        .output()
        .unwrap();
    assert_eq!(
        without_session_id(&String::from_utf8(override_output.stdout).unwrap()),
        "codex --disable apps\n"
    );

    let tilde_output = home
        .command()
        .args(["--dry-run"])
        .env("AYRAN_CONFIG", "~/custom.toml")
        .output()
        .unwrap();
    assert_eq!(
        without_session_id(&String::from_utf8(tilde_output.stdout).unwrap()),
        "codex --disable apps\n"
    );

    let fallback_output = home
        .command()
        .args(["--dry-run"])
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("AYRAN_CONFIG")
        .output()
        .unwrap();
    assert_eq!(
        without_session_id(&String::from_utf8(fallback_output.stdout).unwrap()),
        "env -u COPILOT_SKILLS_DIRS copilot\n"
    );
}

#[test]
fn invalid_user_config_reports_file_and_reason() {
    for (contents, reason) in [
        ("default_harness =", "TOML parse error"),
        ("unexpected = true", "unknown key"),
        ("default_harness = 42", "default_harness"),
        ("[harnesses.claude]\nmodel = 42", "model must be a string"),
        ("[harnesses.claude]\neffort = 'ultra'", "effort must be"),
        ("[harnesses.other]\nmodel = 'x'", "unknown Harness"),
        ("[skills.x]\ndefault = 42", "default must be a boolean"),
        ("[harnesses.claude]\nhome = 'invalid'", "home must be"),
    ] {
        let home = TestHome::new();
        home.config(contents);
        let output = home.run(&["--claude", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{contents}: {output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("ayran: error[config-invalid]"), "{stderr}");
        assert!(stderr.contains("config/ayran/ayran.toml"), "{stderr}");
        assert!(stderr.contains(reason), "{stderr}");
    }
}

#[test]
fn model_and_effort_reach_each_harness_in_native_form() {
    for (harness, expected) in [
        (
            "claude",
            "--model\nmodel with space\n--effort\nhigh\n--settings\n{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}\n--native\nvalue\n",
        ),
        (
            "codex",
            "-m\nmodel with space\n-c\nmodel_reasoning_effort=\"high\"\n--disable\napps\n--native\nvalue\n",
        ),
        (
            "copilot",
            "--model\nmodel with space\n--reasoning-effort\nhigh\n--native\nvalue\n",
        ),
    ] {
        let home = TestHome::new();
        home.fake_harness(harness, 0);
        let args = [
            format!("--{harness}"),
            "-m".into(),
            "model with space".into(),
            "-e".into(),
            "high".into(),
            "--".into(),
            "--native".into(),
            "value".into(),
        ];
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(home.record(), expected);
    }
}

#[test]
fn claude_effort_removes_parent_override_only_when_requested() {
    let home = TestHome::new();
    home.fake_harness("claude", 0);

    let with_effort = home.run_with_effort_env(&["--claude", "-e", "low"], Some("max"));
    assert_eq!(with_effort.status.code(), Some(0));
    assert_eq!(home.env_record(), "unset\n");

    let without_effort = home.run_with_effort_env(&["--claude"], Some("max"));
    assert_eq!(without_effort.status.code(), Some(0));
    assert_eq!(home.env_record(), "max\n");
    assert_eq!(
        home.record(),
        "--settings\n{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}\n"
    );
}

#[test]
fn dry_run_shows_native_args_removed_env_and_flag_sources() {
    let home = TestHome::new();
    let output = home.run(&[
        "--claude",
        "--model",
        "two words",
        "--effort",
        "high",
        "--dry-run",
        "--",
        "--native",
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "env -u CLAUDE_CODE_EFFORT_LEVEL claude --model 'two words' --effort high --settings '{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}' --native\n"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Model: two words (flag)"), "{stderr}");
    assert!(stderr.contains("Effort: high (flag)"), "{stderr}");
    assert!(home.no_record());
}

#[test]
fn dry_run_quotes_codex_and_copilot_native_args() {
    for (args, expected) in [
        (
            vec!["--codex", "-m", "o'malley", "-e", "xhigh", "--dry-run"],
            "codex -m 'o'\"'\"'malley' -c 'model_reasoning_effort=\"xhigh\"' --disable apps\n",
        ),
        (
            vec!["--copilot", "-m", "opus", "-e", "max", "--dry-run"],
            "env -u COPILOT_SKILLS_DIRS copilot --model opus --reasoning-effort max\n",
        ),
    ] {
        let home = TestHome::new();
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            without_session_id(&String::from_utf8(output.stdout).unwrap()),
            expected
        );
        assert!(home.no_record());
    }
}

#[test]
fn invalid_or_repeated_model_and_effort_are_usage_errors() {
    for (args, expected) in [
        (vec!["--claude", "-e", "ultra"], "low"),
        (
            vec!["--claude", "-m", "one", "--model", "two"],
            "at most once",
        ),
        (
            vec!["--claude", "-e", "low", "--effort", "high"],
            "at most once",
        ),
    ] {
        let output = TestHome::new().run(&args);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("ayran: error[usage]"), "{stderr}");
        assert!(stderr.contains(expected), "{stderr}");
    }
}

#[test]
fn launches_each_harness_and_passes_through_arguments_and_exit_code() {
    for (args, name) in [
        (vec!["--claude", "--", "--foo", "bar"], "claude"),
        (vec!["--codex", "--", "--foo", "bar"], "codex"),
        (vec!["--copilot", "--", "--foo", "bar"], "copilot"),
        (vec!["--harness", "codex", "--", "--foo", "bar"], "codex"),
    ] {
        let home = TestHome::new();
        home.fake_harness(name, 23);
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(23), "{output:?}");
        assert_eq!(
            home.record(),
            if name == "claude" {
                "--settings\n{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}\n--foo\nbar\n"
            } else if name == "codex" {
                "--disable\napps\n--foo\nbar\n"
            } else {
                "--foo\nbar\n"
            }
        );
    }
}

#[test]
fn dry_run_reports_harness_source_without_launching() {
    let home = TestHome::new();
    home.fake_harness("claude", 99);
    let output = home.run(&["--claude", "--dry-run", "--", "hello world", "it's good"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "claude --settings '{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}' 'hello world' 'it'\"'\"'s good'\n"
    );
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("Harness: claude (flag)")
    );
    assert!(home.no_record());
}

#[test]
fn errors_have_stable_codes_and_exit_codes() {
    for (args, code, message, status) in [
        (
            vec!["--claude", "--harness", "codex"],
            "usage",
            "conflicting",
            2,
        ),
        (vec!["--claude", "--claude"], "usage", "once", 2),
        (
            vec!["--harness", "claude", "--harness", "claude"],
            "usage",
            "once",
            2,
        ),
        (vec!["--harness", "unknown"], "usage", "claude", 2),
        (
            vec!["foo"],
            "usage",
            "unknown subcommand; Harness args go after --",
            2,
        ),
        (
            vec!["claude"],
            "usage",
            "unknown subcommand; Harness args go after --",
            2,
        ),
        (vec!["--claude"], "harness-not-found", "claude", 3),
        (vec![], "no-harness", "Harness", 3),
    ] {
        let home = TestHome::new();
        if code == "harness-not-found" {
            fs::remove_file(home.dir.path().join("bin/claude")).unwrap();
        }
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains(&format!("ayran: error[{code}]:")),
            "{stderr}"
        );
        assert!(stderr.contains(message), "{stderr}");
    }
}

#[test]
fn quiet_keeps_errors_and_help_and_version_work() {
    let home = TestHome::new();
    let error = home.run(&["-q"]);
    assert!(
        String::from_utf8(error.stderr)
            .unwrap()
            .contains("error[no-harness]")
    );
    for arg in ["--help", "--version"] {
        let output = home.run(&[arg]);
        assert_eq!(output.status.code(), Some(0));
        assert!(!output.stdout.is_empty());
    }
}

#[test]
fn harness_minimum_versions_gate_launch_and_dry_run() {
    for (name, minimum, below) in [
        ("claude", "2.1.283", "2.1.282"),
        ("codex", "0.158.0", "0.157.99"),
        ("copilot", "1.0.88", "1.0.87"),
    ] {
        for dry_run in [false, true] {
            let home = TestHome::new();
            let flag = format!("--{name}");
            let mut command = home.command();
            command.arg(&flag).env("HARNESS_VERSION", below);
            if dry_run {
                command.arg("--dry-run");
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(3), "{name}: {output:?}");
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(stderr.contains("error[harness-too-old]"), "{stderr}");
            assert!(
                stderr.contains(&format!("hint: found {below}; requires {minimum}")),
                "{stderr}"
            );
            assert!(output.stdout.is_empty());
            assert!(home.no_record());

            home.bump_harness_mtime(name);
            command.env("HARNESS_VERSION", minimum);
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(0), "{name}: {output:?}");
            if dry_run {
                assert!(
                    String::from_utf8(output.stderr)
                        .unwrap()
                        .contains(&format!("Harness version: {minimum}"))
                );
                assert!(home.no_record());
            } else {
                assert!(!home.no_record());
            }
        }
    }
}

#[test]
fn harness_version_output_accepts_native_labels_and_compares_numerically() {
    for (name, output, detected) in [
        ("claude", "2.1.283 (Claude Code)", "2.1.283"),
        ("codex", "codex-cli 0.158.0", "0.158.0"),
        ("copilot", "1.0.88\nCommit: fixture", "1.0.88"),
        ("claude", "2.1.1000 (Claude Code)", "2.1.1000"),
        ("claude", "3.0.0 (Claude Code)", "3.0.0"),
        ("codex", "codex-cli 0.200.0", "0.200.0"),
        ("copilot", "1.1.0", "1.1.0"),
    ] {
        let home = TestHome::new();
        let output = home
            .command()
            .args([&format!("--{name}"), "--dry-run"])
            .env("HARNESS_VERSION", output)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{name}: {output:?}");
        let trace = String::from_utf8(output.stderr).unwrap();
        assert!(
            trace.contains(&format!("Harness version: {detected}\n")),
            "{trace}"
        );
        assert!(home.no_record());
    }
}

#[test]
fn copilot_sentence_version_output_respects_the_minimum_and_launches() {
    for (version, expected_code) in [("1.0.90", 0), ("1.0.87", 3)] {
        for dry_run in [false, true] {
            let home = TestHome::new();
            let mut command = home.command();
            command.arg("--copilot").env(
                "HARNESS_VERSION",
                format!(
                    "GitHub Copilot CLI {version}.\nRun 'copilot update' to check for updates.\n"
                ),
            );
            if dry_run {
                command.arg("--dry-run");
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(expected_code), "{output:?}");
            let stderr = String::from_utf8(output.stderr).unwrap();
            if expected_code == 3 {
                assert!(stderr.contains("error[harness-too-old]"), "{stderr}");
                assert!(stderr.contains("found 1.0.87; requires 1.0.88"), "{stderr}");
                assert!(home.no_record());
            } else if dry_run {
                assert!(stderr.contains("Harness version: 1.0.90\n"), "{stderr}");
                assert!(home.no_record());
            } else {
                assert!(!home.no_record());
                let cached = home
                    .command()
                    .arg("--copilot")
                    .env("HARNESS_VERSION", "unknown")
                    .output()
                    .unwrap();
                assert_eq!(cached.status.code(), Some(0), "{cached:?}");
            }
        }
    }
}

#[test]
fn malformed_harness_versions_are_errors_without_starting_a_session() {
    for version in [
        "",
        "unknown",
        "2.1",
        "2.1.283.99",
        "2.1.283..",
        "2.1.283garbage",
        "-2.1.283",
        "2.1.283-preview",
        "2.1.18446744073709551616",
    ] {
        for dry_run in [false, true] {
            let home = TestHome::new();
            let mut command = home.command();
            command.arg("--claude").env("HARNESS_VERSION", version);
            if dry_run {
                command.arg("--dry-run");
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(3), "{version}: {output:?}");
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(stderr.contains("error[harness-version-failed]"), "{stderr}");
            assert!(stderr.contains("unparseable --version output"), "{stderr}");
            assert!(output.stdout.is_empty());
            assert!(home.no_record());
        }
    }
}

#[test]
fn failed_harness_version_probe_refuses_launch_even_with_supported_output() {
    let home = TestHome::new();
    for dry_run in [false, true] {
        let mut command = home.command();
        command.arg("--claude").env("VERSION_EXIT", "42");
        if dry_run {
            command.arg("--dry-run");
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("error[harness-version-failed]"), "{stderr}");
        assert!(
            stderr.contains("--version exited with exit status: 42"),
            "{stderr}"
        );
        assert!(output.stdout.is_empty());
        assert!(home.no_record());
    }
}

#[test]
fn missing_harness_refuses_dry_run() {
    let home = TestHome::new();
    fs::remove_file(home.dir.path().join("bin/codex")).unwrap();
    let output = home.run(&["--codex", "--dry-run"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[harness-not-found]")
    );
    assert!(output.stdout.is_empty());
    assert!(home.no_record());
}

#[test]
fn harness_version_cache_reuses_probe_and_invalidates_when_binary_changes() {
    let home = TestHome::new();
    let first = home.run(&["--claude"]);
    assert_eq!(first.status.code(), Some(0), "{first:?}");
    fs::remove_file(home.dir.path().join("record")).unwrap();

    let cached = home
        .command()
        .args(["--claude", "--dry-run"])
        .env("VERSION_EXIT", "42")
        .output()
        .unwrap();
    assert_eq!(cached.status.code(), Some(0), "{cached:?}");
    assert!(
        String::from_utf8(cached.stderr)
            .unwrap()
            .contains("Harness version: 9.0.0")
    );

    home.bump_harness_mtime("claude");
    let changed = home
        .command()
        .args(["--claude", "--dry-run"])
        .env("HARNESS_VERSION", "2.1.282")
        .output()
        .unwrap();
    assert_eq!(changed.status.code(), Some(3), "{changed:?}");
    assert!(
        String::from_utf8(changed.stderr)
            .unwrap()
            .contains("error[harness-too-old]")
    );
    assert!(home.no_record());
}

#[test]
fn harness_version_cache_invalidates_for_another_binary_with_the_same_mtime() {
    let home = TestHome::new();
    let initial = home.run(&["--claude"]);
    assert_eq!(initial.status.code(), Some(0), "{initial:?}");
    fs::remove_file(home.dir.path().join("record")).unwrap();
    let original = home.dir.path().join("bin/claude");
    let alternate_dir = home.dir.path().join("alternate-bin");
    fs::create_dir(&alternate_dir).unwrap();
    let alternate = alternate_dir.join("claude");
    fs::copy(&original, &alternate).unwrap();
    fs::File::open(&alternate)
        .unwrap()
        .set_times(
            fs::FileTimes::new().set_modified(original.metadata().unwrap().modified().unwrap()),
        )
        .unwrap();
    let output = home
        .command()
        .args(["--claude", "--dry-run"])
        .env("PATH", &alternate_dir)
        .env("HARNESS_VERSION", "2.1.282")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[harness-too-old]")
    );
    assert!(home.no_record());
}

#[test]
fn unusable_harness_version_cache_falls_back_to_probing() {
    let home = TestHome::new();
    fs::write(home.dir.path().join("cache"), "not a directory").unwrap();
    let output = home.run(&["--claude", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(home.no_record());

    fs::remove_file(home.dir.path().join("cache")).unwrap();
    let output = home.run(&["--claude"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    fs::remove_file(home.dir.path().join("record")).unwrap();
    fs::write(
        home.dir.path().join("cache/ayran/versions/claude.json"),
        "invalid JSON",
    )
    .unwrap();
    let output = home
        .command()
        .args(["--claude", "--dry-run"])
        .env("HARNESS_VERSION", "2.1.282")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[harness-too-old]")
    );
    assert!(home.no_record());
}

#[test]
fn dry_run_does_not_write_a_harness_version_cache() {
    let home = TestHome::new();
    let output = home.run(&["--claude", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(!home.dir.path().join("cache").exists());
    assert!(home.no_record());
}

#[test]
fn codex_dotted_plugin_ids_use_inline_tables_and_keep_other_overrides() {
    let home = TestHome::new();
    home.codex_config("[plugins.'a.b@m']\nenabled = false\n[plugins.'c.d@m']\nenabled = false\n");
    home.config("[plugins.first]\ncodex = 'a.b@m'\n[plugins.second]\ncodex = 'c.d@m'\n[plugins.same]\nall = 'a.b@m'\n");
    let output = home.run(&["--codex", "--plugin", "first,second,same", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "codex -c 'plugins={\"a.b@m\"={enabled=true}}' -c 'plugins={\"c.d@m\"={enabled=true}}' --disable apps\n"
    );
    assert!(home.no_record());
    let output = home.run(&["--codex", "--plugin", "first,second,same"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.record(),
        "-c\nplugins={\"a.b@m\"={enabled=true}}\n-c\nplugins={\"c.d@m\"={enabled=true}}\n--disable\napps\n"
    );
}

#[test]
fn codex_rejects_path_bindings_directly_and_through_all() {
    for field in ["codex", "all"] {
        for dry_run in [false, true] {
            let home = TestHome::new();
            home.config(&format!("[plugins.x]\n{field} = {{ path = 'missing' }}\n"));
            let mut args = vec!["--codex", "--plugin", "x"];
            if dry_run {
                args.push("--dry-run");
            }
            let output = home.run(&args);
            assert_eq!(output.status.code(), Some(3), "{output:?}");
            let error = String::from_utf8(output.stderr).unwrap();
            assert!(error.contains("error[unsupported-binding]"), "{error}");
            assert!(error.contains("x") && error.contains("codex"), "{error}");
            assert!(output.stdout.is_empty());
            assert!(home.no_record());
        }
    }
}

#[test]
fn copilot_native_plugins_load_from_the_real_home() {
    let home = TestHome::new();
    home.config("[plugins.msbuild]\nall = 'dotnet-msbuild@dotnet-agent-skills'\n[plugins.same]\ncopilot = 'dotnet-msbuild@dotnet-agent-skills'\n");
    let installed = home
        .dir
        .path()
        .join(".copilot/installed-plugins/dotnet-agent-skills/dotnet-msbuild");
    fs::create_dir_all(&installed).unwrap();
    let output = home
        .command()
        .args(["--copilot", "--plugin", "msbuild,same", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        format!(
            "env -u COPILOT_SKILLS_DIRS copilot --plugin-dir {}\n",
            installed.display()
        )
    );
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(
        trace.contains("Plugin msbuild: explicit (--plugin)"),
        "{trace}"
    );
    assert!(home.no_record());
    let output = home.run(&[
        "--copilot",
        "--plugin",
        "msbuild,same",
        "--",
        "--plugin-dir",
        "extra",
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        home.record(),
        format!(
            "--plugin-dir\n{}\n--plugin-dir\nextra\n",
            installed.display()
        )
    );
}

#[test]
fn copilot_path_bindings_load_absolute_paths_and_validate_existence() {
    let home = TestHome::new();
    let path = home.dir.path().join("config/ayran/working plugin");
    fs::create_dir_all(&path).unwrap();
    home.config("[plugins.working]\ncopilot = { path = 'working plugin' }\n");
    let output = home.run(&["--copilot", "--plugin", "working", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        format!(
            "env -u COPILOT_SKILLS_DIRS copilot --plugin-dir '{}'\n",
            path.display()
        )
    );
    assert!(home.no_record());
    let output = home.run(&["--copilot", "--plugin", "working"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(home.record(), format!("--plugin-dir\n{}\n", path.display()));
    fs::remove_dir(&path).unwrap();
    fs::remove_file(home.dir.path().join("record")).unwrap();
    let output = home.run(&["--copilot", "--plugin", "working", "--dry-run"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[path-not-found]")
    );
    assert!(home.no_record());
}

#[test]
fn copilot_native_bindings_cannot_escape_the_install_directory() {
    for id in [
        "direct-repo",
        "x@",
        "@m",
        "../x@m",
        "x@../m",
        "x@/m",
        "x@m/elsewhere",
        "x@m@other",
        "x@..",
        "..@m",
        "x@m\\bad",
        "x@C:\\bad",
    ] {
        let home = TestHome::new();
        home.config(&format!("[plugins.x]\ncopilot = '{id}'\n"));
        let output = home.run(&["--copilot", "--plugin", "x", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{id}: {output:?}");
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("error[unsupported-binding]"), "{error}");
        assert!(output.stdout.is_empty());
        assert!(home.no_record());
    }
}

#[test]
fn copilot_hides_unselected_installs_and_preserves_the_fixture_home() {
    let home = TestHome::new();
    home.config("[plugins.a]\ncopilot = 'a@m'\n");
    let copilot_home = home.dir.path().join(".copilot");
    let a = copilot_home.join("installed-plugins/m/a");
    let b = copilot_home.join("installed-plugins/m/b");
    for path in [&a, &b] {
        fs::create_dir_all(path).unwrap();
        fs::write(path.join("plugin.json"), "{}").unwrap();
    }
    let before = snapshot(&copilot_home);
    let output = home.run(&["--copilot", "--plugin", "a", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        format!(
            "env -u COPILOT_SKILLS_DIRS COPILOT_PLUGIN_DIR_ONLY=true copilot --plugin-dir {}\n",
            a.display()
        )
    );
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(
        trace.contains("Plugin b@m: hidden (unselected user install)"),
        "{trace}"
    );
    let output = home.run(&["--copilot", "--plugin", "a"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(home.record(), format!("--plugin-dir\n{}\n", a.display()));
    assert_eq!(
        fs::read_to_string(home.dir.path().join("plugin_env_record")).unwrap(),
        "true\n"
    );
    assert_eq!(snapshot(&copilot_home), before);
}

#[test]
fn copilot_direct_installs_require_paths_and_are_hidden_unless_selected() {
    let home = TestHome::new();
    let direct = home
        .dir
        .path()
        .join(".copilot/installed-plugins/_direct/source-id");
    fs::create_dir_all(&direct).unwrap();
    fs::write(direct.join("plugin.json"), "{}").unwrap();
    let output = home.run(&["--copilot", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "env -u COPILOT_SKILLS_DIRS COPILOT_PLUGIN_DIR_ONLY=true copilot\n"
    );
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(
        trace.contains("copilot-direct-plugin")
            && trace.contains("source-id")
            && trace.contains("hidden"),
        "{trace}"
    );
    home.config("[plugins.direct]\ncopilot = { path = '../../.copilot/installed-plugins/_direct/source-id' }\n");
    let output = home.run(&["--copilot", "--plugin", "direct", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains("COPILOT_PLUGIN_DIR_ONLY")
    );
    let output = home.run(&["--copilot", "-q", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        !String::from_utf8(output.stderr)
            .unwrap()
            .contains("note[copilot-direct-plugin]")
    );
    home.config("[plugins.direct]\ncopilot = 'source-id@_direct'\n");
    let output = home.run(&["--copilot", "--plugin", "direct", "--dry-run"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[unsupported-binding]")
    );
}

#[test]
fn copilot_empty_and_fully_selected_installs_do_not_set_hiding_env() {
    let home = TestHome::new();
    for create_empty in [false, true] {
        if create_empty {
            fs::create_dir_all(home.dir.path().join(".copilot/installed-plugins/m")).unwrap();
        }
        let output = home.run(&["--copilot", "--dry-run"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            without_session_id(&String::from_utf8(output.stdout).unwrap()),
            "env -u COPILOT_SKILLS_DIRS copilot\n"
        );
    }
    let path = home.dir.path().join(".copilot/installed-plugins/m/a");
    fs::create_dir_all(&path).unwrap();
    home.config("[plugins.a]\ncopilot = 'a@m'\n");
    let output = home.run(&["--copilot", "--plugin", "a"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        fs::read_to_string(home.dir.path().join("plugin_env_record")).unwrap(),
        "unset\n"
    );
    let output = home.run(&["--copilot", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "env -u COPILOT_SKILLS_DIRS COPILOT_PLUGIN_DIR_ONLY=true copilot\n"
    );
}

#[test]
fn copilot_native_bindings_must_name_installed_plugins() {
    let home = TestHome::new();
    home.config("[plugins.missing]\ncopilot = 'missing@m'\n");
    for dry_run in [false, true] {
        let mut args = vec!["--copilot", "--plugin", "missing"];
        if dry_run {
            args.push("--dry-run");
        }
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(
            error.contains("error[native-not-found]") && error.contains("missing@m"),
            "{error}"
        );
        assert!(home.no_record());
    }
}

#[test]
fn copilot_home_override_controls_enumeration_and_native_activation() {
    let home = TestHome::new();
    home.config("[plugins.a]\ncopilot = 'a@m'\n");
    let custom = home.dir.path().join("custom copilot");
    let a = custom.join("installed-plugins/m/a");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(custom.join("installed-plugins/m/b")).unwrap();
    fs::write(a.join("plugin.json"), "{}").unwrap();
    // Unreadable default-home state must not be consulted.
    fs::create_dir_all(home.dir.path().join(".copilot")).unwrap();
    fs::write(
        home.dir.path().join(".copilot/installed-plugins"),
        "invalid",
    )
    .unwrap();
    let before = snapshot(&custom);
    let output = home
        .command()
        .args(["--copilot", "--plugin", "a", "--dry-run"])
        .env("COPILOT_HOME", "custom copilot")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        format!(
            "env -u COPILOT_SKILLS_DIRS COPILOT_PLUGIN_DIR_ONLY=true copilot --plugin-dir '{}'\n",
            a.display()
        )
    );
    let output = home
        .command()
        .args(["--copilot", "--plugin", "a"])
        .env("COPILOT_HOME", &custom)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(home.record(), format!("--plugin-dir\n{}\n", a.display()));
    assert_eq!(snapshot(&custom), before);
}

#[test]
fn copilot_refuses_unreadable_install_directories() {
    for location in [
        "installed-plugins",
        "installed-plugins/m",
        "installed-plugins/m/a",
    ] {
        let home = TestHome::new();
        let path = home.dir.path().join(".copilot").join(location);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if location == "installed-plugins" {
            fs::write(&path, "not a directory").unwrap();
        } else {
            fs::create_dir_all(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
            // Privileged runners can still read chmod(000) directories.
            if fs::read_dir(&path).is_ok() {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
                continue;
            }
        }
        for dry_run in [false, true] {
            let mut args = vec!["--copilot"];
            if dry_run {
                args.push("--dry-run");
            }
            let output = home.run(&args);
            assert_eq!(output.status.code(), Some(3), "{location}: {output:?}");
            let error = String::from_utf8(output.stderr).unwrap();
            assert!(
                error.contains("error[enumeration-failed]") && error.contains("installed-plugins"),
                "{error}"
            );
            assert!(home.no_record());
        }
        if path.is_dir() {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
}

#[test]
fn copilot_direct_path_bindings_recognize_symlinks_to_selected_installs() {
    let home = TestHome::new();
    let direct = home
        .dir
        .path()
        .join(".copilot/installed-plugins/_direct/source-id");
    fs::create_dir_all(&direct).unwrap();
    let alias = home.dir.path().join("config/ayran/working-plugin");
    fs::create_dir_all(alias.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&direct, &alias).unwrap();
    home.config("[plugins.direct]\ncopilot = { path = 'working-plugin' }\n");
    let output = home.run(&["--copilot", "--plugin", "direct", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        format!(
            "env -u COPILOT_SKILLS_DIRS copilot --plugin-dir {}\n",
            alias.display()
        )
    );
    assert!(
        !String::from_utf8(output.stderr)
            .unwrap()
            .contains("hidden (unselected direct install)")
    );
}

#[test]
fn selected_profiles_expand_nested_members_once_and_report_their_origin() {
    let home = TestHome::new();
    home.config("[plugins.msbuild]\nclaude = 'msbuild@acme'\n[plugins.nuget]\nclaude = 'nuget@acme'\n[profiles.base]\nplugins = ['msbuild']\n[profiles.dotnet]\nplugins = ['nuget', 'msbuild']\nprofiles = ['base']\n[profiles.empty]\n");
    home.claude_installs(r#"{"version":2,"plugins":{"msbuild@acme":[{"scope":"user"}],"nuget@acme":[{"scope":"user"}]}}"#);
    let output = home.run(&["--claude", "--profile", "dotnet,empty", "--dry-run"]);
    let trace = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        trace.contains("Plugin nuget: via Profile dotnet"),
        "{trace}"
    );
    assert!(trace.contains("Plugin msbuild: via Profile"), "{trace}");
    assert_eq!(trace.matches("Plugin msbuild: via Profile").count(), 1);
    assert!(trace.contains(r#""msbuild@acme":true"#), "{trace}");
    assert!(trace.contains(r#""nuget@acme":true"#), "{trace}");
    assert!(home.no_record());
    assert!(home.run(&["--claude", "-p", "dotnet"]).status.success());
    assert!(home.record().contains(r#""nuget@acme":true"#));
}

#[test]
fn alias_profiles_activate_plugins_and_cli_profiles_add_to_them() {
    let home = TestHome::new();
    home.config("[plugins.msbuild]\ncodex = 'msbuild@acme'\n[plugins.nuget]\ncodex = 'nuget@acme'\n[profiles.dotnet]\nplugins = ['msbuild']\n[profiles.extra]\nplugins = ['nuget']\n[aliases.dn]\nharness = 'codex'\nprofiles = ['dotnet']\ndefaults = false\n");
    home.codex_config(
        "[plugins.'msbuild@acme']\nenabled = false\n[plugins.'nuget@acme']\nenabled = false\n",
    );
    let output = home.run(&["--alias", "dn"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(home.record().contains("plugins.msbuild@acme.enabled=true"));
    let output = home.run(&["--alias", "dn", "--profile", "extra", "--dry-run"]);
    let trace = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{trace}");
    assert!(
        trace.contains("Plugin msbuild: via Profile dotnet"),
        "{trace}"
    );
    assert!(trace.contains("Plugin nuget: via Profile extra"), "{trace}");
}

#[test]
fn alias_unknown_profiles_fail_at_launch_even_when_disabled() {
    let home = TestHome::new();
    home.config("[aliases.dn]\nharness = 'codex'\nprofiles = ['missing']\n");
    assert!(home.run(&["activate", "bash"]).status.success());
    for args in [
        vec!["--alias", "dn"],
        vec!["--alias", "dn", "--no-profile", "missing", "--dry-run"],
    ] {
        let output = home.run(&args);
        let trace = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(3), "{trace}");
        assert!(trace.contains("error[unknown-profile]"), "{trace}");
        assert!(trace.contains("missing"), "{trace}");
        assert!(home.no_record());
    }
}

#[test]
fn alias_profile_lists_require_string_arrays() {
    for value in ["'dotnet'", "[1]"] {
        let home = TestHome::new();
        home.config(&format!(
            "[aliases.dn]\nharness = 'codex'\nprofiles = {value}\n"
        ));
        let output = home.run(&["--alias", "dn", "--dry-run"]);
        let trace = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(3), "{trace}");
        assert!(
            trace.contains("aliases.dn.profiles must be an array of strings"),
            "{trace}"
        );
        assert!(home.no_record());
    }
}

#[test]
fn profiles_reject_invalid_fields_and_cycles_even_when_unselected() {
    for (config, reason) in [
        ("profiles = []", "profiles must be a table"),
        ("[profiles]\nx = false", "profiles.x must be a table"),
        ("[profiles.'x,y']", "invalid-name"),
        ("[profiles.x]\nskills = 42", "skills must be an array"),
        ("[profiles.x]\nmcp = [1]", "mcp must be an array of strings"),
        ("[profiles.x]\nextra = []", "unknown key profiles.x.extra"),
        (
            "[profiles.x]\nplugins = [1]",
            "plugins must be an array of strings",
        ),
        (
            "[profiles.x]\nprofiles = 'a'",
            "profiles must be an array of strings",
        ),
        (
            "[profiles.x]\ndescription = false",
            "description must be a string",
        ),
        (
            "[profiles.x]\ndefault = 'true'",
            "default must be a boolean",
        ),
        ("[profiles.a]\nprofiles = ['a']", "profile-cycle"),
        (
            "[profiles.a]\nprofiles = ['b']\n[profiles.b]\nprofiles = ['a']",
            "profile-cycle",
        ),
    ] {
        let home = TestHome::new();
        home.config(config);
        let output = home.run(&["--claude", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{config}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(reason),
            "{output:?}"
        );
        assert!(home.no_record());
    }
}

#[test]
fn unknown_profiles_fail_but_empty_profiles_are_silent() {
    let home = TestHome::new();
    home.config("[profiles.empty]\ndescription = 'Nothing'\n");
    let output = home.run(&[
        "--copilot",
        "--profile",
        "empty",
        "--profile",
        "empty",
        "--dry-run",
    ]);
    assert!(output.status.success(), "{output:?}");
    assert!(
        !String::from_utf8_lossy(&output.stderr)
            .lines()
            .any(|line| line.starts_with("ayran:")),
        "{output:?}"
    );
    let output = home.run(&["--copilot", "--profile", "nope"]);
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown-profile"));
    assert!(home.no_record());
}

#[test]
fn nearer_profile_replaces_all_farther_members_and_metadata() {
    let home = TestHome::new();
    home.config("[profiles.x]\nplugins = ['undefined']\nprofiles = ['undefined']\ndefault = true\ndescription = 'Farther'\n");
    fs::write(home.dir.path().join("ayran.toml"), "[profiles.x]\n").unwrap();
    let output = home.run(&["--copilot", "--profile", "x"]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn profile_false_bindings_are_skipped_but_missing_bindings_still_fail() {
    let home = TestHome::new();
    home.config("[plugins.absent]\nclaude = false\n[plugins.missing]\n[profiles.x]\nplugins = ['absent']\n[profiles.y]\nplugins = ['missing']\n");
    let output = home.run(&["--claude", "--profile", "x", "--dry-run"]);
    assert!(output.status.success(), "{output:?}");
    let trace = String::from_utf8_lossy(&output.stderr);
    assert!(trace.contains("note[binding-skipped]"), "{trace}");
    assert!(trace.contains("via Profile x"), "{trace}");
    let output = home.run(&["--claude", "--profile", "y"]);
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing-binding"));
    let output = home.run(&["--claude", "--profile", "x", "--plugin", "absent"]);
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("binding-absent"));
    assert!(home.no_record());
}

#[test]
fn default_profiles_reset_only_farther_definitions_not_their_members() {
    let home = TestHome::new();
    home.config("[plugins.far]\nall = 'far@m'\n[plugins.near]\nall = 'near@m'\n[plugins.local]\nall = 'local@m'\n[plugins.replaced]\nall = 'replaced@m'\n[profiles.far]\nplugins = ['far']\ndefault = true\n[profiles.replaced]\nplugins = ['far']\ndefault = true\n");
    fs::write(home.dir.path().join("ayran.toml"), "[disable]\ndefaults = true\n[profiles.near]\nplugins = ['near']\ndefault = true\n[profiles.replaced]\nplugins = ['replaced']\ndefault = true\n").unwrap();
    fs::write(
        home.dir.path().join("ayran.local.toml"),
        "[disable]\ndefaults = false\n[profiles.local]\nplugins = ['local']\ndefault = true\n",
    )
    .unwrap();
    home.claude_installs(r#"{"version":2,"plugins":{"far@m":[{"scope":"user"}],"near@m":[{"scope":"user"}],"local@m":[{"scope":"user"}],"replaced@m":[{"scope":"user"}]}}"#);
    let output = home.run(&["--claude", "--dry-run"]);
    assert!(output.status.success(), "{output:?}");
    let command = String::from_utf8(output.stdout).unwrap();
    for item in [
        r#""far@m":false"#,
        r#""near@m":true"#,
        r#""local@m":true"#,
        r#""replaced@m":true"#,
    ] {
        assert!(command.contains(item), "{command}");
    }
    let output = home.run(&["--claude", "--profile", "far", "--dry-run"]);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains(r#""far@m":true"#));
}

#[test]
fn default_profile_launches_and_defaults_controls_remove_it() {
    let home = TestHome::new();
    home.config("[plugins.review]\nall = 'review@acme'\n[profiles.team]\nplugins = ['review']\ndefault = true\n[aliases.clean]\nharness = 'claude'\ndefaults = false\n");
    home.claude_installs(r#"{"version":2,"plugins":{"review@acme":[{"scope":"user"}]}}"#);
    let output = home.run(&["--claude"]);
    assert!(output.status.success(), "{output:?}");
    assert!(home.record().contains(r#""review@acme":true"#));
    for (flags, enabled, origin) in [
        (vec!["--claude"], true, "Default (via Profile team)"),
        (vec!["--claude", "--no-defaults"], false, ""),
        (vec!["--alias", "clean"], false, ""),
        (
            vec!["--claude", "--plugin", "review"],
            true,
            "explicit (--plugin)",
        ),
        (
            vec!["--alias", "clean", "--profile", "team"],
            true,
            "via Profile team",
        ),
    ] {
        let mut args = flags;
        args.push("--dry-run");
        let output = home.run(&args);
        assert!(output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains(&format!(r#""review@acme":{enabled}"#)),
            "{output:?}"
        );
        if enabled {
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains(&format!("Plugin review: {origin} →")),
                "{output:?}"
            );
        }
    }
}

#[test]
fn cli_profile_disables_stop_nested_and_explicit_routes_but_preserve_other_routes() {
    let home = TestHome::new();
    home.config("[plugins.base]\nall = 'base@m'\n[plugins.team]\nall = 'team@m'\n[profiles.base]\nplugins = ['base']\n[profiles.team]\nplugins = ['team']\nprofiles = ['base']\n[profiles.other]\nplugins = ['base']\n");
    home.claude_installs(
        r#"{"version":2,"plugins":{"base@m":[{"scope":"user"}],"team@m":[{"scope":"user"}]}}"#,
    );
    for (selected, enabled) in [("team,base", false), ("team,other", true)] {
        let output = home.run(&[
            "--claude",
            "--profile",
            selected,
            "--no-profile",
            "base,other-empty",
            "--dry-run",
        ]);
        let trace = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{trace}");
        assert!(trace.contains(&format!(r#""base@m":{enabled}"#)), "{trace}");
        assert!(trace.contains(r#""team@m":true"#), "{trace}");
        assert!(
            trace.contains("Profile base: disabled by --no-profile"),
            "{trace}"
        );
    }
}

#[test]
fn config_and_alias_profile_disables_yield_to_explicit_profile_selection() {
    let home = TestHome::new();
    home.config("[plugins.review]\nall = 'review@m'\n[profiles.base]\nplugins = ['review']\ndefault = true\n[profiles.team]\nprofiles = ['base']\n[aliases.work]\nharness = 'claude'\n[aliases.work.disable]\nprofiles = ['base']\n");
    home.claude_installs(r#"{"version":2,"plugins":{"review@m":[{"scope":"user"}]}}"#);
    let path = home.dir.path().join("ayran.toml");
    for alias in [false, true] {
        fs::write(
            &path,
            if alias {
                ""
            } else {
                "[disable]\nprofiles = ['base']\n"
            },
        )
        .unwrap();
        for (selected, enabled) in [("team", false), ("team,base", true)] {
            let mut args = vec!["--profile", selected, "--dry-run"];
            args.extend(if alias {
                vec!["--alias", "work"]
            } else {
                vec!["--claude"]
            });
            let output = home.run(&args);
            let trace = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "{trace}");
            assert!(
                trace.contains(&format!(r#""review@m":{enabled}"#)),
                "{trace}"
            );
            if !enabled {
                let source = if alias {
                    "Alias work".to_owned()
                } else {
                    format!("layer {}", path.display())
                };
                assert!(
                    trace.contains(&format!("Profile base: disabled by {source}")),
                    "{trace}"
                );
            }
        }
    }
}

#[test]
fn profile_disables_union_across_layers_even_when_a_profile_is_redefined() {
    let home = TestHome::new();
    home.config("[disable]\nprofiles = ['far']\n[plugins.review]\nall = 'review@m'\n[profiles.far]\nplugins = ['review']\ndefault = true\n[profiles.near]\nplugins = ['review']\ndefault = true\n");
    fs::write(
        home.dir.path().join("ayran.toml"),
        "[disable]\nprofiles = ['near']\n[profiles.far]\nplugins = ['review']\ndefault = true\n",
    )
    .unwrap();
    home.claude_installs(r#"{"version":2,"plugins":{"review@m":[{"scope":"user"}]}}"#);
    let output = home.run(&["--claude", "--dry-run"]);
    let trace = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{trace}");
    assert!(trace.contains(r#""review@m":false"#), "{trace}");
    assert!(
        trace.contains(&format!(
            "Profile far: disabled by layer {}",
            home.dir.path().join("config/ayran/ayran.toml").display()
        )),
        "{trace}"
    );
    assert!(
        trace.contains(&format!(
            "Profile near: disabled by layer {}",
            home.dir.path().join("ayran.toml").display()
        )),
        "{trace}"
    );
}

#[test]
fn profile_disable_lists_require_string_arrays_in_config_and_aliases() {
    for config in [
        "[disable]\nprofiles = 'base'",
        "[disable]\nprofiles = [1]",
        "[aliases.work]\nharness = 'claude'\n[aliases.work.disable]\nprofiles = 'base'",
        "[aliases.work]\nharness = 'claude'\n[aliases.work.disable]\nprofiles = [1]",
    ] {
        let home = TestHome::new();
        home.config(config);
        let output = home.run(&["--claude", "--dry-run"]);
        let trace = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(3), "{trace}");
        assert!(
            trace.contains("profiles must be an array of strings"),
            "{trace}"
        );
    }
}

#[test]
fn disabling_a_dangling_nested_profile_prevents_its_expansion() {
    let home = TestHome::new();
    home.config("[profiles.team]\nprofiles = ['missing']\ndefault = true\n");
    for selection in [vec![], vec!["--profile", "team"]] {
        let mut args = vec!["--copilot", "--no-profile", "missing", "--dry-run"];
        args.extend(selection);
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let trace = String::from_utf8_lossy(&output.stderr);
        assert!(!trace.contains("dangling-ref"), "{trace}");
        assert!(
            trace.contains("Profile missing: disabled by --no-profile"),
            "{trace}"
        );
    }
}

#[test]
fn unselected_profiles_with_dangling_members_are_silent_at_launch() {
    let home = TestHome::new();
    home.config("[profiles.unused]\nplugins = ['missing']\nprofiles = ['missing']\n");
    let output = home.run(&["--copilot"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(!home.no_record());
}

#[test]
fn dangling_nested_profiles_follow_the_selection_origin() {
    let home = TestHome::new();
    home.config("[profiles.team]\nprofiles = ['base']\ndefault = true\n[profiles.base]\nprofiles = ['missing']\n[aliases.work]\nharness = 'copilot'\nprofiles = ['team']\n");
    for (args, status, severity) in [
        (vec!["--copilot", "--dry-run"], 0, "note"),
        (vec!["--copilot", "--profile", "team"], 3, "error"),
        (vec!["--alias", "work"], 3, "error"),
    ] {
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(status), "{output:?}");
        let trace = String::from_utf8_lossy(&output.stderr);
        assert!(
            trace.contains(&format!("{severity}[dangling-ref]")),
            "{trace}"
        );
        assert!(trace.contains("Profile base"), "{trace}");
        assert!(trace.contains("Profile missing"), "{trace}");
        assert!(home.no_record());
    }
}

#[test]
fn default_profiles_skip_dangling_plugins_and_launch_remaining_members() {
    let home = TestHome::new();
    home.config("[plugins.review]\nall = 'review@m'\n[profiles.team]\nplugins = ['missing', 'review']\ndefault = true\n");
    home.claude_installs(r#"{"version":2,"plugins":{"review@m":[{"scope":"user"}]}}"#);
    let output = home.run(&["--claude", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let trace = String::from_utf8_lossy(&output.stderr);
    assert!(trace.contains("note[dangling-ref]"), "{trace}");
    assert!(trace.contains("Profile team"), "{trace}");
    assert!(trace.contains("Plugin missing"), "{trace}");
    let output = home.run(&["--claude"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(home.record().contains(r#""review@m":true"#));
    assert!(String::from_utf8_lossy(&output.stderr).contains("note[dangling-ref]"));
}

#[test]
fn explicitly_selected_profiles_reject_dangling_plugin_members() {
    let home = TestHome::new();
    home.config("[profiles.team]\nplugins = ['missing']\n");
    let output = home.run(&["--copilot", "--profile", "team"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    assert!(diagnostics.contains("error[dangling-ref]"), "{diagnostics}");
    assert!(diagnostics.contains("Profile team"), "{diagnostics}");
    assert!(diagnostics.contains("Plugin missing"), "{diagnostics}");
    assert!(home.no_record());
}

#[test]
fn disabling_an_explicitly_selected_unknown_profile_does_not_hide_the_name_error() {
    let home = TestHome::new();
    let output = home.run(&[
        "--claude",
        "--profile",
        "missing",
        "--no-profile",
        "missing",
        "--dry-run",
    ]);
    let trace = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(3), "{trace}");
    assert!(trace.contains("unknown-profile"), "{trace}");
}

#[test]
fn any_launch_prunes_old_generated_dirs_across_harnesses_but_keeps_recent_state() {
    let home = TestHome::new();
    let now = std::time::SystemTime::now();
    for harness in ["claude", "codex", "copilot"] {
        for (name, age) in [("old", 31), ("recent", 29), ("future", -1)] {
            let directory = home.dir.path().join("cache/ayran").join(harness).join(name);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("payload"), "cached").unwrap();
            let modified = if age < 0 {
                now + std::time::Duration::from_secs(86400)
            } else {
                now - std::time::Duration::from_secs(age as u64 * 86400)
            };
            filetime::set_file_mtime(&directory, filetime::FileTime::from_system_time(modified))
                .unwrap();
        }
    }
    let other_state = home.dir.path().join("cache/ayran/versions/keep");
    fs::create_dir_all(&other_state).unwrap();
    filetime::set_file_mtime(&other_state, filetime::FileTime::from_unix_time(1, 0)).unwrap();
    let output = home.run(&["--codex"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    for harness in ["claude", "codex", "copilot"] {
        let root = home.dir.path().join("cache/ayran").join(harness);
        assert!(!root.join("old").exists());
        assert!(root.join("recent/payload").exists());
        assert!(root.join("future/payload").exists());
    }
    assert!(other_state.exists());
}

#[test]
fn dry_run_preserves_stale_generated_dirs() {
    let home = TestHome::new();
    let stale = home.dir.path().join("cache/ayran/claude/old");
    fs::create_dir_all(&stale).unwrap();
    fs::write(stale.join("payload"), "cached").unwrap();
    let old = filetime::FileTime::from_unix_time(1, 0);
    filetime::set_file_mtime(&stale, old).unwrap();
    for harness in ["--claude", "--codex", "--copilot"] {
        let output = home.run(&[harness, "--dry-run"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(fs::read_to_string(stale.join("payload")).unwrap(), "cached");
        assert_eq!(
            filetime::FileTime::from_last_modification_time(&fs::metadata(&stale).unwrap()),
            old
        );
    }
}

#[test]
fn cache_pruning_uses_the_home_fallback_for_missing_or_empty_xdg_cache_home() {
    let home = TestHome::new();
    let stale = home.dir.path().join(".cache/ayran/copilot/old");
    for xdg in [None, Some("")] {
        fs::create_dir_all(&stale).unwrap();
        filetime::set_file_mtime(&stale, filetime::FileTime::from_unix_time(1, 0)).unwrap();
        let mut command = home.command();
        command.arg("--claude");
        if let Some(value) = xdg {
            command.env("XDG_CACHE_HOME", value);
        } else {
            command.env_remove("XDG_CACHE_HOME");
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(!stale.exists());
    }
}

#[test]
fn cache_pruning_failures_are_silent_and_do_not_prevent_other_cleanup() {
    let home = TestHome::new();
    let baseline = home.run(&["--codex"]);
    assert_eq!(baseline.status.code(), Some(0), "{baseline:?}");
    let blocked = home.dir.path().join("cache/ayran/claude/old");
    let removable = home.dir.path().join("cache/ayran/copilot/old");
    for directory in [&blocked, &removable] {
        fs::create_dir_all(directory).unwrap();
        fs::write(directory.join("payload"), "cached").unwrap();
        filetime::set_file_mtime(directory, filetime::FileTime::from_unix_time(1, 0)).unwrap();
    }
    let parent = blocked.parent().unwrap();
    fs::set_permissions(parent, fs::Permissions::from_mode(0o555)).unwrap();
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o555)).unwrap();
    let output = home.run(&["--codex"]);
    // Restore permissions before assertions so the fixture can always be removed.
    fs::set_permissions(parent, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stderr, baseline.stderr, "{output:?}");
    assert!(blocked.join("payload").exists());
    assert!(!removable.exists());
}

#[test]
fn cache_pruning_does_not_follow_symlinks_or_delete_non_directory_state() {
    let home = TestHome::new();
    let root = home.dir.path().join("cache/ayran");
    let external = home.dir.path().join("external/old");
    fs::create_dir_all(&external).unwrap();
    fs::write(external.join("payload"), "keep").unwrap();
    filetime::set_file_mtime(&external, filetime::FileTime::from_unix_time(1, 0)).unwrap();
    fs::create_dir_all(root.join("claude")).unwrap();
    let link = root.join("claude/link");
    std::os::unix::fs::symlink(&external, &link).unwrap();
    std::os::unix::fs::symlink(home.dir.path().join("external"), root.join("copilot")).unwrap();
    let file = root.join("claude/file");
    fs::write(&file, "keep").unwrap();
    filetime::set_file_mtime(&file, filetime::FileTime::from_unix_time(1, 0)).unwrap();
    let output = home.run(&["--codex"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        fs::read_to_string(external.join("payload")).unwrap(),
        "keep"
    );
    assert!(fs::symlink_metadata(link).unwrap().is_symlink());
    assert_eq!(fs::read_to_string(file).unwrap(), "keep");
}

#[test]
fn skill_cache_is_reused_touched_and_independent_of_selection_order() {
    let home = TestHome::new();
    for name in ["one", "two"] {
        let path = home.dir.path().join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), "Instructions without frontmatter").unwrap();
    }
    fs::write(
        home.dir.path().join("ayran.toml"),
        "[skills.a]\nall = {path = 'one'}\n[skills.b]\nall = {path = 'two'}\n",
    )
    .unwrap();
    let output = home.run(&["--claude", "--skill", "a"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let first_record = home.record();
    let first = PathBuf::from(first_record.lines().nth(3).unwrap());
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1);
    fs::File::open(&first)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old))
        .unwrap();
    fs::write(first.join("marker"), "unchanged").unwrap();
    // Writing the marker touches the directory; reset its time again.
    fs::File::open(&first)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old))
        .unwrap();
    let output = home.run(&["--claude", "--skill", "a,a"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(home.record(), first_record);
    assert_eq!(
        fs::read_to_string(first.join("marker")).unwrap(),
        "unchanged"
    );
    assert!(fs::metadata(&first).unwrap().modified().unwrap() > old);
    let output = home.run(&["--claude", "--skill", "a,b"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let both = home.record();
    assert_ne!(both, first_record);
    let output = home.run(&["--claude", "--skill", "b", "--skill", "a"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(home.record(), both);
    let cache = PathBuf::from(both.lines().nth(3).unwrap());
    assert_eq!(
        fs::read_dir(cache.join(".claude/skills")).unwrap().count(),
        2
    );
    assert_eq!(fs::read_dir(first.parent().unwrap()).unwrap().count(), 2);
}

#[test]
fn skill_bindings_replace_as_whole_tables_and_specific_bindings_beat_all() {
    let home = TestHome::new();
    let path = home.dir.path().join("local-skill");
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("SKILL.md"), "---\nname: 'local-name'\n---\n").unwrap();
    home.config(
        "[skills.lint]\nall = { path = '~/missing' }\nclaude = false\ndescription = 'farther'\n",
    );
    fs::write(
        home.dir.path().join("ayran.toml"),
        "[skills.lint]\nall = { path = '~/local-skill' }\n",
    )
    .unwrap();
    let output = home.run(&["--claude", "--skill", "lint", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("→ local-name")
    );
    fs::write(
        home.dir.path().join("ayran.local.toml"),
        "[skills.lint]\nall = { path = 'missing' }\nclaude = 'native-lint'\n",
    )
    .unwrap();
    home.skill(".claude/skills/native-lint", "native-lint");
    let output = home.run(&["--claude", "--skill", "lint", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "claude --settings '{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}'\n"
    );
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("→ native-lint")
    );
    assert!(!home.dir.path().join("cache").exists());
}

#[test]
fn skill_errors_fail_before_launch_and_do_not_create_cache() {
    for (config, selection, code) in [
        ("", "unknown", "unknown-skill"),
        ("[skills.x]\ncodex = 'x'", "x", "missing-binding"),
        ("[skills.x]\nall = false", "x", "binding-absent"),
        (
            "[skills.x]\nall = {path = 'missing'}",
            "x",
            "path-not-found",
        ),
        ("[skills.x]\nall = {path = 'empty'}", "x", "path-not-found"),
        (
            "[skills.x]\nall = {path = 'one'}\n[skills.y]\nall = {path = 'two'}",
            "x,y",
            "skill-name-clash",
        ),
        (
            "[skills.x]\nall = {path = 'one'}\n[skills.y]\nall = {path = 'one'}",
            "x,y",
            "skill-name-clash",
        ),
        (
            "[skills.x]\nall = 'same'\n[skills.y]\nall = {path = 'one'}",
            "x,y",
            "skill-name-clash",
        ),
    ] {
        let home = TestHome::new();
        home.skill(".claude/skills/same", "same");
        for directory in ["one", "two", "empty"] {
            let path = home.dir.path().join(directory);
            fs::create_dir_all(&path).unwrap();
            if directory != "empty" {
                fs::write(path.join("SKILL.md"), "---\nname: same\n---\n").unwrap();
            }
        }
        fs::write(home.dir.path().join("ayran.toml"), config).unwrap();
        for dry in [false, true] {
            let mut args = vec!["--claude", "--skill", selection];
            if dry {
                args.push("--dry-run");
            }
            let output = home.run(&args);
            assert_eq!(output.status.code(), Some(3), "{config}: {output:?}");
            assert!(
                String::from_utf8(output.stderr)
                    .unwrap()
                    .contains(&format!("error[{code}]"))
            );
            assert!(home.no_record());
            assert!(!home.dir.path().join("cache").exists());
        }
    }
}

#[test]
fn skill_schema_rejects_plugin_items_and_future_fields_at_load_time() {
    for config in [
        "[skills.x]\nall = 'pa:tdd'",
        "[skills.x]\ncodex = 'anthropic-skills:x'",
        "[skills.x]\nall = 'x'\ndefault = 42",
        "[disable]\nskills = 42",
        "[aliases.x]\nharness = 'claude'\nskills = 42",
        "[aliases.x]\nharness = 'claude'\n[aliases.x.disable]\nskills = 42",
        "[profiles.x]\nskills = 42",
        "[skills.x]\nall = true",
        "[skills.x]\nall = {path = 'x', extra = true}",
        "[skills.x]\ndescription = 42",
    ] {
        let home = TestHome::new();
        home.config(config);
        let output = home.run(&["--claude", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{config}: {output:?}");
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("error[config-invalid]")
        );
    }
}

#[test]
fn native_skill_aliases_deduplicate_on_claude_and_copilot() {
    let home = TestHome::new();
    home.config("[skills.x]\nall = 'tdd'\n[skills.y]\nall = 'tdd'\n");
    home.skill(".claude/skills/tdd", "tdd");
    let output = home.run(&["--claude", "--skill", "x,y,x", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "claude --settings '{\"disableClaudeAiConnectors\":true,\"syncClaudeAiSkills\":false}'\n"
    );
    let trace = String::from_utf8(output.stderr).unwrap();
    assert_eq!(trace.matches("Skill x:").count(), 1);
    assert_eq!(trace.matches("Skill y:").count(), 1);
    home.skill(".copilot/skills/tdd", "tdd");
    let output = home.run(&["--copilot", "--skill", "x,y,x", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        without_session_id(&String::from_utf8(output.stdout).unwrap()),
        "env -u COPILOT_SKILLS_DIRS copilot\n"
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("warning[leak]"));
}

#[test]
fn concurrent_skill_launches_share_one_complete_cache_directory() {
    let home = TestHome::new();
    let path = home.dir.path().join("lint");
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("SKILL.md"), "---\nname: lint\n---\n").unwrap();
    fs::write(
        home.dir.path().join("ayran.toml"),
        "[skills.lint]\nclaude = {path = 'lint'}\n",
    )
    .unwrap();
    let mut launches = Vec::new();
    for index in 0..6 {
        let mut command = home.command();
        command
            .args(["--claude", "--skill", "lint"])
            .env("RECORD", home.dir.path().join(format!("record-{index}")));
        launches.push(command.spawn().unwrap());
    }
    for mut launch in launches {
        assert!(launch.wait().unwrap().success());
    }
    let records: Vec<_> = (0..6)
        .map(|index| {
            without_session_id(
                &fs::read_to_string(home.dir.path().join(format!("record-{index}"))).unwrap(),
            )
        })
        .collect();
    assert!(records.iter().all(|record| record == &records[0]));
    let directory = Path::new(records[0].lines().nth(3).unwrap());
    assert_eq!(
        fs::read_link(directory.join(".claude/skills/lint")).unwrap(),
        path
    );
    assert_eq!(
        fs::read_dir(directory.parent().unwrap()).unwrap().count(),
        1
    );
}

#[test]
fn path_skill_frontmatter_is_yaml_and_cannot_escape_generated_directory() {
    for (contents, success) in [
        (
            "---\nname: \"lint\" # a YAML comment\ndescription: |\n  Lint the project\n---\n",
            true,
        ),
        ("---\ndescription: No name\n---\n", true),
        ("---\nname: ../escape\n---\n", false),
        ("---\nname: /absolute\n---\n", false),
        ("---\nname: ''\n---\n", false),
        ("---\nname: 123\n---\n", false),
        ("---\nname: [\n---\n", false),
        ("---\nname: lint\n", false),
    ] {
        let home = TestHome::new();
        let path = home.dir.path().join("lint");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), contents).unwrap();
        fs::write(
            home.dir.path().join("ayran.toml"),
            "[skills.lint]\nall = {path = 'lint'}\n",
        )
        .unwrap();
        let output = home.run(&["--claude", "--skill", "lint", "--dry-run"]);
        assert_eq!(
            output.status.code(),
            Some(if success { 0 } else { 3 }),
            "{contents}: {output:?}"
        );
        if success {
            assert!(String::from_utf8(output.stderr).unwrap().contains("→ lint"));
        }
        assert!(!home.dir.path().join("cache").exists());
    }
}

#[test]
fn skill_cache_falls_back_to_home_and_hashes_the_absolute_target() {
    let home = TestHome::new();
    for name in ["one", "two"] {
        let path = home.dir.path().join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), "---\nname: lint\n---\n").unwrap();
    }
    fs::write(
        home.dir.path().join("ayran.toml"),
        "[skills.one]\nall = {path = 'one'}\n[skills.two]\nall = {path = 'two'}\n",
    )
    .unwrap();
    let mut directories = Vec::new();
    for selection in ["one", "two"] {
        let output = home
            .command()
            .args(["--claude", "--skill", selection])
            .env_remove("XDG_CACHE_HOME")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let record = home.record();
        let directory = PathBuf::from(record.lines().nth(3).unwrap());
        assert!(directory.starts_with(home.dir.path().join(".cache/ayran/claude")));
        assert_eq!(
            fs::read_link(directory.join(".claude/skills/lint")).unwrap(),
            home.dir.path().join(selection)
        );
        directories.push(directory);
    }
    assert_ne!(directories[0], directories[1]);
}

#[test]
fn copilot_path_skills_load_in_a_synthetic_plugin_alongside_selected_plugins() {
    let home = TestHome::new();
    home.skill("lint-source", "native-lint");
    home.config("[skills.lint]\nall = {path = '~/lint-source'}\n[plugins.review]\nall = {path = '~/review'}\n");
    fs::create_dir_all(home.dir.path().join("review")).unwrap();
    let dry = home.run(&[
        "--copilot",
        "--skill",
        "lint",
        "--plugin",
        "review",
        "--dry-run",
    ]);
    assert_eq!(dry.status.code(), Some(0), "{dry:?}");
    assert!(!home.dir.path().join("cache/ayran/copilot").exists());
    let output = home
        .command()
        .env("COPILOT_PLUGIN_DIR_ONLY", "true")
        .args(["--copilot", "--skill", "lint", "--plugin", "review"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let record = home.record();
    let args: Vec<_> = record.lines().collect();
    let dirs: Vec<_> = args
        .windows(2)
        .filter(|pair| pair[0] == "--plugin-dir")
        .map(|pair| PathBuf::from(pair[1]))
        .collect();
    assert_eq!(dirs.len(), 2, "{record}");
    assert_eq!(dirs[0], home.dir.path().join("review"));
    assert_eq!(dirs[1].file_name().unwrap(), "ayran");
    assert_eq!(
        fs::read_link(dirs[1].join("skills/native-lint")).unwrap(),
        home.dir.path().join("lint-source")
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(dirs[1].join("plugin.json")).unwrap()).unwrap();
    assert_eq!(manifest["name"], "ayran");
    assert_eq!(
        fs::read_to_string(home.dir.path().join("plugin_env_record")).unwrap(),
        "true\n"
    );
    let generated = dirs[1].parent().unwrap();
    let contents_before = snapshot(&dirs[1]);
    filetime::set_file_mtime(generated, filetime::FileTime::from_unix_time(1, 0)).unwrap();
    let unselected = home
        .dir
        .path()
        .join(".copilot/installed-plugins/m/unselected");
    fs::create_dir_all(&unselected).unwrap();
    fs::write(unselected.join("plugin.json"), "{}").unwrap();
    let harness_before = snapshot(&home.dir.path().join(".copilot"));
    let output = home.run(&["--copilot", "--skill", "lint,lint", "--plugin", "review"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(home.record(), record);
    assert_eq!(snapshot(&dirs[1]), contents_before);
    assert_eq!(snapshot(&home.dir.path().join(".copilot")), harness_before);
    assert!(
        fs::metadata(generated).unwrap().modified().unwrap()
            > std::time::UNIX_EPOCH + std::time::Duration::from_secs(1)
    );
    assert_eq!(
        fs::read_dir(generated.parent().unwrap()).unwrap().count(),
        1
    );
    assert_eq!(
        fs::read_to_string(home.dir.path().join("plugin_env_record")).unwrap(),
        "true\n"
    );
}

#[test]
fn copilot_native_skills_are_discovered_and_unselected_personal_skills_leak() {
    let home = TestHome::new();
    home.skill(".copilot/skills/old-dir", "selected");
    home.skill(".agents/skills/leaked", "leaked");
    home.skill(".github/skills/project", "project");
    home.config("[skills.x]\nall = 'selected'\n[skills.y]\nall = 'project'\n");
    let output = home.run(&["--copilot", "--skill", "x,y"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        diagnostics.matches("warning[leak]").count(),
        1,
        "{diagnostics}"
    );
    assert!(
        diagnostics.contains("copilot-personal-skill") && diagnostics.contains("leaked"),
        "{diagnostics}"
    );
    assert!(!diagnostics.contains(": selected"), "{diagnostics}");
    assert_eq!(home.record(), "");
    let quiet = home.run(&["--copilot", "--quiet", "--skill", "x,y"]);
    assert!(
        quiet.status.success() && quiet.stderr.is_empty(),
        "{quiet:?}"
    );
}

#[test]
fn copilot_disabled_native_and_shadowed_path_skills_produce_quietable_notes() {
    let home = TestHome::new();
    home.skill(".copilot/skills/tdd", "tdd");
    home.skill(".github/skills/lint", "lint");
    home.skill("lint-source", "lint");
    fs::write(
        home.dir.path().join(".copilot/settings.json"),
        "{ // fixture\n \"disabledSkills\": [\"tdd\",],\n}",
    )
    .unwrap();
    home.config("[skills.tdd]\nall = 'tdd'\n[skills.lint]\nall = {path = '~/lint-source'}\n");
    let output = home.run(&["--copilot", "--skill", "tdd,lint"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let notes = String::from_utf8_lossy(&output.stderr);
    assert!(
        notes.contains("note[skill-disabled]") && notes.contains("copilot skill enable tdd"),
        "{notes}"
    );
    assert!(
        notes.contains("note[skill-shadowed]") && notes.contains("lint"),
        "{notes}"
    );
    assert!(!notes.contains("warning[leak]"), "{notes}");
    let output = home.run(&["--copilot", "--skill", "tdd,lint", "--quiet"]);
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{output:?}"
    );
}

#[test]
fn copilot_skill_state_errors_refuse_launch_without_writing_harness_state() {
    for (relative, contents) in [
        (".copilot/settings.json", "{broken"),
        (".copilot/settings.json", "{\"disabledSkills\": false}"),
        (".copilot/settings.json", "{\"disabledSkills\": [3]}"),
        (".copilot/settings.json", "[]"),
        (".copilot/settings.json", r#"{"skillDirectories": false}"#),
        (".copilot/settings.json", r#"{"skillDirectories": [3]}"#),
        (".copilot/settings.json", r#"{"skillDirectories": null}"#),
        (".copilot/skills/broken/SKILL.md", "---\nname: [bad]\n---"),
        (
            ".agents/skills/broken/SKILL.md",
            "---\nname: ../escape\n---",
        ),
    ] {
        let home = TestHome::new();
        let path = home.dir.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        let output = home.run(&["--copilot"]);
        assert_eq!(output.status.code(), Some(3), "{relative}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("enumeration-failed"),
            "{output:?}"
        );
        assert!(home.no_record());
        assert_eq!(fs::read_to_string(path).unwrap(), contents);
    }
    let home = TestHome::new();
    fs::write(home.dir.path().join(".agents"), "not a directory").unwrap();
    let output = home.run(&["--copilot"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("enumeration-failed"));
    assert!(home.no_record());
}

#[test]
fn copilot_native_skill_validation_and_personal_leaks_respect_home_override() {
    let home = TestHome::new();
    home.skill("custom/skills/tdd", "tdd");
    home.skill(".copilot/skills/ignored", "ignored");
    home.skill(".agents/skills/shared", "shared");
    fs::write(
        home.dir.path().join("custom/settings.json"),
        r#"{"disabledSkills":["tdd"]}"#,
    )
    .unwrap();
    home.config("[skills.tdd]\nall = 'tdd'\n[skills.missing]\nall = 'missing'\n");
    for custom in [PathBuf::from("custom"), home.dir.path().join("custom")] {
        let output = home
            .command()
            .env("COPILOT_HOME", custom)
            .args(["--copilot", "--skill", "tdd"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let diagnostics = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostics.contains("skill-disabled") && diagnostics.contains("shared"),
            "{diagnostics}"
        );
        assert!(!diagnostics.contains("ignored"), "{diagnostics}");
    }
    let output = home.run(&["--copilot", "--skill", "missing", "--quiet"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("native-not-found"));
}

#[test]
fn copilot_project_skills_are_discovered_up_to_git_root_and_home_roots_stay_personal() {
    let home = TestHome::new();
    let repo = home.dir.path().join("repo");
    let nested = repo.join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(repo.join(".git").join("HEAD"), "ref: refs/heads/main").unwrap();
    fs::create_dir_all(home.dir.path().join(".git")).unwrap();
    fs::write(
        home.dir.path().join(".git").join("HEAD"),
        "ref: refs/heads/main",
    )
    .unwrap();
    home.skill("repo/.github/skills/github", "github");
    home.skill("repo/.agents/skills/agents", "agents");
    home.skill("repo/.claude/skills/claude", "claude");
    home.skill(".github/skills/outside", "outside");
    home.skill(".agents/skills/personal", "personal");
    home.config("[skills.a]\nall = 'github'\n[skills.b]\nall = 'agents'\n[skills.c]\nall = 'claude'\n[skills.outside]\nall = 'outside'\n");
    let output = home
        .command()
        .current_dir(&nested)
        .args(["--copilot", "--skill", "a,b,c"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostics.contains("personal") && !diagnostics.contains("github"),
        "{diagnostics}"
    );
    let output = home
        .command()
        .current_dir(&nested)
        .args(["--copilot", "--skill", "outside"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let output = home.run(&["--copilot"]);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("personal"),
        "{output:?}"
    );
}

#[test]
fn skill_default_switches_on_without_explicit_selection() {
    let home = TestHome::new();
    home.skill(".claude/skills/tdd", "tdd");
    home.config("[skills.tdd]\nall = 'tdd'\ndefault = true\n");
    let output = home.run(&["--claude", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(trace.contains("Skill tdd: Default → tdd"), "{trace}");
}

#[test]
fn skill_alias_and_cli_lists_add_and_cli_disables_win() {
    let home = TestHome::new();
    for name in ["a", "b", "c"] {
        home.skill(&format!(".claude/skills/{name}"), name);
    }
    home.config("[skills.a]\nall = 'a'\n[skills.b]\nall = 'b'\ndefault = true\n[skills.c]\nall = 'c'\n[disable]\nskills = ['a']\n[aliases.work]\nharness = 'claude'\nskills = ['a']\ndisable = { skills = ['b'] }\n");
    let output = home.run(&[
        "--alias",
        "work",
        "--skill",
        "c,a",
        "--no-skill",
        "c",
        "--no-skill",
        "a",
        "--dry-run",
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let command = String::from_utf8(output.stdout).unwrap();
    for name in ["a", "b", "c"] {
        assert!(
            command.contains(&format!("\"{name}\":\"off\"")),
            "{command}"
        );
    }
    let output = home.run(&["--alias", "work", "--skill", "c", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let command = String::from_utf8(output.stdout).unwrap();
    assert!(!command.contains(r#""a":"on""#), "{command}");
    assert!(!command.contains(r#""c":"on""#), "{command}");
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(trace.contains("Skill a: selected (natively on"), "{trace}");
    assert!(trace.contains("Skill c: selected (natively on"), "{trace}");
    assert!(trace.contains("Skill a: explicit (Alias work)"), "{trace}");
    assert!(trace.contains("Skill b: disabled by Alias work"), "{trace}");
}

#[test]
fn skill_defaults_reset_only_farther_layers_and_disables_union() {
    let home = TestHome::new();
    for name in ["far", "redefined", "near", "local"] {
        home.skill(&format!(".claude/skills/{name}"), name);
    }
    home.config("[skills.far]\nall = 'far'\ndefault = true\n[skills.redefined]\nall = 'redefined'\ndefault = true\n[disable]\nskills = ['near']\n[aliases.work]\nharness = 'claude'\ndefaults = false\n");
    fs::write(home.dir.path().join("ayran.toml"), "[disable]\ndefaults = true\nskills = ['local']\n[skills.redefined]\nall = 'redefined'\ndefault = true\n[skills.near]\nall = 'near'\ndefault = true\n").unwrap();
    fs::write(
        home.dir.path().join("ayran.local.toml"),
        "[skills.local]\nall = 'local'\ndefault = true\n",
    )
    .unwrap();
    let output = home.run(&["--claude", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let command = String::from_utf8(output.stdout).unwrap();
    assert!(!command.contains(r#""redefined":"on""#), "{command}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Skill redefined: selected (natively on")
    );
    for name in ["far", "near", "local"] {
        assert!(
            command.contains(&format!("\"{name}\":\"off\"")),
            "{command}"
        );
    }
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(trace.contains("(Defaults)"), "{trace}");
    for flags in [vec!["--claude", "--no-defaults"], vec!["--alias", "work"]] {
        let mut args = flags;
        args.push("--dry-run");
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let command = String::from_utf8(output.stdout).unwrap();
        assert!(command.contains(r#""redefined":"off""#), "{command}");
    }
}

#[test]
fn disabled_path_skills_are_not_read_or_materialized() {
    for route in ["cli", "config", "alias", "defaults"] {
        let home = TestHome::new();
        let mut config =
            "[skills.missing]\nall = { path = 'missing' }\ndefault = true\n".to_owned();
        let mut args = vec!["--claude", "--dry-run"];
        match route {
            "cli" => args.extend(["--skill", "missing", "--no-skill", "missing"]),
            "config" => config.push_str("[disable]\nskills = ['missing']\n"),
            "alias" => {
                config.push_str(
                    "[aliases.work]\nharness = 'claude'\ndisable = { skills = ['missing'] }\n",
                );
                args.extend(["--alias", "work"]);
            }
            "defaults" => args.push("--no-defaults"),
            _ => unreachable!(),
        }
        home.config(&config);
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(0), "{route}: {output:?}");
        assert!(!home.dir.path().join("cache").exists());
        assert!(
            !String::from_utf8(output.stdout)
                .unwrap()
                .contains("--add-dir")
        );
    }
}

#[test]
fn default_native_skills_activate_on_all_harnesses() {
    for harness in ["--claude", "--codex", "--copilot"] {
        let home = TestHome::new();
        home.skill(".claude/skills/tdd", "tdd");
        fs::write(
            home.dir.path().join(".claude/settings.json"),
            r#"{"skillOverrides":{"tdd":"off"}}"#,
        )
        .unwrap();
        home.skill(".agents/skills/tdd", "tdd");
        home.codex_config("[[skills.config]]\nname = 'tdd'\nenabled = false\n");
        home.config("[skills.tdd]\nall = 'tdd'\ndefault = true\n");
        let output = home.run(&[harness]);
        assert_eq!(output.status.code(), Some(0), "{harness}: {output:?}");
        match harness {
            "--claude" => assert!(home.record().contains(r#""tdd":"on""#)),
            "--codex" => assert_eq!(
                home.codex_skill_rules()
                    .get(&home.dir.path().join(".agents/skills/tdd/SKILL.md")),
                Some(&true)
            ),
            "--copilot" => assert!(
                !String::from_utf8(output.stderr)
                    .unwrap()
                    .contains("copilot-personal-skill")
            ),
            _ => unreachable!(),
        }
    }
}

#[test]
fn codex_mcp_stdio_passes_one_inline_override_with_environment_names() {
    let home = TestHome::new();
    home.config("[mcp.files]\nall = { command = './bin/files', args = ['--label', 'a \"quoted\" \\ value'], env = { MODE = 'read' }, env_vars = ['TOKEN'] }\n");
    let executable = home.dir.path().join("config/ayran/bin/files");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(&executable, "fixture").unwrap();
    let output = home
        .command()
        .env("TOKEN", "secret-must-not-appear")
        .args([
            "--codex",
            "--mcp",
            "files,files",
            "--",
            "-c",
            "mcp_servers.files.enabled=false",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let record = home.record();
    assert!(!record.contains("secret-must-not-appear"));
    let args: Vec<_> = record.lines().collect();
    let overrides: Vec<_> = args
        .iter()
        .enumerate()
        .filter(|(_, arg)| {
            arg.starts_with("mcp_servers.files={") || arg.starts_with("mcp_servers.files = {")
        })
        .collect();
    assert_eq!(overrides.len(), 1, "{record}");
    let (index, value) = overrides[0];
    assert_eq!(args[index - 1], "-c");
    let config: toml::Table = value.parse().unwrap();
    let expected: toml::Table = format!("[mcp_servers.files]\ncommand = '{}'\nargs = ['--label', 'a \"quoted\" \\ value']\nenv = {{ MODE = 'read' }}\nenv_vars = ['TOKEN']\n", executable.display()).parse().unwrap();
    assert_eq!(config, expected);
    assert!(index < args.len() - 2, "passthrough must come last");
    assert_eq!(
        &args[args.len() - 2..],
        &["-c", "mcp_servers.files.enabled=false"]
    );
    assert!(!home.dir.path().join("cache/ayran/codex").exists());
}

#[test]
fn codex_mcp_http_uses_native_secret_fields_and_dry_run_writes_nothing() {
    let home = TestHome::new();
    home.config("[mcp.linear]\nall = { url = 'https://example.test/mcp', headers = { Accept = 'application/json' }, env_headers = { 'X-Api-Key' = 'API_KEY' }, bearer_token_env = 'TOKEN' }\n[mcp.public]\ncodex = { url = 'https://public.test/mcp' }\n");
    let output = home
        .command()
        .env("TOKEN", "secret-must-not-appear")
        .env("API_KEY", "another-secret")
        .args(["--codex", "--mcp", "linear,public", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains("mcp_servers.linear={"), "{stdout}");
    assert!(stdout.contains("env_http_headers"), "{stdout}");
    assert!(!stdout.contains("secret-must-not-appear"));
    assert!(!stdout.contains("another-secret"));
    assert!(!stderr.contains("MCP config:"));
    assert!(!home.dir.path().join("cache").exists());
    assert!(home.no_record());
    let output = home.run(&["--codex", "--mcp", "linear,public"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let record = home.record();
    let overrides: Vec<_> = record
        .lines()
        .filter(|arg| arg.starts_with("mcp_servers."))
        .collect();
    assert_eq!(overrides.len(), 2);
    let linear: toml::Table = overrides[0].parse().unwrap();
    let expected: toml::Table = "[mcp_servers.linear]\nurl = 'https://example.test/mcp'\nhttp_headers = { Accept = 'application/json' }\nenv_http_headers = { 'X-Api-Key' = 'API_KEY' }\nbearer_token_env_var = 'TOKEN'\n".parse().unwrap();
    assert_eq!(linear, expected);
    let public: toml::Table = overrides[1].parse().unwrap();
    let expected: toml::Table = "[mcp_servers.public]\nurl = 'https://public.test/mcp'\nhttp_headers = {}\nenv_http_headers = {}\n".parse().unwrap();
    assert_eq!(public, expected);
}

#[test]
fn copilot_mcp_stdio_uses_a_cached_config_with_all_tools_and_secret_references() {
    let home = TestHome::new();
    home.config("[mcp.files]\nall = { command = './bin/files', args = ['--root', '.'], env = { MODE = 'read' }, env_vars = ['TOKEN'] }\n");
    let executable = home.dir.path().join("config/ayran/bin/files");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(&executable, "fixture").unwrap();
    let output = home
        .command()
        .env("TOKEN", "secret-must-not-appear")
        .args(["--copilot", "--mcp", "files", "--", "--prompt", "fixture"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let record = home.record();
    let args: Vec<_> = record.lines().collect();
    let index = args
        .iter()
        .position(|arg| *arg == "--additional-mcp-config")
        .unwrap();
    let path = Path::new(
        args[index + 1]
            .strip_prefix('@')
            .expect("Copilot file reference needs @"),
    );
    assert!(path.starts_with(home.dir.path().join("cache/ayran/copilot")));
    assert_eq!(&args[args.len() - 2..], &["--prompt", "fixture"]);
    let contents = fs::read_to_string(path).unwrap();
    assert!(!contents.contains("secret-must-not-appear"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&contents).unwrap(),
        serde_json::json!({
            "mcpServers": { "files": { "type": "local", "command": executable, "args": ["--root", "."], "env": { "MODE": "read", "TOKEN": "${TOKEN}" }, "tools": ["*"] } }
        })
    );
    let again = home.run(&["--copilot", "--mcp", "files"]);
    assert_eq!(again.status.code(), Some(0), "{again:?}");
    assert!(home.record().contains(path.to_str().unwrap()));
}

#[test]
fn copilot_mcp_http_dry_run_shows_secret_references_and_creates_nothing() {
    let home = TestHome::new();
    home.config("[mcp.linear]\nall = { url = 'https://example.test/mcp', headers = { Accept = 'application/json' }, env_headers = { 'X-Api-Key' = 'API_KEY' }, bearer_token_env = 'TOKEN' }\n");
    let output = home
        .command()
        .env("TOKEN", "secret-must-not-appear")
        .env("API_KEY", "another-secret")
        .args(["--copilot", "--mcp", "linear", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("--additional-mcp-config"), "{stdout}");
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(!trace.contains("secret-must-not-appear"));
    assert!(!trace.contains("another-secret"));
    let config: serde_json::Value = serde_json::from_str(
        trace
            .lines()
            .find_map(|line| line.strip_prefix("MCP config: "))
            .unwrap(),
    )
    .unwrap();
    let expected = serde_json::json!({"mcpServers": { "linear": {
        "type": "http", "url": "https://example.test/mcp", "tools": ["*"],
        "headers": { "Accept": "application/json", "X-Api-Key": "${API_KEY}", "Authorization": "Bearer ${TOKEN}" }
    } }});
    assert_eq!(config, expected);
    assert!(trace.contains("not created yet"));
    assert!(!home.dir.path().join("cache").exists());
    assert!(home.no_record());
    let output = home
        .command()
        .env("TOKEN", "secret-must-not-appear")
        .env("API_KEY", "another-secret")
        .args(["--copilot", "--mcp", "linear"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let record = home.record();
    let mut args = record.lines();
    assert_eq!(args.next(), Some("--additional-mcp-config"));
    let path = args.next().unwrap().strip_prefix('@').unwrap();
    let contents = fs::read_to_string(path).unwrap();
    assert!(!contents.contains("secret-must-not-appear"));
    assert!(!contents.contains("another-secret"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&contents).unwrap(),
        expected
    );
}

#[test]
fn claude_mcp_stdio_uses_declaring_layer_paths_and_secret_references() {
    let home = TestHome::new();
    home.config("[mcp.files]\nall = { command = './bin/files', args = ['--root', '.'], env = { MODE = 'read' }, env_vars = ['TOKEN'] }\n");
    let executable = home.dir.path().join("config/ayran/bin/files");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(&executable, "fixture").unwrap();
    let output = home
        .command()
        .env("TOKEN", "secret-must-not-appear")
        .args(["--claude", "--mcp", "files", "--", "prompt"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let record = home.record();
    let args: Vec<_> = record.lines().collect();
    let index = args.iter().position(|arg| *arg == "--mcp-config").unwrap();
    let path = Path::new(args[index + 1]);
    assert!(path.starts_with(home.dir.path().join("cache/ayran/claude")));
    assert!(
        args[index + 2].starts_with('-'),
        "variadic flag must stop before prompt"
    );
    let contents = fs::read_to_string(path).unwrap();
    assert!(!contents.contains("secret-must-not-appear"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&contents).unwrap(),
        serde_json::json!({
            "mcpServers": { "files": { "command": executable, "args": ["--root", "."], "env": { "MODE": "read", "TOKEN": "${TOKEN}" } } }
        })
    );
    let again = home.run(&["--claude", "--mcp", "files"]);
    assert_eq!(again.status.code(), Some(0), "{again:?}");
    assert!(home.record().contains(path.to_str().unwrap()));
}

#[test]
fn claude_mcp_http_dry_run_has_secret_references_and_creates_nothing() {
    let home = TestHome::new();
    home.config("[mcp.linear]\nall = { url = 'https://example.test/mcp', headers = { Accept = 'application/json' }, env_headers = { 'X-Api-Key' = 'API_KEY' }, bearer_token_env = 'TOKEN' }\n");
    let output = home
        .command()
        .env("TOKEN", "secret-token")
        .env("API_KEY", "secret-key")
        .args(["--claude", "--mcp", "linear", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let trace = String::from_utf8(output.stderr).unwrap();
    let config = trace
        .lines()
        .find_map(|line| line.strip_prefix("MCP config: "))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(config).unwrap(),
        serde_json::json!({
            "mcpServers": { "linear": { "type": "http", "url": "https://example.test/mcp", "headers": { "Accept": "application/json", "X-Api-Key": "${API_KEY}", "Authorization": "Bearer ${TOKEN}" } } }
        })
    );
    assert!(trace.contains("MCP server linear: definition (explicit (--mcp); layer"));
    assert!(!trace.contains("secret-token"));
    assert!(!trace.contains("secret-key"));
    assert!(!home.dir.path().join("cache").exists());
    assert!(home.no_record());
    let output = home
        .command()
        .env("TOKEN", "secret-token")
        .env("API_KEY", "secret-key")
        .args(["--claude", "--mcp", "linear"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let record = home.record();
    let mut args = record.lines();
    assert_eq!(args.next(), Some("--mcp-config"));
    assert_eq!(fs::read_to_string(args.next().unwrap()).unwrap(), config);
}

#[test]
fn mcp_literals_reject_interpolation_even_when_unselected() {
    for binding in [
        "{ command = '${COMMAND}' }",
        "{ command = 'npx', args = ['${ARG}'] }",
        "{ command = 'npx', env = { T = '${TOKEN}' } }",
        "{ url = 'https://${HOST}/mcp' }",
        "{ url = 'https://example.test', headers = { T = '${TOKEN}' } }",
    ] {
        let home = TestHome::new();
        home.config(&format!("[mcp.x]\nall = {binding}\n"));
        let output = home.run(&["--claude"]);
        assert_eq!(output.status.code(), Some(3), "{binding}: {output:?}");
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("error[literal-interpolation]")
        );
        assert!(home.no_record());
    }
}

#[test]
fn mcp_schema_rejects_bad_names_unknown_keys_and_invalid_definitions_at_load() {
    for (config, code) in [
        ("[mcp.'bad.name']\nall = 'native'", "invalid-name"),
        ("[mcp.'']\nall = 'native'", "invalid-name"),
        ("[mcp.'é']\nall = 'native'", "invalid-name"),
        ("[mcp.'x,y']\nall = 'native'", "invalid-name"),
        ("[mcp.x]\nunknown = 'native'", "config-invalid"),
        (
            "[mcp.x]\nall = { command = 'npx', url = 'https://example.test' }",
            "config-invalid",
        ),
        ("[mcp.x]\nall = {}", "config-invalid"),
        ("[mcp.x]\nall = { path = './server' }", "config-invalid"),
        (
            "[mcp.x]\nall = { command = 'npx', typo = [] }",
            "config-invalid",
        ),
        (
            "[mcp.x]\nall = { url = 'https://example.test', args = [] }",
            "config-invalid",
        ),
        ("[mcp.x]\nall = { command = 1 }", "config-invalid"),
        (
            "[mcp.x]\nall = { command = 'npx', args = [1] }",
            "config-invalid",
        ),
        (
            "[mcp.x]\nall = { command = 'npx', env = { T = 1 } }",
            "config-invalid",
        ),
        (
            "[mcp.x]\nall = { command = 'npx', env_vars = 'TOKEN' }",
            "config-invalid",
        ),
        (
            "[mcp.x]\nall = { url = 'https://example.test', headers = [] }",
            "config-invalid",
        ),
        (
            "[mcp.x]\nall = { url = 'https://example.test', env_headers = { T = 1 } }",
            "config-invalid",
        ),
        (
            "[mcp.x]\nall = { url = 'https://example.test', bearer_token_env = true }",
            "config-invalid",
        ),
        ("[mcp.x]\nall = true", "config-invalid"),
        ("[mcp.x]\ndefault = 'true'", "config-invalid"),
        ("[mcp.x]\ndescription = 1", "config-invalid"),
    ] {
        let home = TestHome::new();
        home.config(config);
        let output = home.run(&["--claude"]);
        assert_eq!(output.status.code(), Some(3), "{config}: {output:?}");
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains(&format!("error[{code}]")),
            "{config}"
        );
        assert!(!home.dir.path().join("cache").exists());
        assert!(home.no_record());
    }
}

#[test]
fn mcp_native_and_definition_selections_accept_repeatable_comma_lists() {
    let home = TestHome::new();
    home.codex_config("[mcp_servers.codex-native]\ncommand = 'fixture'\n");
    fs::create_dir_all(home.dir.path().join(".copilot")).unwrap();
    fs::write(
        home.dir.path().join(".copilot/mcp-config.json"),
        r#"{"mcpServers":{"copilot-native":{"command":"fixture"}}}"#,
    )
    .unwrap();
    fs::write(
        home.dir.path().join(".claude.json"),
        r#"{"mcpServers":{"claude-native":{"command":"fixture"}}}"#,
    )
    .unwrap();
    home.config("[mcp.native]\nall = false\nclaude = 'claude-native'\ncodex = 'codex-native'\ncopilot = 'copilot-native'\n[mcp.x]\nall = { command = 'npx' }\n[mcp.y]\nclaude = { command = 'uvx' }\n");
    let output = home.run(&["--claude", "--mcp", "native,x", "--mcp", "y,x", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let trace = String::from_utf8(output.stderr).unwrap();
    assert!(trace.contains("MCP server native: native claude-native"));
    let config: serde_json::Value = serde_json::from_str(
        trace
            .lines()
            .find_map(|line| line.strip_prefix("MCP config: "))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        config,
        serde_json::json!({"mcpServers": {
            "x": { "command": "npx", "args": [], "env": {} },
            "y": { "command": "uvx", "args": [], "env": {} }
        }})
    );
    for harness in ["--codex", "--copilot"] {
        let output = home.run(&[harness, "--mcp", "native", "--dry-run"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }
}

#[test]
fn mcp_launch_errors_distinguish_missing_bindings_unknown_names_and_missing_commands() {
    for (config, name, code) in [
        (
            "[mcp.x]\nall = 'native'\nclaude = false",
            "x",
            "binding-absent",
        ),
        ("[mcp.x]\ncodex = 'native'", "x", "missing-binding"),
        ("", "unknown", "unknown-mcp"),
        (
            "[mcp.x]\nall = { command = './bin/missing' }",
            "x",
            "path-not-found",
        ),
        (
            "[mcp.x]\nall = { command = './bin' }",
            "x",
            "path-not-found",
        ),
    ] {
        let home = TestHome::new();
        home.config(config);
        let output = home.run(&["--claude", "--mcp", name]);
        assert_eq!(output.status.code(), Some(3), "{config}: {output:?}");
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains(&format!("error[{code}]")),
            "{config}"
        );
        assert!(home.no_record());
    }
    let home = TestHome::new();
    home.config(&format!(
        "[mcp.x]\nall = {{ command = '{}' }}",
        home.dir.path().join("absent").display()
    ));
    for harness in ["--claude", "--codex", "--copilot"] {
        let output = home.run(&[harness, "--mcp", "x"]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("error[path-not-found]")
        );
        assert!(home.no_record());
        assert!(!home.dir.path().join("cache").exists());
    }
    for harness in ["--claude", "--codex", "--copilot"] {
        let output = home.run(&[harness]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "unselected paths are unchecked: {output:?}"
        );
    }
}

#[test]
fn mcp_nearer_definitions_replace_whole_tables_and_keep_declaring_paths() {
    let home = TestHome::new();
    home.config("[mcp.x]\nall = { command = 'farther' }\nclaude = false\ndescription = 'farther description'\ndefault = true\n");
    let project = home.dir.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("ayran.toml"),
        "[mcp.x]\nall = { command = './server' }\n",
    )
    .unwrap();
    fs::write(project.join("server"), "fixture").unwrap();
    let output = home
        .command()
        .current_dir(&project)
        .args(["--claude", "--mcp", "x", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let trace = String::from_utf8(output.stderr).unwrap();
    let config: serde_json::Value = serde_json::from_str(
        trace
            .lines()
            .find_map(|line| line.strip_prefix("MCP config: "))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        config["mcpServers"]["x"]["command"],
        project.join("server").to_str().unwrap()
    );
    fs::write(
        project.join("ayran.local.toml"),
        "[mcp.x]\ncodex = 'nearer-native'\n",
    )
    .unwrap();
    let output = home
        .command()
        .current_dir(&project)
        .args(["--claude", "--mcp", "x"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("error[missing-binding]")
    );
}

#[test]
fn mcp_cache_is_refreshed_on_reuse_and_pruned_after_thirty_days() {
    let home = TestHome::new();
    home.config("[mcp.x]\nall = { command = 'npx' }\n");
    let output = home.run(&["--claude", "--mcp", "x"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let record = home.record();
    let file = Path::new(record.lines().nth(1).unwrap());
    let directory = file.parent().unwrap();
    let old = filetime::FileTime::from_unix_time(1, 0);
    filetime::set_file_mtime(directory, old).unwrap();
    let output = home.run(&["--claude", "--mcp", "x"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(file.is_file());
    assert_ne!(
        filetime::FileTime::from_last_modification_time(&fs::metadata(directory).unwrap()),
        old
    );
    filetime::set_file_mtime(directory, old).unwrap();
    let output = home.run(&["--claude"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(!directory.exists());
}

#[test]
fn mcp_dry_run_reports_whether_the_generated_directory_exists() {
    let home = TestHome::new();
    home.config("[mcp.x]\nall = { command = 'npx' }\n");
    let output = home.run(&["--claude", "--mcp", "x", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("(not created yet)")
    );
    let output = home.run(&["--claude", "--mcp", "x"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let output = home.run(&["--claude", "--mcp", "x", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("(exists)")
    );
}

#[test]
fn mcp_defaults_aliases_and_disables_follow_selection_precedence() {
    for (extra, flags, enabled, origin) in [
        ("", vec![], true, "Default"),
        (
            "[disable]\nmcp = ['files']\n",
            vec![],
            false,
            "disabled by layer",
        ),
        (
            "[disable]\nmcp = ['files']\n",
            vec!["--mcp", "files"],
            true,
            "explicit (--mcp)",
        ),
        (
            "",
            vec!["--no-defaults"],
            false,
            "disabled by --no-defaults",
        ),
        (
            "",
            vec!["--mcp", "files", "--no-defaults"],
            true,
            "explicit (--mcp)",
        ),
        (
            "",
            vec!["--mcp", "files", "--no-mcp", "files"],
            false,
            "disabled by --no-mcp",
        ),
        (
            "[aliases.work]\nharness = 'claude'\nmcp = ['files']\ndefaults = false\n[aliases.work.disable]\nmcp = ['files']\n",
            vec!["--alias", "work"],
            true,
            "explicit (Alias work)",
        ),
        (
            "[aliases.work]\nharness = 'claude'\n[aliases.work.disable]\nmcp = ['files']\n",
            vec!["--alias", "work"],
            false,
            "disabled by Alias work",
        ),
        (
            "[aliases.work]\nharness = 'claude'\nmcp = ['files']\n",
            vec!["--alias", "work", "--no-mcp", "files"],
            false,
            "disabled by --no-mcp",
        ),
        (
            "[aliases.work]\nharness = 'claude'\ndefaults = false\n",
            vec!["--alias", "work"],
            false,
            "defaults = false",
        ),
    ] {
        let home = TestHome::new();
        home.config(&format!(
            "[mcp.files]\nall = {{ command = 'fixture' }}\ndefault = true\n{extra}"
        ));
        let mut args = vec!["--dry-run"];
        if !flags.contains(&"--alias") {
            args.push("--claude");
        }
        args.extend(flags);
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let trace = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).contains("--mcp-config"),
            enabled,
            "{trace}"
        );
        assert!(trace.contains(origin), "{trace}");
    }
}

#[test]
fn mcp_layer_disables_union_and_defaults_reset_only_farther_definitions() {
    let home = TestHome::new();
    home.config("[mcp.far]\nall = { command = 'far' }\ndefault = true\n[mcp.replaced]\nall = { command = 'old' }\ndefault = true\n[disable]\nmcp = ['same']\n");
    let project = home.dir.path().join("ayran.toml");
    fs::write(&project, "[disable]\ndefaults = true\nmcp = ['near']\n[mcp.same]\nall = { command = 'same' }\ndefault = true\n[mcp.replaced]\nall = { command = 'new' }\ndefault = true\n").unwrap();
    fs::write(home.dir.path().join("ayran.local.toml"), "[disable]\ndefaults = false\nmcp = []\n[mcp.near]\nall = { command = 'near' }\ndefault = true\n[mcp.local]\nall = { command = 'local' }\ndefault = true\n").unwrap();
    let output = home.run(&["--claude", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let trace = String::from_utf8_lossy(&output.stderr);
    assert!(
        trace.contains("MCP server far: disabled by layer"),
        "{trace}"
    );
    assert!(
        trace.contains("MCP server same: disabled by layer"),
        "{trace}"
    );
    assert!(
        trace.contains("MCP server near: disabled by layer"),
        "{trace}"
    );
    assert!(
        trace.contains("MCP server replaced: definition (Default"),
        "{trace}"
    );
    assert!(
        trace.contains("MCP server local: definition (Default"),
        "{trace}"
    );
    assert!(trace.contains("\"command\":\"new\""), "{trace}");
    let output = home.run(&["--claude", "--mcp", "far,same,near", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("disabled by layer"));
}

#[test]
fn mcp_profile_members_expand_recursively_and_disables_keep_hiding_native_servers() {
    for harness in ["claude", "codex", "copilot"] {
        let home = TestHome::new();
        home.config("[mcp.files]\nall = 'native'\n[profiles.work]\nmcp = ['files']\n[profiles.outer]\nprofiles = ['work']\n");
        fs::write(
            home.dir.path().join(".claude.json"),
            r#"{"mcpServers":{"native":{"command":"fixture"}}}"#,
        )
        .unwrap();
        home.codex_config("[mcp_servers.native]\ncommand = 'fixture'\n");
        fs::create_dir_all(home.dir.path().join(".copilot")).unwrap();
        fs::write(
            home.dir.path().join(".copilot/mcp-config.json"),
            r#"{"mcpServers":{"native":{"command":"fixture"}}}"#,
        )
        .unwrap();
        for (extra, origin, hidden) in [
            (vec![], "via Profile work", false),
            (vec!["--no-mcp", "files"], "disabled by --no-mcp", true),
            (vec!["--no-profile", "work"], "unselected", true),
        ] {
            let mut args = vec!["--harness", harness, "--profile", "outer", "--dry-run"];
            args.extend(extra);
            let output = home.run(&args);
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            let trace = String::from_utf8_lossy(&output.stderr);
            assert!(trace.contains(origin), "{trace}");
            assert_eq!(
                trace.contains("MCP server native: hidden"),
                hidden,
                "{trace}"
            );
        }
    }
}

#[test]
fn mcp_profile_dangling_references_follow_default_and_explicit_origins() {
    for (default, flags, code) in [
        (false, vec!["--profile", "work"], 3),
        (true, vec![], 0),
        (true, vec!["--profile", "work"], 3),
    ] {
        let home = TestHome::new();
        home.config(&format!(
            "[profiles.work]\nmcp = ['missing']\ndefault = {default}\n"
        ));
        let mut args = vec!["--claude", "--dry-run"];
        args.extend(flags);
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(code), "{output:?}");
        let trace = String::from_utf8_lossy(&output.stderr);
        assert!(
            trace.contains("dangling-ref") && trace.contains("undefined MCP server missing"),
            "{trace}"
        );
    }
}

#[test]
fn mcp_false_bindings_skip_defaults_and_profiles_but_explicit_selection_fails() {
    for selection in ["default = true", ""] {
        let home = TestHome::new();
        home.config(&format!(
            "[mcp.files]\nall = false\n{selection}\n[profiles.work]\nmcp = ['files']\n"
        ));
        let mut args = vec!["--claude", "--dry-run"];
        if selection.is_empty() {
            args.extend(["--profile", "work"]);
        }
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("note[binding-skipped]"));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("--mcp-config"));
        let output = home.run(&["--claude", "--mcp", "files", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("binding-absent"));
        let output = home.run(&[
            "--claude",
            "--mcp",
            "files",
            "--no-mcp",
            "files",
            "--dry-run",
        ]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }
    let home = TestHome::new();
    let output = home.run(&[
        "--claude",
        "--mcp",
        "missing",
        "--no-mcp",
        "missing",
        "--dry-run",
    ]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown-mcp"));
}

// Identity is tested separately; Capability assertions ignore the generated UUID.
fn without_session_id(value: &str) -> String {
    for separator in ["\n", " "] {
        let marker = format!("--session-id{separator}");
        if let Some(start) = value.find(&marker) {
            let id_start = start + marker.len();
            if let Some(end) = value[id_start..].find([' ', '\n']) {
                let id_end = id_start + end;
                if uuid::Uuid::parse_str(&value[id_start..id_end]).is_ok() {
                    let mut result = value.to_owned();
                    let (start, end) = if separator == " " && value.as_bytes()[id_end] == b'\n' {
                        (start - 1, id_end)
                    } else {
                        (start, id_end + separator.len())
                    };
                    result.replace_range(start..end, "");
                    return result;
                }
            }
        }
    }
    value.to_owned()
}

#[test]
fn codex_native_link_is_manual_persistent_and_reused_without_rollouts() {
    let home = TestHome::new();
    assert!(home.run(&["--codex"]).status.success());
    let path = home.session_path();
    let before = fs::read(&path).unwrap();
    let record: serde_json::Value = serde_json::from_slice(&before).unwrap();
    let id = record["id"].as_str().unwrap();
    let native = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let output = home.run(&[
        "resume",
        id,
        "--native",
        native,
        "--dry-run",
        "--json",
        "--",
        "-m",
        "native-only",
    ]);
    assert!(output.status.success(), "{output:?}");
    let dry: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(dry["argv"][2], native);
    assert!(
        !dry["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "codex-resume-override")
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(
        home.run(&["resume", id, "--native", native])
            .status
            .success()
    );
    let cached: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(cached["native_id"], native);
    assert!(home.run(&["resume", "--last", "--codex"]).status.success());
    assert!(
        home.raw_record()
            .starts_with(&format!("resume\n{native}\n"))
    );
}

#[test]
fn dry_run_json_preserves_capability_and_leak_metadata() {
    let home = TestHome::new();
    home.config("[skills.tdd]\ndefault=true\ncodex=false\n");
    let output = home.run(&["--codex", "--dry-run", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let diagnostics = json["diagnostics"].as_array().unwrap();
    let skipped = diagnostics
        .iter()
        .find(|d| d["code"] == "binding-skipped")
        .unwrap();
    assert_eq!(skipped["harness"], "codex");
    assert_eq!(
        skipped["capability"],
        serde_json::json!({"kind":"skill","name":"tdd"})
    );
    let leak = diagnostics.iter().find(|d| d["code"] == "leak").unwrap();
    assert_eq!(leak["harness"], "codex");
    assert_eq!(leak["cause"], "codex-remote-plugin");
}

#[test]
fn claude_connector_selection_denies_only_other_cached_connectors() {
    let home = TestHome::new();
    home.config("[mcp.linear]\nclaude = { connector = 'claude.ai Linear' }\n");
    fs::write(
        home.dir.path().join(".claude.json"),
        r#"{"claudeAiMcpEverConnected":["claude.ai Linear","claude.ai Slack"]}"#,
    )
    .unwrap();
    let output = home.run(&["--claude", "--mcp", "linear", "--dry-run", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let argv = value["argv"].as_array().unwrap();
    let index = argv.iter().position(|arg| arg == "--settings").unwrap();
    let settings: serde_json::Value =
        serde_json::from_str(argv[index + 1].as_str().unwrap()).unwrap();
    assert!(settings.get("disableClaudeAiConnectors").is_none());
    assert_eq!(
        settings["deniedMcpServers"],
        serde_json::json!([{"serverName":"claude.ai Slack"}])
    );
    assert!(
        value["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "leak"
                && d["cause"] == "claude-connector"
                && d["severity"] == "warning")
    );
    let trace = home.run(&["--claude", "--mcp", "linear", "--dry-run"]);
    assert!(
        String::from_utf8_lossy(&trace.stderr)
            .contains("MCP server linear: connector claude.ai Linear")
    );
}

#[test]
fn claude_connectors_follow_selection_origins_and_disables_without_a_cache() {
    for (extra, flags, selected) in [
        ("", vec!["--mcp", "linear"], true),
        ("default = true\n", vec![], true),
        (
            "[profiles.team]\nmcp = ['linear']\n",
            vec!["-p", "team"],
            true,
        ),
        (
            "[aliases.work]\nharness = 'claude'\nmcp = ['linear']\n",
            vec!["--alias", "work"],
            true,
        ),
        ("default = true\n", vec!["--no-defaults"], false),
        ("default = true\n", vec!["--no-mcp", "linear"], false),
    ] {
        let home = TestHome::new();
        home.config(&format!(
            "[mcp.linear]\nclaude = {{ connector = 'claude.ai Linear' }}\n{extra}"
        ));
        let mut args = vec!["--claude", "--dry-run"];
        args.extend(flags);
        let output = home.run(&args);
        assert!(output.status.success(), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            stderr.contains("warning[leak]: claude-connector"),
            selected,
            "{stderr}"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).contains("disableClaudeAiConnectors"),
            !selected
        );
        args.push("-q");
        let quiet = home.run(&args);
        assert!(
            quiet.status.success() && quiet.stderr.is_empty(),
            "{quiet:?}"
        );
    }
}

#[test]
fn connector_bindings_reject_invalid_tables_and_unsupported_harnesses() {
    for binding in [
        "{ connector = 3 }",
        "{ connector = 'x', url = 'https://example.com' }",
        "{ connector = 'x', extra = true }",
    ] {
        let home = TestHome::new();
        home.config(&format!("[mcp.x]\nclaude = {binding}\n"));
        let output = home.run(&["--claude", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("config-invalid"));
    }
    for (field, harness) in [
        ("all", "claude"),
        ("all", "codex"),
        ("all", "copilot"),
        ("copilot", "copilot"),
    ] {
        let home = TestHome::new();
        home.config(&format!("[mcp.x]\n{field} = {{ connector = 'x' }}\n"));
        let output = home.run(&[&format!("--{harness}"), "--mcp", "x", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported-binding"));
        assert!(home.no_record());
    }
}

#[test]
fn claude_connector_disabled_toggle_and_empty_cache_still_launch() {
    let home = TestHome::new();
    fs::create_dir(home.dir.path().join(".git")).unwrap();
    home.config("[mcp.linear]\nclaude = { connector = 'claude.ai Linear' }\n");
    fs::write(home.dir.path().join(".claude.json"), serde_json::json!({
        "claudeAiMcpEverConnected": [],
        "projects": {home.dir.path().to_string_lossy(): {"disabledMcpServers": ["claude.ai Linear"]}}
    }).to_string()).unwrap();
    let output = home.run(&["--claude", "--mcp", "linear"]);
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("note[mcp-disabled]") && stderr.contains("warning[leak]: claude-connector"),
        "{stderr}"
    );
    assert_eq!(
        home.claude_settings(),
        serde_json::json!({"syncClaudeAiSkills":false})
    );
}

#[test]
fn claude_connector_deny_list_uses_the_session_home_and_combines_with_native_hides() {
    for isolated in [false, true] {
        let home = TestHome::new();
        home.config(&format!(
            "[mcp.linear]\nclaude = {{ connector = 'claude.ai Linear' }}\n{}",
            if isolated {
                "[harnesses.claude]\nhome = 'isolated'\n"
            } else {
                ""
            }
        ));
        fs::write(
            home.dir.path().join(".claude.json"),
            r#"{"claudeAiMcpEverConnected":["claude.ai Wrong home"]}"#,
        )
        .unwrap();
        let directory = home.dir.path().join(if isolated {
            "state/ayran/homes/claude"
        } else {
            "custom"
        });
        fs::create_dir_all(&directory).unwrap();
        let state = r#"{"claudeAiMcpEverConnected":["claude.ai Linear","claude.ai Slack"],"mcpServers":{"hidden":{"command":"fixture"}}}"#;
        fs::write(directory.join(".claude.json"), state).unwrap();
        let output = home
            .command()
            .env("CLAUDE_CONFIG_DIR", &directory)
            .args(["--claude", "--mcp", "linear"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            home.claude_settings()["deniedMcpServers"],
            serde_json::json!([{"serverName":"claude.ai Slack"},{"serverName":"hidden"}])
        );
        assert_eq!(
            fs::read_to_string(directory.join(".claude.json")).unwrap(),
            state
        );
        assert_eq!(
            home.record()
                .lines()
                .filter(|arg| *arg == "--settings")
                .count(),
            1
        );
    }
}

#[test]
fn selected_claude_connector_is_not_denied_when_a_configured_server_shares_its_name() {
    let home = TestHome::new();
    home.config("[mcp.linear]\nclaude = { connector = 'claude.ai Linear' }\n");
    fs::write(home.dir.path().join(".claude.json"),
        r#"{"claudeAiMcpEverConnected":["claude.ai Linear","claude.ai Slack"],"mcpServers":{"claude.ai Linear":{"command":"fixture"}}}"#).unwrap();
    let output = home.run(&["--claude", "--mcp", "linear"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings()["deniedMcpServers"],
        serde_json::json!([{"serverName":"claude.ai Slack"}])
    );
}

#[test]
fn codex_connector_selection_enables_only_selected_apps() {
    let home = TestHome::new();
    home.config("[mcp.x]\ncodex = { connector = 'connector_x' }\n");
    let output = home.run(&["--codex", "--mcp", "x", "--dry-run", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let argv = plan["argv"].as_array().unwrap();
    assert!(argv.contains(&serde_json::json!("apps._default.enabled=false")));
    assert!(argv.contains(&serde_json::json!("apps.connector_x.enabled=true")));
    assert!(!argv.contains(&serde_json::json!("--disable")));
    assert!(!plan["diagnostics"].as_array().unwrap().iter().any(|d| d["code"] == "leak" && d["cause"].as_str().unwrap_or("").contains("connector")));
}

#[test]
fn codex_connector_selection_hides_apps_enabled_in_the_session_home() {
    for isolated in [false, true] {
        let home = TestHome::new();
        home.config(&format!(
            "[mcp.x]\ncodex = {{ connector = 'connector_x' }}\n{}",
            if isolated {
                "[harnesses.codex]\nhome = 'isolated'\n"
            } else {
                ""
            }
        ));
        let contents = "[apps.connector_x]\nenabled = true\n[apps.connector_y]\nenabled = true\n[apps.connector_z]\nenabled = false\n[apps.implicit]\ndescription = 'unspecified'\n";
        if isolated {
            home.codex_config("[apps.real_home]\nenabled = true\n");
            let directory = home.dir.path().join("state/ayran/homes/codex");
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("config.toml"), contents).unwrap();
        } else {
            home.codex_config(contents);
        }
        let output = home.run(&["--codex", "--mcp", "x", "--dry-run", "--json"]);
        assert!(output.status.success(), "{output:?}");
        let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let argv = plan["argv"].as_array().unwrap();
        assert!(
            argv.contains(&serde_json::json!("apps.connector_y.enabled=false")),
            "{plan}"
        );
        assert!(!argv.contains(&serde_json::json!("apps.connector_x.enabled=false")));
        assert!(!argv.contains(&serde_json::json!("apps.connector_z.enabled=false")));
        assert!(!argv.contains(&serde_json::json!("apps.implicit.enabled=false")));
        assert!(!argv.contains(&serde_json::json!("apps.real_home.enabled=false")));
    }
}

#[test]
fn codex_connector_selection_handles_dotted_ids_and_whole_apps_precedence() {
    let home = TestHome::new();
    home.config("[mcp.x]\ncodex = { connector = 'a.b' }\n[mcp.accounts]\ncodex = 'codex_apps'\n");
    home.codex_config("[apps.'c.d']\nenabled = true\n[apps._default]\nenabled = true\n");
    for selection in ["x", "x,accounts", "accounts,x"] {
        let output = home.run(&["--codex", "--mcp", selection, "--dry-run", "--json"]);
        assert!(output.status.success(), "{output:?}");
        let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let argv = plan["argv"].as_array().unwrap();
        assert!(!argv.contains(&serde_json::json!("--disable")));
        if selection == "x" {
            assert!(
                argv.contains(&serde_json::json!("apps={\"a.b\"={enabled=true}}")),
                "{plan}"
            );
            assert!(argv.contains(&serde_json::json!("apps={\"c.d\"={enabled=false}}")));
            assert_eq!(
                argv.iter()
                    .filter(|a| **a == serde_json::json!("apps._default.enabled=false"))
                    .count(),
                1
            );
        } else {
            assert!(
                !argv.iter().any(|a| a.as_str().unwrap().starts_with("apps")),
                "{plan}"
            );
        }
    }
}

#[test]
fn codex_connector_selection_is_resolved_again_on_resume_and_fork() {
    let home = TestHome::new();
    home.config("[mcp.x]\ncodex = { connector = 'connector_x' }\n");
    assert!(home.run(&["--codex", "--mcp", "x"]).status.success());
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(home.session_path()).unwrap()).unwrap();
    let id = record["id"].as_str().unwrap();
    let native = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let directory = home.dir.path().join(".codex/sessions");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("rollout-parent.jsonl"), serde_json::json!({"type":"session_meta","payload":{"id":native,"cwd":record["cwd"],"timestamp":record["started_at"],"source":"cli"}}).to_string()).unwrap();
    home.config("[mcp.x]\ncodex = { connector = 'connector_changed' }\n");
    home.codex_config("[apps.connector_y]\nenabled = true\n");
    for fork in [false, true] {
        let mut args = vec!["resume", id, "--dry-run", "--json"];
        if fork {
            args.push("--fork");
        }
        let output = home.run(&args);
        assert!(output.status.success(), "{output:?}");
        let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let argv = plan["argv"].as_array().unwrap();
        assert_eq!(argv[1], if fork { "fork" } else { "resume" });
        assert_eq!(argv[2], native);
        for setting in [
            "apps._default.enabled=false",
            "apps.connector_changed.enabled=true",
            "apps.connector_y.enabled=false",
        ] {
            assert!(argv.contains(&serde_json::json!(setting)), "{plan}");
        }
        assert!(!argv.contains(&serde_json::json!("apps.connector_x.enabled=true")));
        assert!(!argv.contains(&serde_json::json!("--disable")));
    }
}

#[test]
fn copilot_custom_skill_directories_are_discovered_and_leak_only_when_unselected() {
    let home = TestHome::new();
    home.skill("absolute/old-name", "absolute");
    home.skill("relative/relative", "relative");
    home.skill("~/literal/tilde", "tilde");
    home.skill("literal/expanded", "expanded");
    home.skill("linked-source", "linked");
    std::os::unix::fs::symlink(
        home.dir.path().join("linked-source"),
        home.dir.path().join("relative/link"),
    )
    .unwrap();
    fs::create_dir_all(home.dir.path().join(".copilot")).unwrap();
    fs::write(home.dir.path().join(".copilot/settings.json"), serde_json::json!({
        "skillDirectories": [home.dir.path().join("absolute"), "relative", "~/literal", "missing"],
        "disabledSkills": ["absolute"]
    }).to_string()).unwrap();
    home.config("[skills.chosen]\ncopilot = 'absolute'\n[skills.relative]\ncopilot = 'relative'\n[skills.tilde]\ncopilot = 'tilde'\n[skills.linked]\ncopilot = 'linked'\n");
    let output = home.run(&["--copilot", "--skill", "chosen", "--dry-run", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let leaks: Vec<_> = json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "leak")
        .collect();
    assert_eq!(leaks.len(), 1, "{json}");
    assert_eq!(leaks[0]["cause"], "copilot-custom-skill-dir");
    assert_eq!(leaks[0]["harness"], "copilot");
    assert_eq!(leaks[0]["item"], "linked, relative, tilde");
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "skill-disabled")
    );
    let output = home.run(&["--copilot", "--skill", "chosen"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(stderr.matches("warning[leak]").count(), 1, "{stderr}");
    assert!(
        stderr.contains("copilot-custom-skill-dir") && stderr.contains("linked, relative, tilde"),
        "{stderr}"
    );
    let quiet = home.run(&["--copilot", "--skill", "chosen", "-q"]);
    assert!(
        quiet.status.success() && quiet.stderr.is_empty(),
        "{quiet:?}"
    );
    let output = home.run(&[
        "--copilot",
        "--skill",
        "chosen,relative,tilde,linked",
        "--dry-run",
        "--json",
    ]);
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["code"] != "leak"),
        "{json}"
    );
}

#[test]
fn copilot_inherited_skill_dirs_are_removed_on_launch_and_resume_in_every_home_mode() {
    for isolated in [false, true] {
        let home = TestHome::new();
        home.skill("env-skills/env-only", "env-only");
        home.config(&format!("[skills.env_only]\ncopilot = 'env-only'\n[skills.path]\ncopilot = {{path = '~/env-skills/env-only'}}\n{}", if isolated { "[harnesses.copilot]\nhome = 'isolated'\n" } else { "" }));
        fs::write(home.dir.path().join("bin/copilot"), "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 9.0.0; exit 0; fi\nprintf '%s\\n' \"${COPILOT_SKILLS_DIRS-unset}\" > \"$HOME/skills_env_record\"\n").unwrap();
        let run = |args: &[&str]| {
            home.command()
                .env("COPILOT_SKILLS_DIRS", home.dir.path().join("env-skills"))
                .args(args)
                .output()
                .unwrap()
        };
        let output = run(&["--copilot", "--dry-run"]);
        assert!(output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("env -u COPILOT_SKILLS_DIRS"),
            "{output:?}"
        );
        let output = run(&["--copilot", "--skill", "env_only"]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("native-not-found"));
        let output = run(&["--copilot", "--skill", "path"]);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            fs::read_to_string(home.dir.path().join("skills_env_record")).unwrap(),
            "unset\n"
        );
        let record: serde_json::Value =
            serde_json::from_slice(&fs::read(home.session_path()).unwrap()).unwrap();
        let id = record["id"].as_str().unwrap();
        let output = run(&["resume", id, "--dry-run", "--json"]);
        assert!(output.status.success(), "{output:?}");
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            json["env_remove"],
            serde_json::json!(["COPILOT_SKILLS_DIRS"])
        );
        fs::remove_file(home.dir.path().join("skills_env_record")).unwrap();
        let output = run(&["resume", id]);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            fs::read_to_string(home.dir.path().join("skills_env_record")).unwrap(),
            "unset\n"
        );
    }
}

#[test]
fn copilot_custom_skill_leaks_follow_the_launch_home_including_isolated_settings() {
    let home = TestHome::new();
    home.skill("shared-skills/shared", "shared");
    home.skill("override-skills/override", "override");
    home.skill("isolated-skills/isolated", "isolated");
    for (directory, skills) in [
        (".copilot", "shared-skills"),
        ("custom-home", "override-skills"),
    ] {
        fs::create_dir_all(home.dir.path().join(directory)).unwrap();
        fs::write(
            home.dir.path().join(directory).join("settings.json"),
            serde_json::json!({"skillDirectories": [skills]}).to_string(),
        )
        .unwrap();
    }
    home.config("");
    let output = home
        .command()
        .env("COPILOT_HOME", "custom-home")
        .args(["--copilot", "--dry-run", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["diagnostics"][0]["item"], "override");
    home.config("[harnesses.copilot]\nhome = 'isolated'\n");
    let output = home.run(&["--copilot", "--dry-run", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["diagnostics"], serde_json::json!([]));
    let isolated = home.dir.path().join("state/ayran/homes/copilot");
    fs::create_dir_all(&isolated).unwrap();
    fs::write(
        isolated.join("settings.json"),
        r#"{"skillDirectories":["isolated-skills"]}"#,
    )
    .unwrap();
    let output = home.run(&["--copilot", "--dry-run", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["diagnostics"][0]["cause"], "copilot-custom-skill-dir");
    assert_eq!(json["diagnostics"][0]["item"], "isolated");
}

#[test]
fn harness_args_follow_generated_flags_alias_and_passthrough() {
    let home = TestHome::new();
    home.config("[harnesses.claude]\nargs = ['--harness-flag', 'value']\n[aliases.work]\nharness = 'claude'\nargs = ['--alias-flag']\n");
    let output = home.run(&[
        "--alias",
        "work",
        "-m",
        "sonnet",
        "--dry-run",
        "--json",
        "--",
        "--typed-flag",
    ]);
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let argv = json["argv"].as_array().unwrap();
    assert_eq!(
        &argv[argv.len() - 4..],
        &["--harness-flag", "value", "--alias-flag", "--typed-flag"]
    );
    let output = home.run(&["--alias", "work", "--dry-run"]);
    let trace = String::from_utf8_lossy(&output.stderr);
    assert!(
        trace.contains("--harness-flag") && trace.contains("ayran.toml"),
        "{trace}"
    );
    assert!(trace.contains("--alias-flag (Alias work)"), "{trace}");
}

#[test]
fn harness_args_resume_fork_and_persist_suppression() {
    let home = TestHome::new();
    home.config("[harnesses.claude]\nargs = ['--old']\n[aliases.work]\nharness = 'claude'\nargs = ['--alias']\n");
    assert!(
        home.run(&["--alias", "work", "--", "--once"])
            .status
            .success()
    );
    home.config("[harnesses.claude]\nargs = ['--new']\n[aliases.work]\nharness = 'claude'\nargs = ['--alias']\n");
    for fork in [false, true] {
        let mut args = vec!["resume", "--last", "--dry-run", "--json"];
        if fork {
            args.push("--fork");
        }
        let output = home.run(&args);
        assert!(output.status.success(), "{output:?}");
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let argv = json["argv"].as_array().unwrap();
        assert_eq!(&argv[argv.len() - 2..], &["--new", "--alias"]);
        assert!(!argv.contains(&serde_json::json!("--once")));
    }
    assert!(
        home.run(&["resume", "--last", "--no-harness-args", "--", "--typed"])
            .status
            .success()
    );
    assert!(home.raw_record().ends_with("--typed\n"));
    let output = home.run(&["resume", "--last", "--dry-run", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        !json["argv"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("--new"))
    );
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(home.session_path()).unwrap()).unwrap();
    assert_eq!(record["request"]["no_harness_args"], true);
}

#[test]
fn doctor_warns_only_for_managed_flags_in_harness_and_alias_args() {
    let home = TestHome::new();
    home.config("[harnesses.claude]\nargs = ['--model=opus', '--effort', 'high']\n[harnesses.codex]\nargs = ['-m', 'gpt-5', '-c', 'model_reasoning_effort=\"high\"']\n[aliases.work]\nharness = 'copilot'\nargs = ['--reasoning-effort=high']\n[aliases.safe]\nharness = 'copilot'\nargs = ['--allow-all', '--model-like']\n");
    let output = home.run(&["doctor", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let warnings: Vec<_> = json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "harness-args-overlap")
        .collect();
    assert_eq!(warnings.len(), 3, "{json}");
    assert!(warnings.iter().all(|d| d["severity"] == "warning"));
}

#[test]
fn harness_args_private_layers_replace_lists_and_alias_can_drop_harness_args() {
    let home = TestHome::new();
    home.config("[harnesses.claude]\nmodel = 'sonnet'\nargs = ['--far']\n[aliases.work]\nharness = 'claude'\nargs = ['--alias']\nharness_args = false\n");
    fs::write(
        home.dir.path().join("ayran.local.toml"),
        "[harnesses.claude]\nargs = ['--near', '--near']\n",
    )
    .unwrap();
    for (args, suffix) in [
        (
            vec!["--claude", "--dry-run", "--json"],
            vec!["--near", "--near"],
        ),
        (
            vec!["--alias", "work", "--dry-run", "--json"],
            vec!["--alias"],
        ),
        (
            vec![
                "--alias",
                "work",
                "--no-harness-args",
                "--dry-run",
                "--json",
                "--",
                "--typed",
            ],
            vec!["--typed"],
        ),
    ] {
        let output = home.run(&args);
        assert!(output.status.success(), "{output:?}");
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let argv = json["argv"].as_array().unwrap();
        assert_eq!(&argv[argv.len() - suffix.len()..], suffix);
        assert!(!argv.contains(&serde_json::json!("--far")));
        assert!(argv.contains(&serde_json::json!("sonnet")));
    }
    fs::write(
        home.dir.path().join("ayran.local.toml"),
        "[harnesses.claude]\nargs = []\n",
    )
    .unwrap();
    assert!(home.run(&["--claude"]).status.success());
    assert!(!home.record().contains("--far"));
}

#[test]
fn harness_args_reject_project_layers_and_invalid_entries() {
    let home = TestHome::new();
    fs::write(
        home.dir.path().join("ayran.toml"),
        "[harnesses.claude]\nargs = []\n",
    )
    .unwrap();
    for (args, status) in [(vec!["--claude", "--dry-run"], 3), (vec!["doctor"], 1)] {
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(status), "{output:?}");
        let diagnostics = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(diagnostics.contains("private-layer-only"), "{diagnostics}");
    }
    fs::remove_file(home.dir.path().join("ayran.toml")).unwrap();
    for prefix in ["[harnesses.claude]", "[aliases.work]\nharness = 'claude'"] {
        for value in ["['']", "['--']", "[1]", "'flag'"] {
            home.config(&format!("{prefix}\nargs = {value}\n"));
            let output = home.run(&["--claude", "--dry-run"]);
            assert_eq!(output.status.code(), Some(3), "{output:?}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("config-invalid"));
        }
    }
}

#[test]
fn codex_native_off_servers_skip_overrides_but_project_values_keep_them() {
    let home = TestHome::new();
    home.codex_config("[mcp_servers.off]\ncommand = 'server'\nenabled = false\n[mcp_servers.on]\ncommand = 'server'\nenabled = true\n[mcp_servers.default]\ncommand = 'server'\n");
    home.config("[mcp.on]\ncodex = 'on'\n[mcp.default]\ncodex = 'default'\n");
    let output = home.run(&["--codex", "--mcp", "on,default", "--dry-run", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let args = json["argv"].as_array().unwrap();
    assert!(
        !args
            .iter()
            .any(|arg| arg == "mcp_servers.off.enabled=false"),
        "{json}"
    );
    assert!(
        !args.iter().any(|arg| arg == "mcp_servers.on.enabled=true"),
        "{json}"
    );
    assert!(
        args.iter()
            .any(|arg| arg == "mcp_servers.default.enabled=true"),
        "{json}"
    );
    let trace = home.run(&["--codex", "--mcp", "on,default", "--dry-run"]);
    assert!(String::from_utf8_lossy(&trace.stderr).contains("hidden (natively off"));
    let project = home.dir.path().join("project");
    fs::create_dir_all(project.join(".codex")).unwrap();
    fs::write(
        project.join(".codex/config.toml"),
        "[mcp_servers.off]\nenabled = true\n",
    )
    .unwrap();
    let output = home
        .command()
        .current_dir(project)
        .args(["--codex", "--dry-run"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("mcp_servers.off.enabled=false"));
}

#[test]
fn doctor_enumerates_codex_profiles_in_harness_and_alias_args() {
    let home = TestHome::new();
    home.codex_config("");
    home.config("[harnesses.codex]\nargs = ['-pwork']\n[aliases.other]\nharness = 'codex'\nargs = ['--profile=other']\n[mcp.work]\ncodex = 'work'\n[mcp.other]\ncodex = 'other'\n");
    for name in ["work", "other"] {
        fs::write(
            home.dir.path().join(format!(".codex/{name}.config.toml")),
            format!("[mcp_servers.{name}]\ncommand = 'server'\n"),
        )
        .unwrap();
    }
    let output = home.run(&["doctor", "--codex", "--json"]);
    assert!(output.status.success(), "{output:?}");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("native-not-found"),
        "{output:?}"
    );
    fs::write(
        home.dir.path().join(".codex/other.config.toml"),
        "invalid = [",
    )
    .unwrap();
    let output = home.run(&["doctor", "--codex", "--json"]);
    assert!(!output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("enumeration-failed"));
}

#[test]
fn codex_profile_spelling_sources_and_suppression_control_native_items() {
    let home = TestHome::new();
    home.config("[mcp.connector]\ncodex = { connector = 'selected_app' }\n");
    home.codex_config("[mcp_servers.base]\ncommand = 'server'\nenabled = false\n");
    fs::write(home.dir.path().join(".codex/work.config.toml"), "[mcp_servers.base]\nenabled = true\n[mcp_servers.profile]\ncommand = 'server'\nenabled = false\n[apps.profile_app]\nenabled = true\n").unwrap();
    for flags in [
        vec!["-p", "work"],
        vec!["-pwork"],
        vec!["--profile", "work"],
        vec!["--profile=work"],
    ] {
        let mut args = vec!["--codex", "--mcp", "connector", "--dry-run", "--"];
        args.extend(flags);
        let output = home.run(&args);
        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let trace = String::from_utf8_lossy(&output.stderr);
        assert!(
            stdout.contains("mcp_servers.base.enabled=false"),
            "{stdout}"
        );
        assert!(
            !stdout.contains("mcp_servers.profile.enabled=false"),
            "{stdout}"
        );
        assert!(
            stdout.contains("apps.profile_app.enabled=false"),
            "{stdout}"
        );
        assert!(
            trace.contains("MCP server profile: hidden (natively off"),
            "{trace}"
        );
    }
    home.config("[harnesses.codex]\nargs = ['-pwork']\n[aliases.work]\nharness = 'codex'\nargs = ['--profile=work']\n[mcp.profile]\ncodex = 'profile'\n");
    let output = home.run(&["--codex", "--dry-run"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("mcp_servers.base.enabled=false"));
    let output = home.run(&[
        "--codex",
        "--no-harness-args",
        "--mcp",
        "profile",
        "--dry-run",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("native-not-found"));
    // Both configured lists name a profile: duplicate flags disable every optimisation.
    let output = home.run(&["--alias", "work", "--dry-run"]);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("mcp_servers.profile.enabled=false"));
    home.config("[aliases.work]\nharness = 'codex'\nargs = ['--profile', 'work']\n[mcp.profile]\ncodex = 'profile'\n");
    let output = home.run(&["--alias", "work", "--mcp", "profile", "--dry-run"]);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("mcp_servers.profile.enabled=true"));
    let output = home.run(&["--codex", "--dry-run", "--", "-p", "missing"]);
    assert!(output.status.success(), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("mcp_servers.base.enabled=false"));
    fs::write(
        home.dir.path().join(".codex/work.config.toml"),
        "broken = [",
    )
    .unwrap();
    let output = home.run(&["--alias", "work", "--dry-run"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("enumeration-failed"));
}

#[test]
fn codex_profile_plugins_use_explicit_state_and_require_an_active_cache() {
    let home = TestHome::new();
    home.codex_config("[plugins.'off@m']\nenabled = false\n[plugins.'on@m']\nenabled = true\n[plugins.'default@m']\n");
    home.config("[plugins.on]\ncodex = 'on@m'\n[plugins.default]\ncodex = 'default@m'\n");
    let output = home.run(&["--codex", "--plugin", "on,default", "--dry-run"]);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("plugins.off@m.enabled=false"), "{stdout}");
    assert!(!stdout.contains("plugins.on@m.enabled=true"), "{stdout}");
    assert!(
        stdout.contains("plugins.default@m.enabled=true"),
        "{stdout}"
    );
    fs::write(home.dir.path().join(".codex/work.config.toml"), "[plugins.'off@m']\nenabled = true\n[plugins.'on@m']\nenabled = false\n[plugins.'profile@m']\nenabled = true\n[plugins.'stale@m']\nenabled = true\n").unwrap();
    fs::create_dir_all(home.dir.path().join(".codex/plugins/cache/m/profile/local")).unwrap();
    let output = home.run(&["--codex", "--dry-run", "--", "-pwork"]);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("plugins.off@m.enabled=false"), "{stdout}");
    assert!(!stdout.contains("plugins.on@m.enabled=false"), "{stdout}");
    assert!(
        stdout.contains("plugins.profile@m.enabled=false"),
        "{stdout}"
    );
    assert!(!stdout.contains("stale@m"), "{stdout}");
    let project = home.dir.path().join("project");
    fs::create_dir_all(project.join(".codex")).unwrap();
    fs::write(
        project.join(".codex/config.toml"),
        "[plugins.'off@m']\nenabled = true\n[plugins.'on@m']\nenabled = false\n",
    )
    .unwrap();
    let output = home
        .command()
        .current_dir(project)
        .args(["--codex", "--plugin", "on", "--dry-run"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("plugins.off@m.enabled=false"), "{stdout}");
    assert!(stdout.contains("plugins.on@m.enabled=true"), "{stdout}");
}

#[test]
fn codex_skill_rules_use_last_matching_user_and_profile_rule() {
    let home = TestHome::new();
    home.skill(".codex/skills/off", "off");
    home.skill(".codex/skills/on", "on");
    home.skill("external/profile", "profile");
    home.config("[skills.on]\ncodex = 'on'\n[skills.off]\ncodex = 'off'\n[skills.profile]\ncodex = 'profile'\n");
    home.codex_config("[[skills.config]]\nname = 'off'\nenabled = false\n[[skills.config]]\nname = 'on'\nenabled = true\n");
    let output = home.run(&["--codex", "--skill", "on", "--dry-run"]);
    assert!(output.status.success(), "{output:?}");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("skills.config="),
        "{output:?}"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("natively off"));
    let path = home.dir.path().join("external/profile/SKILL.md");
    fs::write(home.dir.path().join(".codex/work.config.toml"), format!("[[skills.config]]\nname = 'off'\nenabled = true\n[[skills.config]]\npath = '{}'\nenabled = false\n", path.display())).unwrap();
    let output = home.run(&["--codex", "--skill", "off,profile", "--", "-pwork"]);
    assert!(output.status.success(), "{output:?}");
    let rules = home.codex_skill_rules();
    assert_eq!(rules.get(&path), Some(&true));
    assert!(!rules.contains_key(&home.dir.path().join(".codex/skills/off/SKILL.md")));
    assert_eq!(
        rules.get(&home.dir.path().join(".codex/skills/on/SKILL.md")),
        Some(&false)
    );
    let output = home.run(&["--codex", "--skill", "profile", "--dry-run"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("native-not-found"));
    // Ambiguous profiles retain both selected-on and unselected-off rules.
    let output = home.run(&["--codex", "--skill", "on", "--", "-pwork", "-pmissing"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(home.codex_skill_rules().len(), 3);
}

#[test]
fn codex_skill_native_proof_ignores_directory_and_ambiguous_selectors() {
    let home = TestHome::new();
    home.skill(".codex/skills/one", "one");
    home.skill(".codex/skills/two", "two");
    home.codex_config(&format!("[[skills.config]]\npath = '{}'\nenabled = false\n[[skills.config]]\npath = '{}'\nname = 'two'\nenabled = false\n", home.dir.path().join(".codex/skills/one").display(), home.dir.path().join(".codex/skills/two/SKILL.md").display()));
    let output = home.run(&["--codex"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(home.codex_skill_rules().len(), 2);
    assert!(home.codex_skill_rules().values().all(|enabled| !enabled));
}

#[test]
fn codex_profile_servers_join_definition_and_project_and_plugin_collisions() {
    let home = TestHome::new();
    home.codex_config("");
    fs::write(
        home.dir.path().join(".codex/work.config.toml"),
        "[mcp_servers.shared]\ncommand = 'server'\n[plugins.'plugin@m']\nenabled = true\n",
    )
    .unwrap();
    let plugin = home.dir.path().join(".codex/plugins/cache/m/plugin/local");
    fs::create_dir_all(plugin.join(".codex-plugin")).unwrap();
    fs::write(
        plugin.join(".codex-plugin/plugin.json"),
        r#"{"mcpServers":{"shared":{"command":"server"}}}"#,
    )
    .unwrap();
    home.config("[mcp.shared]\ncodex = { command = 'definition' }\n");
    let output = home.run(&["--codex", "--mcp", "shared", "--dry-run", "--", "-pwork"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("mcp-definition-collision"));
    home.config("[plugins.plugin]\ncodex = 'plugin@m'\n");
    let output = home.run(&["--codex", "--plugin", "plugin", "--dry-run", "--", "-pwork"]);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("plugin-server-shadow"));
    let project = home.dir.path().join("project");
    fs::create_dir_all(project.join(".codex")).unwrap();
    fs::write(
        project.join(".codex/config.toml"),
        "[mcp_servers.shared]\ncommand = 'project'\n",
    )
    .unwrap();
    let output = home
        .command()
        .current_dir(project)
        .args(["--codex", "--dry-run", "--", "-pwork"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("mcp-project-shadow"));
}

#[test]
fn codex_profile_root_markers_cannot_hide_an_ancestor_enabled_override() {
    let home = TestHome::new();
    home.codex_config("[mcp_servers.off]\ncommand = 'server'\nenabled = false\n");
    fs::write(
        home.dir.path().join(".codex/work.config.toml"),
        "project_root_markers = ['.parentmarker']\n",
    )
    .unwrap();
    let parent = home.dir.path().join("parent");
    let project = parent.join("repo");
    fs::create_dir_all(project.join(".git")).unwrap();
    fs::write(parent.join(".parentmarker"), "").unwrap();
    fs::write(project.join(".git/HEAD"), "ref: refs/heads/main").unwrap();
    fs::create_dir_all(parent.join(".codex")).unwrap();
    fs::write(
        parent.join(".codex/config.toml"),
        "[mcp_servers.off]\nenabled = true\n",
    )
    .unwrap();
    let output = home
        .command()
        .current_dir(project)
        .args(["--codex", "--dry-run", "--", "-pwork"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("mcp_servers.off.enabled=false"),
        "{output:?}"
    );
}

#[test]
fn claude_skips_only_proven_plugin_overrides_after_merging_settings() {
    let home = TestHome::new();
    home.config("[plugins.selected]\nclaude = 'selected@m'\n");
    home.claude_installs(r#"{"version":2,"plugins":{"off@m":[{"scope":"user"}],"project@m":[{"scope":"user"}],"local@m":[{"scope":"user"}],"absent@m":[{"scope":"user"}],"selected@m":[{"scope":"user"}]}}"#);
    let custom = home.dir.path().join("custom-claude");
    fs::create_dir_all(&custom).unwrap();
    fs::copy(
        home.dir
            .path()
            .join(".claude/plugins/installed_plugins.json"),
        {
            fs::create_dir_all(custom.join("plugins")).unwrap();
            custom.join("plugins/installed_plugins.json")
        },
    )
    .unwrap();
    fs::write(
        custom.join("settings.json"),
        r#"{"enabledPlugins":{"off@m":false,"project@m":false,"local@m":false,"selected@m":true}}"#,
    )
    .unwrap();
    fs::write(
        home.dir.path().join(".claude/settings.json"),
        r#"{"enabledPlugins":{"project@m":true}}"#,
    )
    .unwrap();
    fs::write(
        home.dir.path().join(".claude/settings.local.json"),
        r#"{"enabledPlugins":{"local@m":true}}"#,
    )
    .unwrap();
    let output = home
        .command()
        .env("CLAUDE_CONFIG_DIR", &custom)
        .args(["--claude", "--plugin", "selected"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings()["enabledPlugins"],
        serde_json::json!({"absent@m":false,"project@m":false,"local@m":false})
    );
    let trace = home
        .command()
        .env("CLAUDE_CONFIG_DIR", &custom)
        .args(["--claude", "--plugin", "selected", "--dry-run"])
        .output()
        .unwrap();
    assert!(trace.status.success(), "{trace:?}");
    let trace = String::from_utf8_lossy(&trace.stderr);
    assert!(
        trace.contains("Plugin off@m: hidden (natively off"),
        "{trace}"
    );
    assert!(
        trace.contains("Plugin selected@m: selected (natively on"),
        "{trace}"
    );
}

#[test]
fn claude_skips_skill_overrides_only_for_exact_native_states() {
    let home = TestHome::new();
    for name in [
        "off",
        "partial",
        "invocable",
        "project",
        "selected",
        "default",
        "unknown",
    ] {
        home.skill(&format!(".claude/skills/{name}"), name);
    }
    home.config("[skills.selected]\nclaude = 'selected'\n[skills.default]\nclaude = 'default'\n[skills.unknown]\nclaude = 'unknown'\n");
    let custom = home.dir.path().join("custom-claude");
    fs::create_dir_all(&custom).unwrap();
    std::os::unix::fs::symlink(
        home.dir.path().join(".claude/skills"),
        custom.join("skills"),
    )
    .unwrap();
    fs::write(custom.join("settings.json"), r#"{"skillOverrides":{"off":"off","partial":"name-only","invocable":"user-invocable-only","project":"off","selected":"off","unknown":null}}"#).unwrap();
    fs::write(
        home.dir.path().join(".claude/settings.json"),
        r#"{"skillOverrides":{"project":"on","selected":"on"}}"#,
    )
    .unwrap();
    let args = [
        "--claude", "--skill", "selected", "--skill", "default", "--skill", "unknown",
    ];
    let output = home
        .command()
        .env("CLAUDE_CONFIG_DIR", &custom)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings()["skillOverrides"],
        serde_json::json!({"partial":"off","invocable":"off","project":"off","unknown":"on"})
    );
    let trace = home
        .command()
        .env("CLAUDE_CONFIG_DIR", &custom)
        .args(args)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(trace.status.success(), "{trace:?}");
    let trace = String::from_utf8_lossy(&trace.stderr);
    for expected in [
        "Skill off: hidden (natively off",
        "Skill selected: selected (natively on",
        "Skill default: selected (natively on",
    ] {
        assert!(trace.contains(expected), "{trace}");
    }
    let output = home.run(&[
        "--claude", "--skill", "default", "--skill", "selected", "--skill", "unknown",
    ]);
    assert!(output.status.success(), "{output:?}");
    // Default-on selected Skills need no entries, while unselected Skills remain hidden.
    assert_eq!(
        home.claude_settings()["skillOverrides"],
        serde_json::json!({"off":"off","partial":"off","invocable":"off","project":"off"})
    );
}

#[test]
fn claude_skips_native_mcp_denies_from_toggles_and_all_settings_layers() {
    let home = TestHome::new();
    let custom = home.dir.path().join("custom-claude");
    fs::create_dir_all(&custom).unwrap();
    fs::create_dir_all(home.dir.path().join(".git")).unwrap();
    fs::create_dir_all(home.dir.path().join(".claude")).unwrap();
    let servers = serde_json::json!({"mcpServers":{"toggle":{"command":"test"},"user":{"command":"test"},"project":{"command":"test"},"local":{"command":"test"},"command":{"command":"test"},"url":{"url":"https://example.com"},"allowed":{"command":"test"}},"projects":{home.dir.path().to_string_lossy().as_ref():{"disabledMcpServers":["toggle"]}}});
    fs::write(custom.join(".claude.json"), servers.to_string()).unwrap();
    fs::write(custom.join("settings.json"), r#"{"deniedMcpServers":[{"serverName":"user"},{"serverCommand":["test"]},{"serverUrl":"https://example.com"}],"allowedMcpServers":[{"serverName":"allowed"}]}"#).unwrap();
    fs::write(
        home.dir.path().join(".claude/settings.json"),
        r#"{"deniedMcpServers":[{"serverName":"project"}]}"#,
    )
    .unwrap();
    fs::write(
        home.dir.path().join(".claude/settings.local.json"),
        r#"{"deniedMcpServers":[{"serverName":"local"}]}"#,
    )
    .unwrap();
    let output = home
        .command()
        .env("CLAUDE_CONFIG_DIR", &custom)
        .arg("--claude")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings()["deniedMcpServers"],
        serde_json::json!([{"serverName":"allowed"},{"serverName":"command"},{"serverName":"url"}])
    );
    let trace = home
        .command()
        .env("CLAUDE_CONFIG_DIR", &custom)
        .args(["--claude", "--dry-run"])
        .output()
        .unwrap();
    assert!(trace.status.success(), "{trace:?}");
    let trace = String::from_utf8_lossy(&trace.stderr);
    for name in ["toggle", "user", "project", "local"] {
        assert!(
            trace.contains(&format!("MCP server {name}: hidden (natively off")),
            "{trace}"
        );
    }
}

#[test]
fn claude_omits_empty_override_keys_and_preserves_overrides_with_extra_settings() {
    let home = TestHome::new();
    home.config(
        "[skills.selected]\nclaude = 'selected'\n[plugins.selected]\nclaude = 'selected@m'\n",
    );
    home.skill(".claude/skills/selected", "selected");
    home.skill(".claude/skills/hidden", "hidden");
    home.claude_installs(r#"{"version":2,"plugins":{"hidden@m":[{"scope":"user"}],"selected@m":[{"scope":"user"}]}}"#);
    fs::write(home.dir.path().join(".claude/settings.json"), r#"{"enabledPlugins":{"hidden@m":false,"selected@m":true},"skillOverrides":{"hidden":"off","selected":"on"},"deniedMcpServers":[{"serverName":"server"}]}"#).unwrap();
    fs::write(
        home.dir.path().join(".claude.json"),
        r#"{"mcpServers":{"server":{"command":"test"}}}"#,
    )
    .unwrap();
    let args = ["--claude", "--skill", "selected", "--plugin", "selected"];
    let output = home.run(&args);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings(),
        serde_json::json!({"disableClaudeAiConnectors":true,"syncClaudeAiSkills":false})
    );
    let full = serde_json::json!({"disableClaudeAiConnectors":true,"syncClaudeAiSkills":false,"enabledPlugins":{"hidden@m":false,"selected@m":true},"skillOverrides":{"hidden":"off","selected":"on"},"deniedMcpServers":[{"serverName":"server"}]});
    for extra in [vec!["--settings", "{}"], vec!["--settings={}"]] {
        let mut passthrough = args.to_vec();
        passthrough.push("--");
        passthrough.extend(extra);
        let output = home.run(&passthrough);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(home.claude_settings(), full);
        assert!(!String::from_utf8_lossy(&output.stderr).contains("natively "));
    }
    home.config("[harnesses.claude]\nargs = ['--settings={}' ]\n[skills.selected]\nclaude = 'selected'\n[plugins.selected]\nclaude = 'selected@m'\n");
    let output = home.run(&args);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(home.claude_settings(), full);
    let mut without_args = args.to_vec();
    without_args.push("--no-harness-args");
    let output = home.run(&without_args);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings(),
        serde_json::json!({"disableClaudeAiConnectors":true,"syncClaudeAiSkills":false})
    );
}

#[test]
fn claude_keeps_overrides_when_native_settings_are_uncertain() {
    let home = TestHome::new();
    home.skill(".claude/skills/selected", "selected");
    home.claude_installs(r#"{"version":2,"plugins":{"hidden@m":[{"scope":"user"}]}}"#);
    home.config("[skills.selected]\nclaude = 'selected'\n");
    let settings = home.dir.path().join(".claude/settings.json");
    for contents in [
        r#"{"enabledPlugins":{"hidden@m":null},"skillOverrides":{"selected":null}}"#,
        r#"{"enabledPlugins":[],"skillOverrides":{"selected":"on"}}"#,
        r#"{"enabledPlugins":{"hidden@m":false},"skillOverrides":[]}"#,
        r#"{"enabledPlugins":{"hidden@m":false},"deniedMcpServers":{}}"#,
    ] {
        fs::write(&settings, contents).unwrap();
        let output = home.run(&["--claude", "--skill", "selected"]);
        assert!(output.status.success(), "{contents}: {output:?}");
        assert_eq!(
            home.claude_settings()["enabledPlugins"],
            serde_json::json!({"hidden@m":false})
        );
        assert_eq!(
            home.claude_settings()["skillOverrides"],
            serde_json::json!({"selected":"on"})
        );
    }
    fs::write(&settings, "{").unwrap();
    let output = home.run(&["--claude", "--dry-run"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("enumeration-failed"));
    fs::remove_file(&settings).unwrap();
    fs::create_dir(&settings).unwrap();
    let output = home.run(&["--claude", "--dry-run"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("enumeration-failed"));
}

#[test]
fn claude_preserves_overrides_when_nested_local_settings_paths_are_uncertain() {
    let home = TestHome::new();
    home.claude_installs(
        r#"{"version":2,"plugins":{"hidden@m":[{"scope":"user"}],"off@m":[{"scope":"user"}]}}"#,
    );
    fs::create_dir_all(home.dir.path().join(".git")).unwrap();
    fs::create_dir_all(home.dir.path().join("nested/.claude")).unwrap();
    fs::write(
        home.dir.path().join(".claude/settings.local.json"),
        r#"{"enabledPlugins":{"hidden@m":true,"off@m":false}}"#,
    )
    .unwrap();
    fs::write(
        home.dir.path().join("nested/.claude/settings.json"),
        r#"{"enabledPlugins":{"hidden@m":false,"off@m":true}}"#,
    )
    .unwrap();
    let output = home
        .command()
        .current_dir(home.dir.path().join("nested"))
        .arg("--claude")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings()["enabledPlugins"],
        serde_json::json!({"hidden@m":false,"off@m":false})
    );
}

#[test]
fn claude_does_not_use_parent_project_settings_as_native_off_proof() {
    let home = TestHome::new();
    home.claude_installs(r#"{"version":2,"plugins":{"hidden@m":[{"scope":"user"}]}}"#);
    home.skill(".claude/skills/selected", "selected");
    home.config("[skills.selected]\nclaude = 'selected'\n");
    fs::create_dir_all(home.dir.path().join(".git")).unwrap();
    fs::create_dir_all(home.dir.path().join("nested")).unwrap();
    fs::write(
        home.dir.path().join(".claude/settings.json"),
        r#"{"enabledPlugins":{"hidden@m":false},"skillOverrides":{"selected":"off"}}"#,
    )
    .unwrap();
    let output = home
        .command()
        .current_dir(home.dir.path().join("nested"))
        .args(["--claude", "--skill", "selected"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    // The user file still applies, even though it is also a parent project file.
    assert!(home.claude_settings().get("enabledPlugins").is_none());
    assert_eq!(
        home.claude_settings()["skillOverrides"],
        serde_json::json!({"selected":"on"})
    );
    let custom = home.dir.path().join("custom-claude");
    fs::create_dir_all(custom.join("plugins")).unwrap();
    fs::copy(
        home.dir
            .path()
            .join(".claude/plugins/installed_plugins.json"),
        custom.join("plugins/installed_plugins.json"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        home.dir.path().join(".claude/skills"),
        custom.join("skills"),
    )
    .unwrap();
    let output = home
        .command()
        .env("CLAUDE_CONFIG_DIR", custom)
        .current_dir(home.dir.path().join("nested"))
        .args(["--claude", "--skill", "selected"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings()["enabledPlugins"],
        serde_json::json!({"hidden@m":false})
    );
    assert!(home.claude_settings().get("skillOverrides").is_none());
}

#[test]
fn claude_preserves_overrides_when_worktree_local_settings_paths_are_uncertain() {
    let home = TestHome::new();
    home.claude_installs(r#"{"version":2,"plugins":{"hidden@m":[{"scope":"user"}]}}"#);
    home.skill(".claude/skills/selected", "selected");
    home.config("[skills.selected]\nclaude = 'selected'\n");
    fs::write(
        home.dir.path().join(".git"),
        "gitdir: /some/main/checkout/.git/worktrees/fixture\n",
    )
    .unwrap();
    fs::write(
        home.dir.path().join(".claude/settings.json"),
        r#"{"enabledPlugins":{"hidden@m":false},"skillOverrides":{"selected":"on"}}"#,
    )
    .unwrap();
    let output = home.run(&["--claude", "--skill", "selected"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        home.claude_settings()["enabledPlugins"],
        serde_json::json!({"hidden@m":false})
    );
    assert_eq!(
        home.claude_settings()["skillOverrides"],
        serde_json::json!({"selected":"on"})
    );
}

#[test]
fn copilot_mcp_natively_off_user_server_omits_hide_and_keeps_trace() {
    let home = TestHome::new();
    let directory = home.dir.path().join(".copilot");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("mcp-config.json"),
        r#"{"mcpServers":{"off":{"command":"test"},"other":{"command":"test"}}}"#,
    )
    .unwrap();
    fs::write(
        directory.join("settings.json"),
        r#"{"disabledMcpServers":["off"]}"#,
    )
    .unwrap();
    let output = home.run(&["--copilot", "--dry-run", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let dry: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let args = dry["argv"].as_array().unwrap();
    assert!(
        !args
            .windows(2)
            .any(|pair| pair[0] == "--disable-mcp-server" && pair[1] == "off")
    );
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--disable-mcp-server" && pair[1] == "other")
    );
    let trace = home.run(&["--copilot", "--dry-run"]);
    assert!(
        String::from_utf8_lossy(&trace.stderr).contains("MCP server off: hidden (natively off")
    );
}

#[test]
fn copilot_mcp_explicit_enable_preserves_hide_for_natively_off_server() {
    for source in [
        "harness",
        "alias",
        "passthrough",
        "equals",
        "no-harness-args",
    ] {
        let home = TestHome::new();
        let directory = home.dir.path().join(".copilot");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("mcp-config.json"),
            r#"{"mcpServers":{"off":{"command":"test"}}}"#,
        )
        .unwrap();
        fs::write(
            directory.join("settings.json"),
            r#"{"disabledMcpServers":["off"]}"#,
        )
        .unwrap();
        let config = match source {
            "harness" | "no-harness-args" => {
                "[harnesses.copilot]\nargs = ['--enable-mcp-server', 'off']\n"
            }
            "alias" => {
                "[aliases.work]\nharness = 'copilot'\nargs = ['--enable-mcp-server', 'off']\n"
            }
            _ => "",
        };
        home.config(config);
        let args = match source {
            "alias" => vec!["--alias", "work", "--dry-run", "--json"],
            "passthrough" => vec![
                "--copilot",
                "--dry-run",
                "--json",
                "--",
                "--enable-mcp-server",
                "off",
            ],
            "equals" => vec![
                "--copilot",
                "--dry-run",
                "--json",
                "--",
                "--enable-mcp-server=off",
            ],
            "no-harness-args" => vec!["--copilot", "--no-harness-args", "--dry-run", "--json"],
            _ => vec!["--copilot", "--dry-run", "--json"],
        };
        let output = home.run(&args);
        assert!(output.status.success(), "{source}: {output:?}");
        let dry: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            dry["argv"]
                .as_array()
                .unwrap()
                .windows(2)
                .any(|pair| pair[0] == "--disable-mcp-server" && pair[1] == "off"),
            source != "no-harness-args",
            "{source}: {dry}"
        );
    }
}

#[test]
fn copilot_mcp_selected_server_enables_only_when_user_or_repo_settings_disable_it() {
    for settings in [
        None,
        Some(".copilot/settings.json"),
        Some("repo/.github/copilot/settings.json"),
        Some("repo/.github/copilot/settings.local.json"),
    ] {
        let home = TestHome::new();
        home.config("[mcp.selected]\ncopilot = 'selected'\n");
        let directory = home.dir.path().join(".copilot");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("mcp-config.json"), r#"{"mcpServers":{"selected":{"command":"test","enabled":false,"disabled":true},"repo_off":{"command":"test"}}}"#).unwrap();
        let repo = home.dir.path().join("repo");
        let nested = repo.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir(repo.join(".git")).unwrap();
        if let Some(relative) = settings {
            let path = home.dir.path().join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, r#"{"disabledMcpServers":["selected","repo_off"]}"#).unwrap();
        }
        let output = home
            .command()
            .current_dir(&nested)
            .args(["--copilot", "--mcp", "selected", "--dry-run", "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{settings:?}: {output:?}");
        let dry: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let args = dry["argv"].as_array().unwrap();
        assert_eq!(
            args.windows(2)
                .any(|pair| pair[0] == "--enable-mcp-server" && pair[1] == "selected"),
            settings.is_some(),
            "{settings:?}: {dry}"
        );
        if settings != Some(".copilot/settings.json") {
            assert!(
                args.windows(2)
                    .any(|pair| pair[0] == "--disable-mcp-server" && pair[1] == "repo_off"),
                "{dry}"
            );
        }
        if settings.is_none() {
            let trace = home
                .command()
                .current_dir(&nested)
                .args(["--copilot", "--mcp", "selected", "--dry-run"])
                .output()
                .unwrap();
            assert!(
                String::from_utf8_lossy(&trace.stderr).contains("MCP server selected: natively on")
            );
        }
    }
}

#[test]
fn command_line_preset_replaces_alias_run_settings() {
    let home = TestHome::new();
    home.config("[presets.luna]\nharness = 'codex'\nmodel = 'gpt-6-luna'\neffort = 'high'\nargs = ['--preset-arg']\nharness_args = false\n[aliases.matt]\nharness = 'claude'\nmodel = 'opus'\nargs = ['--alias-arg']\n[profiles.matt]\n[aliases.matt.disable]\nplugins = []\n");
    let output = home.run(&["--alias", "matt", "--preset", "luna", "--dry-run", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let argv = plan["argv"].as_array().unwrap();
    assert!(argv.iter().any(|arg| arg == "gpt-6-luna"));
    assert!(argv.iter().any(|arg| arg == "--preset-arg"));
    assert!(!argv.iter().any(|arg| arg == "--alias-arg" || arg == "opus"));
    let shorthand = home.run(&["--alias", "matt", "--luna", "--dry-run", "--json"]);
    assert!(shorthand.status.success(), "{shorthand:?}");
}

#[test]
fn preset_resume_reresolves_names_and_rejects_harness_changes() {
    let home = TestHome::new();
    home.config("[presets.opus]\nharness = 'claude'\nmodel = 'opus-old'\n");
    let launch = home.run(&["--opus"]);
    assert!(launch.status.success(), "{launch:?}");
    let record_path = home.session_path();
    let before = fs::read(&record_path).unwrap();
    let record: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(record["request"]["preset"], "opus");
    home.config("[presets.opus]\nharness = 'claude'\nmodel = 'opus-new'\n");
    let resumed = home.run(&["resume", "--last", "--dry-run", "--json"]);
    assert!(resumed.status.success(), "{resumed:?}");
    assert!(String::from_utf8_lossy(&resumed.stdout).contains("opus-new"));
    assert_eq!(fs::read(&record_path).unwrap(), before);
    let listed = home.run(&["list", "sessions"]);
    assert!(String::from_utf8_lossy(&listed.stdout).contains("--preset opus"));
    home.config("[presets.opus]\nharness = 'codex'\n");
    let changed = home.run(&["resume", "--last", "--dry-run"]);
    assert_eq!(changed.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&changed.stderr).contains("preset-harness-changed"));
    let conflicting = home.run(&["resume", "--last", "--opus", "--dry-run"]);
    assert_eq!(conflicting.status.code(), Some(2));
    home.config("");
    let deleted = home.run(&["resume", "--last", "--dry-run"]);
    assert_eq!(deleted.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&deleted.stderr).contains("unknown-preset"));
    assert_eq!(fs::read(&record_path).unwrap(), before);
}

#[test]
fn alias_missing_preset_is_reported_on_resume() {
    let home = TestHome::new();
    home.config("[presets.opus]\nharness = 'claude'\n[aliases.matt]\npreset = 'opus'\n");
    assert!(home.run(&["--alias", "matt"]).status.success());
    home.config("[aliases.matt]\npreset = 'opus'\n");
    let output = home.run(&["resume", "--last", "--dry-run"]);
    assert_eq!(output.status.code(), Some(3));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unknown-preset"),
        "{output:?}"
    );
}

#[test]
fn alias_inherits_preset_and_inline_fields_override_it() {
    let home = TestHome::new();
    home.config("[harnesses.claude]\nargs = ['--harness-arg']\n[presets.opus]\nharness = 'claude'\nmodel = 'opus'\neffort = 'medium'\nargs = ['--preset-arg']\nharness_args = false\n[aliases.matt]\npreset = 'opus'\nprofiles = ['coding']\ndefaults = false\n[profiles.coding]\nplugins = ['optional']\n[plugins.optional]\nall = false\n");
    let inherited = home.run(&["--alias", "matt", "--dry-run"]);
    assert!(inherited.status.success(), "{inherited:?}");
    let trace = String::from_utf8_lossy(&inherited.stderr);
    for expected in [
        "claude (Preset opus from Alias matt)",
        "opus (Preset opus from Alias matt)",
        "medium (Preset opus from Alias matt)",
        "Harness arg: --preset-arg (Preset opus from Alias matt)",
        "Profile coding",
    ] {
        assert!(trace.contains(expected), "missing {expected}: {trace}");
    }
    assert!(!String::from_utf8_lossy(&inherited.stdout).contains("--harness-arg"));
    home.config("[harnesses.claude]\nargs = ['--harness-arg']\n[presets.opus]\nharness = 'claude'\nmodel = 'opus'\neffort = 'medium'\nargs = ['--preset-arg']\nharness_args = false\n[aliases.matt]\npreset = 'opus'\nmodel = 'sonnet'\neffort = 'high'\nargs = []\nharness_args = true\n");
    let inline = home.run(&["--alias", "matt", "--dry-run"]);
    assert!(inline.status.success(), "{inline:?}");
    let trace = String::from_utf8_lossy(&inline.stderr);
    assert!(trace.contains("sonnet (Alias matt)"), "{trace}");
    assert!(trace.contains("high (Alias matt)"), "{trace}");
    let argv = String::from_utf8_lossy(&inline.stdout);
    assert!(argv.contains("--harness-arg"));
    assert!(!argv.contains("--preset-arg"));
    let flags = home.run(&[
        "--alias",
        "matt",
        "--opus",
        "-m",
        "haiku",
        "-e",
        "low",
        "--dry-run",
    ]);
    assert!(flags.status.success(), "{flags:?}");
    let trace = String::from_utf8_lossy(&flags.stderr);
    assert!(trace.contains("haiku (flag)"));
    assert!(trace.contains("low (flag)"));
}

#[test]
fn presets_validate_names_fields_and_user_layer_scope() {
    let home = TestHome::new();
    for (config, code) in [
        ("[presets.'bad.name']\nharness = 'claude'", "invalid-name"),
        (
            "[presets.quiet]\nharness = 'claude'",
            "preset-reserved-name",
        ),
        (
            "[presets.codex]\nharness = 'claude'",
            "preset-reserved-name",
        ),
        ("[presets.fork]\nharness = 'claude'", "preset-reserved-name"),
        ("[presets.opus]\nmodel = 'opus'", "config-invalid"),
        (
            "[presets.opus]\nharness = 'claude'\neffort = 'ultra'",
            "config-invalid",
        ),
        (
            "[presets.opus]\nharness = 'claude'\nplugins = []",
            "config-invalid",
        ),
        (
            "[presets.opus]\nharness = 'claude'\nargs = ['--']",
            "config-invalid",
        ),
        (
            "[presets.opus]\nharness = 'claude'\n[aliases.work]\npreset = 'opus'\nharness = 'codex'",
            "config-invalid",
        ),
    ] {
        home.config(config);
        let output = home.run(&["--claude", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{config}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(code),
            "{config}: {output:?}"
        );
    }
    home.config("");
    for file in ["ayran.toml", "ayran.local.toml"] {
        let path = home.dir.path().join(file);
        fs::write(&path, "[presets.opus]\nharness = 'claude'\n").unwrap();
        let output = home.run(&["--claude", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("user-level-only"));
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn preset_usage_errors_and_unknown_names_have_distinct_exit_codes() {
    let home = TestHome::new();
    home.config("default_harness = 'claude'\n[presets.opus]\nharness = 'claude'\n[presets.luna]\nharness = 'codex'\n[aliases.work]\n");
    for args in [
        vec!["--preset", "opus", "--preset", "luna", "--dry-run"],
        vec!["--opus", "--luna", "--dry-run"],
        vec!["--opus", "--opus", "--dry-run"],
        vec!["--preset", "opus", "--opus", "--dry-run"],
        vec!["--codex", "--opus", "--dry-run"],
    ] {
        let output = home.run(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
    }
    for args in [
        vec!["--preset", "gone", "--dry-run"],
        vec!["--alias", "missing", "--dry-run"],
    ] {
        assert_eq!(home.run(&args).status.code(), Some(3));
    }
    let fallback = home.run(&["--alias", "work", "--dry-run", "--json"]);
    assert!(fallback.status.success(), "{fallback:?}");
    let explicit = home.run(&["--alias", "work", "--codex", "--dry-run", "--json"]);
    assert!(explicit.status.success(), "{explicit:?}");
}

#[test]
fn presets_list_and_complete_on_launch_alias_and_resume() {
    let home = TestHome::new();
    home.config("[presets.opus]\nharness = 'claude'\nmodel = 'opus'\neffort = 'medium'\ndescription = 'Claude coding'\nargs = ['secret-arg']\n[presets.luna]\nharness = 'codex'\nmodel = 'gpt-6-luna'\n[aliases.matt]\npreset = 'opus'\n");
    let listed = home.run(&["list", "presets", "--json", "--harness", "codex"]);
    assert!(listed.status.success(), "{listed:?}");
    let rows: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(rows["presets"].as_array().unwrap().len(), 1);
    assert_eq!(rows["presets"][0]["name"], "luna");
    assert!(!String::from_utf8_lossy(&listed.stdout).contains("secret-arg"));
    let all = home.run(&["list", "--json"]);
    let rows: serde_json::Value = serde_json::from_slice(&all.stdout).unwrap();
    assert_eq!(rows["presets"].as_array().unwrap().len(), 2);
    assert_eq!(rows["aliases"][0]["harness"], "claude");
    assert!(
        String::from_utf8_lossy(&home.run(&["list", "presets"]).stdout).contains("Claude coding")
    );
    for args in [
        vec!["__complete", "--", "ayran", "--preset", ""],
        vec![
            "__complete",
            "--",
            "ayran",
            "--alias",
            "matt",
            "--preset",
            "",
        ],
        vec![
            "__complete",
            "--",
            "ayran",
            "resume",
            "--last",
            "--preset",
            "",
        ],
    ] {
        let output = home.run(&args);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "luna\nopus\n",
            "{args:?}: {output:?}"
        );
    }
    let filtered = home.run(&["__complete", "--", "ayran", "--codex", "--preset", ""]);
    assert_eq!(String::from_utf8_lossy(&filtered.stdout), "luna\n");
    let shorthand = home.run(&["__complete", "--", "ayran", "--alias", "matt", "--l"]);
    assert_eq!(String::from_utf8_lossy(&shorthand.stdout), "--luna\n");
    let models = home.run(&[
        "__complete",
        "--",
        "ayran",
        "--alias",
        "matt",
        "--luna",
        "-m",
        "gpt-6",
    ]);
    assert_eq!(String::from_utf8_lossy(&models.stdout), "gpt-6-luna\n");
    let filtered = home.run(&["__complete", "--", "ayran", "--codex", "--o"]);
    assert!(filtered.stdout.is_empty());
}

#[test]
fn doctor_reports_undefined_alias_presets_and_overlapping_preset_args() {
    let home = TestHome::new();
    home.config("[aliases.work]\npreset = 'gone'\n[presets.opus]\nharness = 'claude'\nargs = ['--model', 'opus']\n");
    let output = home.run(&["doctor", "--json"]);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.iter().any(|d| d["code"] == "unknown-preset"),
        "{output:?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d["code"] == "harness-args-overlap"
                && d["message"].as_str().unwrap().contains("Preset opus")),
        "{output:?}"
    );
}

#[test]
fn switching_preset_keeps_alias_capabilities_disables_and_defaults() {
    let home = TestHome::new();
    home.config("[harnesses.codex]\nmodel = 'harness-model'\neffort = 'low'\nargs = ['--harness-arg']\n[presets.luna]\nharness = 'codex'\nargs = ['--preset-arg']\n[aliases.matt]\nharness = 'claude'\nmodel = 'alias-model'\neffort = 'high'\nargs = ['--alias-arg']\nharness_args = false\nprofiles = ['coding']\ndefaults = false\ndisable = { profiles = ['blocked'] }\n[profiles.coding]\nplugins = ['selected']\nprofiles = ['blocked']\n[profiles.blocked]\nplugins = ['undefined']\n[plugins.selected]\nall = false\n[plugins.unwanted]\ndefault = true\nall = false\n");
    let output = home.run(&[
        "--alias",
        "matt",
        "--luna",
        "--dry-run",
        "--",
        "--typed-arg",
    ]);
    assert!(output.status.success(), "{output:?}");
    let trace = String::from_utf8_lossy(&output.stderr);
    for expected in [
        "harness-model",
        "low",
        "via Profile coding",
        "Profile blocked: disabled by Alias matt",
        "defaults = false",
        "Preset luna from command line",
    ] {
        assert!(trace.contains(expected), "missing {expected}: {trace}");
    }
    let argv = String::from_utf8_lossy(&output.stdout);
    assert!(
        argv.contains("--harness-arg --preset-arg --typed-arg"),
        "{argv}"
    );
    assert!(!argv.contains("alias-model") && !argv.contains("--alias-arg"));
    let suppressed = home.run(&[
        "--alias",
        "matt",
        "--luna",
        "--no-harness-args",
        "--dry-run",
        "--",
        "--typed-arg",
    ]);
    assert!(suppressed.status.success(), "{suppressed:?}");
    let argv = String::from_utf8_lossy(&suppressed.stdout);
    assert!(!argv.contains("--harness-arg") && !argv.contains("--preset-arg"));
    assert!(argv.contains("--typed-arg"));
}

#[test]
fn resume_replacement_preset_is_persisted_and_can_repair_a_deleted_choice() {
    let home = TestHome::new();
    home.config("[presets.opus]\nharness = 'claude'\nmodel = 'opus'\n");
    assert!(home.run(&["--opus"]).status.success());
    home.config("[presets.fast]\nharness = 'claude'\nmodel = 'haiku'\n");
    let dry = home.run(&["resume", "--last", "--preset", "fast", "--dry-run"]);
    assert!(dry.status.success(), "{dry:?}");
    let resumed = home.run(&["resume", "--last", "--fast"]);
    assert!(resumed.status.success(), "{resumed:?}");
    let listed = home.run(&["list", "sessions", "--json"]);
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(listed["sessions"][0]["request"]["preset"], "fast");
    assert!(home.raw_record().contains("haiku"));
}

#[test]
fn presets_cannot_collide_with_any_documented_launch_or_resume_flag() {
    let home = TestHome::new();
    let mut names = std::collections::BTreeSet::new();
    for args in [vec!["--help"], vec!["resume", "--help"]] {
        let help = home.run(&args);
        assert!(help.status.success(), "{help:?}");
        for word in String::from_utf8_lossy(&help.stdout).split_whitespace() {
            if let Some(name) = word.strip_prefix("--") {
                let name = name.trim_end_matches(',');
                if !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                {
                    names.insert(name.to_owned());
                }
            }
        }
    }
    assert!(names.contains("preset") && names.contains("native") && names.contains("quiet"));
    for name in names {
        home.config(&format!("[presets.{name}]\nharness = 'claude'\n"));
        let output = home.run(&["--claude", "--dry-run"]);
        assert_eq!(output.status.code(), Some(3), "{name}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("preset-reserved-name"),
            "{name}: {output:?}"
        );
    }
}

#[test]
fn invalid_preset_config_keeps_its_diagnostic_when_using_shorthand() {
    let home = TestHome::new();
    home.config("[presets.opus]\nharness = 'claude'\neffort = 'ultra'\n");
    let output = home.run(&["--opus", "--dry-run"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("config-invalid"),
        "{output:?}"
    );
}

#[test]
fn invalid_preset_config_does_not_change_unrelated_usage_errors() {
    let home = TestHome::new();
    home.config("[presets.opus]\nharness = 'claude'\neffort = 'ultra'\n");
    let output = home.run(&["--bogus"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("error[usage]"),
        "{output:?}"
    );
}

#[test]
fn builtin_skill_dry_run_materialization_and_resume_use_embedded_tree() {
    for harness in ["--claude", "--copilot"] {
        let home = TestHome::new();
        let before = snapshot(home.dir.path());
        let dry = home.run(&[harness, "--skill", "ayran", "--dry-run"]);
        assert!(dry.status.success(), "{dry:?}");
        let trace = String::from_utf8_lossy(&dry.stderr);
        assert!(trace.contains("builtin ayran, built-in"), "{trace}");
        assert!(trace.contains("not created yet"), "{trace}");
        assert_eq!(snapshot(home.dir.path()), before);
        let launch = home.run(&[harness, "--skill", "ayran"]);
        assert!(launch.status.success(), "{launch:?}");
        let root = home.dir.path().join("cache/ayran/builtin-skills");
        let source = fs::read_dir(&root).unwrap().next().unwrap().unwrap().path();
        assert_eq!(
            fs::read(source.join("SKILL.md")).unwrap(),
            fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.agents/skills/ayran/SKILL.md")
            )
            .unwrap()
        );
        let generated = home.dir.path().join("cache/ayran").join(&harness[2..]);
        let directory = fs::read_dir(generated)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let link = directory.join(if harness == "--claude" {
            ".claude/skills/ayran"
        } else {
            "ayran/skills/ayran"
        });
        assert_eq!(link.canonicalize().unwrap(), source.canonicalize().unwrap());
        let record: serde_json::Value =
            serde_json::from_slice(&fs::read(home.session_path()).unwrap()).unwrap();
        assert_eq!(record["request"]["skills"], serde_json::json!(["ayran"]));
        let resume = home.run(&["resume", "--last", "--dry-run"]);
        assert!(resume.status.success(), "{resume:?}");
        assert!(String::from_utf8_lossy(&resume.stderr).contains("builtin ayran, built-in"));
        assert!(home.run(&[harness, "--skill", "ayran"]).status.success());
        assert_eq!(fs::read_dir(root).unwrap().count(), 1);
    }
}

#[test]
fn builtin_skill_codex_hint_and_whole_table_redefinition() {
    let home = TestHome::new();
    let output = home.run(&["--codex", "--skill", "ayran", "--dry-run"]);
    let errors = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(3));
    assert!(
        errors.contains("unsupported-binding")
            && errors.contains("ayran install --skill ayran --codex"),
        "{errors}"
    );
    home.config("[skills.ayran]\nclaude = false\n");
    let output = home.run(&["--copilot", "--skill", "ayran", "--dry-run"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing-binding"));
    home.config("[skills.ayran]\nall = { builtin = 'ayran' }\ndefault = true\ncodex = false\n");
    let output = home.run(&["--claude", "--dry-run"]);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("Default"));
    let output = home.run(&["--claude", "--no-skill", "ayran", "--dry-run"]);
    assert!(output.status.success(), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("--add-dir"));
}

#[test]
fn path_skill_in_builtin_named_parent_keeps_its_own_contents() {
    let home = TestHome::new();
    let source = home.dir.path().join("config/ayran/builtin-skills/custom");
    fs::create_dir_all(&source).unwrap();
    let text = "---\nname: custom\ndescription: Custom skill\n---\n";
    fs::write(source.join("SKILL.md"), text).unwrap();
    let old = filetime::FileTime::from_unix_time(1000, 0);
    filetime::set_file_mtime(&source, old).unwrap();
    home.config("[skills.custom]\nall = { path = 'builtin-skills/custom' }\n");
    let output = home.run(&["--claude", "--skill", "custom"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(fs::read_to_string(source.join("SKILL.md")).unwrap(), text);
    // Path activation must not touch its source tree's last-use time either.
    assert_eq!(
        filetime::FileTime::from_last_modification_time(&fs::metadata(&source).unwrap()),
        old
    );
}

#[test]
fn builtin_skill_works_through_profiles_aliases_and_other_logical_names() {
    let home = TestHome::new();
    home.config("[skills.manual]\nclaude = { builtin = 'ayran' }\n[profiles.help]\nskills = ['manual']\n[aliases.help]\nharness = 'claude'\nprofiles = ['help']\n");
    for args in [
        vec!["--claude", "--skill", "manual", "--dry-run"],
        vec!["--claude", "--profile", "help", "--dry-run"],
        vec!["--alias", "help", "--dry-run"],
    ] {
        let output = home.run(&args);
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("builtin ayran"));
        assert!(String::from_utf8_lossy(&output.stdout).contains("--add-dir"));
    }
    let output = home.run(&["--alias", "help", "--no-skill", "manual", "--dry-run"]);
    assert!(output.status.success(), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("--add-dir"));
}
