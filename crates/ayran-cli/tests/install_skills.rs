#![cfg(unix)]
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::{Command, Output},
};
use tempfile::TempDir;

struct Workspace(TempDir);
impl Workspace {
    fn new(harness: &str) -> Self {
        let w = Self(tempfile::tempdir().unwrap());
        w.write(
            "user.toml",
            &format!("[skills.logical]\n{harness} = {{ path = 'source' }}\n"),
        );
        w.write(
            "source/SKILL.md",
            "---\nname: snapshot\ndescription: Fixture skill\n---\nUse supporting assets.\n",
        );
        w.write("source/scripts/run.sh", "#!/bin/sh\necho fixture\n");
        w.write("source/assets/data.txt", "fixture asset");
        w.write(
            &format!("bin/{harness}"),
            &format!(
                "#!/bin/sh\necho {}\n",
                match harness {
                    "claude" => "2.1.288",
                    "codex" => "0.160.0",
                    _ => "1.0.91",
                }
            ),
        );
        fs::set_permissions(
            w.0.path().join(format!("bin/{harness}")),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        fs::set_permissions(
            w.0.path().join("source/scripts/run.sh"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        w
    }
    fn write(&self, path: &str, text: &str) {
        let path = self.0.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ayran"))
            .current_dir(self.0.path())
            .args(args)
            .env("HOME", self.0.path())
            .env("USERPROFILE", self.0.path())
            .env("AYRAN_CONFIG", self.0.path().join("user.toml"))
            .env("PATH", self.0.path().join("bin"))
            .env("XDG_STATE_HOME", self.0.path().join("state"))
            .env("XDG_CACHE_HOME", self.0.path().join("cache"))
            .env_remove("CODEX_HOME")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("COPILOT_HOME")
            .output()
            .unwrap()
    }
    fn json(&self, args: &[&str], code: i32) -> Value {
        let out = self.run(args);
        assert_eq!(out.status.code(), Some(code), "{out:?}");
        serde_json::from_slice(&out.stdout).unwrap()
    }
}
#[test]
fn codex_installs_complete_snapshot_off_and_retry_is_unchanged() {
    let w = Workspace::new("codex");
    let args = ["install", "--skill", "logical", "--codex", "--json"];
    let out = w.json(&args, 0);
    assert_eq!(out["skills"][0]["outcome"], "installed");
    assert_eq!(out["skills"][0]["id"], "snapshot");
    assert_eq!(
        fs::read_to_string(w.0.path().join(".codex/skills/snapshot/assets/data.txt")).unwrap(),
        "fixture asset"
    );
    assert_eq!(
        fs::metadata(w.0.path().join(".codex/skills/snapshot/scripts/run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0o111
    );
    let listing = w.json(&["native", "skill", "list", "--codex", "--json"], 0);
    assert!(listing.to_string().contains("snapshot"));
    assert!(listing.to_string().contains("off"));
    assert_eq!(w.json(&args, 0)["skills"][0]["outcome"], "unchanged");
}

#[test]
fn codex_selection_uses_verified_snapshot_and_rejects_tampering() {
    let w = Workspace::new("codex");
    let before = w.run(&["--codex", "--skill", "logical", "--dry-run"]);
    assert_eq!(before.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&before.stderr).contains("unsupported-binding"));
    w.json(&["install", "--skill", "logical", "--codex", "--json"], 0);
    let launch = w.run(&["--codex", "--skill", "logical", "--dry-run"]);
    assert!(launch.status.success(), "{launch:?}");
    let text = String::from_utf8_lossy(&launch.stdout);
    assert!(
        text.contains("enabled = true") || text.contains("enabled=true"),
        "{text}"
    );
    assert!(text.contains(".codex/skills/snapshot/SKILL.md"), "{text}");
    assert!(!w.0.path().join("cache").exists());
    w.write(".codex/skills/snapshot/assets/data.txt", "tampered");
    let launch = w.run(&["--codex", "--skill", "logical", "--dry-run"]);
    assert_eq!(launch.status.code(), Some(3), "{launch:?}");
}

#[test]
fn project_trust_covers_canonical_declarations_but_not_future_bytes() {
    let w = Workspace::new("codex");
    w.write("user.toml", "");
    w.write(
        "ayran.toml",
        "[skills.logical]\ncodex = { path = 'source' }\n",
    );
    let args = ["install", "--skill", "logical", "--codex", "--json"];
    assert!(w.json(&args, 3).to_string().contains("untrusted-layer"));
    assert!(!w.0.path().join(".codex").exists());
    assert!(!w.0.path().join("state").exists());
    assert!(w.run(&["trust"]).status.success());
    w.write("ayran.toml", "[skills.logical]\ncodex = { path = 'source' }\ndescription = 'unrelated'\n[skills.other]\nall = false\n");
    w.json(&args, 0);
    w.write("source/assets/data.txt", "changed source bytes");
    assert_eq!(w.json(&args, 0)["skills"][0]["copy"], "replaced");
    let listing = w.run(&["trust", "--list"]);
    assert!(String::from_utf8_lossy(&listing.stdout).contains("true"));
    w.write(
        "other/SKILL.md",
        "---\nname: other\ndescription: Other source\n---\n",
    );
    w.write(
        "ayran.toml",
        "[skills.logical]\ncodex = { path = 'other' }\n",
    );
    assert!(w.json(&args, 3).to_string().contains("untrusted-layer"));
    assert!(!w.0.path().join(".codex/skills/other").exists());
}

#[test]
fn snapshots_use_each_launch_home_and_report_copilot_limitation() {
    for harness in ["claude", "codex", "copilot"] {
        for isolated in [false, true] {
            let w = Workspace::new(harness);
            if isolated {
                w.write("user.toml", &format!("[harnesses.{harness}]\nhome = 'isolated'\n[skills.logical]\n{harness} = {{ path = 'source' }}\n"));
            }
            let flag = format!("--{harness}");
            let args = ["install", "--skill", "logical", &flag, "--json"];
            let out = w.json(&args, 0);
            let root = if isolated {
                format!("state/ayran/homes/{harness}")
            } else {
                format!(".{harness}")
            };
            assert!(
                w.0.path()
                    .join(format!("{root}/skills/snapshot/scripts/run.sh"))
                    .exists()
            );
            let launch = w.run(&[&flag, "--skill", "logical", "--dry-run"]);
            assert!(launch.status.success(), "{launch:?}");
            if harness == "claude" {
                assert!(String::from_utf8_lossy(&launch.stdout).contains("skillOverrides"));
            }
            if harness == "copilot" {
                assert_eq!(out["skills"][0]["disable"], "unsupported");
                assert!(out.to_string().contains("skill-install-on"));
            }
        }
    }
}

#[test]
fn doctor_and_completion_offer_local_snapshot_installation() {
    let w = Workspace::new("codex");
    let doctor = w.json(&["doctor", "--codex", "--json"], 0);
    assert!(
        doctor.to_string().contains("ayran install --skill logical"),
        "{doctor}"
    );
    let out = w.run(&["__complete", "--", "ayran", "install", "--skill", "l"]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("logical"),
        "{out:?}"
    );
    w.json(&["install", "--skill", "logical", "--codex", "--json"], 0);
    let doctor = w.json(&["doctor", "--codex", "--json"], 0);
    assert!(
        !doctor.to_string().contains("unsupported-binding"),
        "{doctor}"
    );
}

#[test]
fn dry_run_and_invalid_batch_create_no_homes_caches_or_trust() {
    let w = Workspace::new("codex");
    w.json(
        &[
            "install",
            "--skill",
            "logical",
            "--codex",
            "--dry-run",
            "--json",
        ],
        0,
    );
    for p in [".codex", "state", "cache"] {
        assert!(!w.0.path().join(p).exists());
    }
    w.write(
        "bad/SKILL.md",
        "---\nname: ../escape\ndescription: invalid\n---\n",
    );
    w.write(
        "user.toml",
        "[skills.logical]\ncodex = { path = 'source' }\n[skills.bad]\ncodex = { path = 'bad' }\n",
    );
    w.json(
        &["install", "--skill", "logical", "bad", "--codex", "--json"],
        3,
    );
    assert!(!w.0.path().join(".codex").exists());
}

#[test]
fn native_binding_verifies_discovery_and_missing_bindings_are_skipped() {
    let w = Workspace::new("codex");
    w.json(&["install", "--skill", "logical", "--codex", "--json"], 0);
    w.write("user.toml", "[skills.native]\ncodex = 'snapshot'\n[skills.absent]\ncodex = false\n[skills.missing]\nclaude = false\n");
    let out = w.json(
        &[
            "install", "--skill", "native", "absent", "missing", "--codex", "--json",
        ],
        0,
    );
    assert_eq!(out["skills"][0]["outcome"], "unchanged");
    assert!(out.to_string().contains("install-skipped"));
    assert!(!w.0.path().join("cache").exists());
}

#[test]
fn source_and_destination_conflicts_preserve_existing_skill_trees() {
    for fixture in ["unowned", "plugin", "symlink", "identity"] {
        let w = Workspace::new("codex");
        match fixture {
            "unowned" => w.write(
                ".codex/skills/snapshot/SKILL.md",
                "---\nname: snapshot\ndescription: Existing\n---\n",
            ),
            "plugin" => w.write("source/.claude-plugin/plugin.json", "{}"),
            "symlink" => {
                std::os::unix::fs::symlink("assets/data.txt", w.0.path().join("source/link"))
                    .unwrap()
            }
            _ => {
                w.write(
                    "other/SKILL.md",
                    "---\nname: snapshot\ndescription: Another source\n---\n",
                );
                w.write("user.toml", "[skills.logical]\ncodex = { path = 'source' }\n[skills.other]\ncodex = { path = 'other' }\n");
            }
        }
        let names = if fixture == "identity" {
            vec!["logical", "other"]
        } else {
            vec!["logical"]
        };
        let mut args = vec!["install", "--skill"];
        args.extend(names);
        args.extend(["--codex", "--json"]);
        w.json(&args, 3);
        if fixture == "unowned" {
            assert!(
                fs::read_to_string(w.0.path().join(".codex/skills/snapshot/SKILL.md"))
                    .unwrap()
                    .contains("Existing")
            );
        } else {
            assert!(!w.0.path().join(".codex").exists());
        }
    }
}

#[test]
fn retry_completes_native_off_after_copy_succeeds_and_state_write_fails() {
    let w = Workspace::new("codex");
    fs::create_dir_all(w.0.path().join(".codex/skills")).unwrap();
    let home = w.0.path().join(".codex");
    fs::set_permissions(&home, fs::Permissions::from_mode(0o555)).unwrap();
    let out = w.json(&["install", "--skill", "logical", "--codex", "--json"], 3);
    fs::set_permissions(&home, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(out["skills"][0]["copy"], "added");
    assert_eq!(out["skills"][0]["disable"], "failed");
    assert!(
        w.0.path()
            .join(".codex/skills/snapshot/assets/data.txt")
            .exists()
    );
    let retry = w.json(&["install", "--skill", "logical", "--codex", "--json"], 0);
    assert_eq!(retry["skills"][0]["copy"], "unchanged");
    assert_eq!(retry["skills"][0]["disable"], "changed");
}

#[test]
fn repeated_install_refuses_a_second_personal_skill_with_same_identity() {
    for harness in ["claude", "copilot"] {
        let w = Workspace::new(harness);
        let flag = format!("--{harness}");
        let args = ["install", "--skill", "logical", &flag, "--json"];
        w.json(&args, 0);
        w.write(
            &format!(".{harness}/skills/alias/SKILL.md"),
            "---\nname: snapshot\ndescription: Conflicting native identity\n---\n",
        );
        assert!(
            w.json(&args, 3)
                .to_string()
                .contains("skill-install-conflict")
        );
    }
}

#[test]
fn copilot_saved_off_rule_for_a_new_name_refuses_install_without_writes() {
    let w = Workspace::new("copilot");
    w.write(
        ".copilot/settings.json",
        r#"{"disabledSkills":["snapshot"]}"#,
    );
    assert!(
        w.json(&["install", "--skill", "logical", "--copilot", "--json"], 3)
            .to_string()
            .contains("skill-install-conflict")
    );
    assert!(!w.0.path().join(".copilot/skills").exists());
}

#[test]
fn installed_snapshot_stays_selectable_after_source_content_changes() {
    let w = Workspace::new("codex");
    w.json(&["install", "--skill", "logical", "--codex", "--json"], 0);
    w.write("source/assets/data.txt", "future source content");
    let launch = w.run(&["--codex", "--skill", "logical", "--dry-run"]);
    assert!(launch.status.success(), "{launch:?}");
    assert_eq!(
        fs::read_to_string(w.0.path().join(".codex/skills/snapshot/assets/data.txt")).unwrap(),
        "fixture asset"
    );
    assert_eq!(
        w.json(&["install", "--skill", "logical", "--codex", "--json"], 0)["skills"][0]["copy"],
        "replaced"
    );
}

#[test]
fn changing_source_native_identity_replaces_the_owned_snapshot() {
    let w = Workspace::new("codex");
    w.json(&["install", "--skill", "logical", "--codex", "--json"], 0);
    w.write(
        "source/SKILL.md",
        "---\nname: renamed\ndescription: Changed identity\n---\n",
    );
    assert_eq!(
        w.json(&["install", "--skill", "logical", "--codex", "--json"], 0)["skills"][0]["copy"],
        "replaced"
    );
    assert!(w.0.path().join(".codex/skills/renamed").exists());
    assert!(!w.0.path().join(".codex/skills/snapshot").exists());
}

#[test]
fn explicit_replacement_preserves_native_state_in_each_launch_home() {
    for harness in ["claude", "codex", "copilot"] {
        for isolated in [false, true] {
            let w = Workspace::new(harness);
            if isolated {
                w.write("user.toml", &format!("[harnesses.{harness}]\nhome = 'isolated'\n[skills.logical]\n{harness} = {{ path = 'source' }}\n"));
            }
            let flag = format!("--{harness}");
            let args = ["install", "--skill", "logical", &flag, "--json"];
            let original = w.json(&args, 0);
            let destination =
                std::path::PathBuf::from(original["skills"][0]["path"].as_str().unwrap());
            let record = fs::read(destination.join(".ayran-snapshot.json")).unwrap();
            w.write("source/assets/data.txt", "updated asset");
            fs::remove_file(w.0.path().join("source/scripts/run.sh")).unwrap();
            let doctor = w.json(&["doctor", &flag, "--json"], 0);
            assert!(
                doctor.to_string().contains("skill-not-installed"),
                "{doctor}"
            );
            let dry = w.json(
                &[
                    "install",
                    "--skill",
                    "logical",
                    &flag,
                    "--json",
                    "--dry-run",
                ],
                0,
            );
            assert_eq!(dry["skills"][0]["copy"], "replace-planned");
            assert_eq!(
                fs::read(destination.join(".ayran-snapshot.json")).unwrap(),
                record
            );
            assert_eq!(
                fs::read_to_string(destination.join("assets/data.txt")).unwrap(),
                "fixture asset"
            );
            let out = w.json(&args, 0);
            assert_eq!(out["skills"][0]["copy"], "replaced");
            assert_eq!(
                fs::read_to_string(destination.join("assets/data.txt")).unwrap(),
                "updated asset"
            );
            assert!(!destination.join("scripts/run.sh").exists());
            if harness == "copilot" {
                assert!(out.to_string().contains("skill-install-on"));
                assert_eq!(out["skills"][0]["disable"], "unsupported");
            } else {
                assert_eq!(out["skills"][0]["disable"], "unchanged");
                let listing = w.json(&["native", "skill", "list", &flag, "--json"], 0);
                assert!(listing.to_string().contains("off"), "{listing}");
            }
            assert_eq!(w.json(&args, 0)["skills"][0]["outcome"], "unchanged");
            let doctor = w.json(&["doctor", &flag, "--json"], 0);
            assert!(
                !doctor.to_string().contains("skill-not-installed"),
                "{doctor}"
            );
        }
    }
}

#[test]
fn replacement_refuses_modified_owned_snapshots_and_native_name_collisions() {
    for fixture in ["modified", "collision"] {
        let w = Workspace::new("codex");
        let args = ["install", "--skill", "logical", "--codex", "--json"];
        w.json(&args, 0);
        if fixture == "modified" {
            w.write(".codex/skills/snapshot/assets/data.txt", "personal edit");
            w.write("source/assets/data.txt", "upstream update");
        } else {
            w.write(
                "source/SKILL.md",
                "---\nname: renamed\ndescription: New identity\n---\n",
            );
            w.write(
                ".agents/skills/other/SKILL.md",
                "---\nname: renamed\ndescription: Collision\n---\n",
            );
        }
        let destination = w.0.path().join(".codex/skills/snapshot/assets/data.txt");
        let before = fs::read(&destination).unwrap();
        assert!(
            w.json(&args, 3)
                .to_string()
                .contains("skill-install-conflict")
        );
        assert_eq!(fs::read(destination).unwrap(), before);
        assert!(!w.0.path().join(".codex/skills/renamed").exists());
    }
}

#[test]
fn replacement_failure_retains_previous_snapshot_and_retry_completes() {
    let w = Workspace::new("codex");
    let args = ["install", "--skill", "logical", "--codex", "--json"];
    w.json(&args, 0);
    w.write("source/assets/data.txt", "updated asset");
    let root = w.0.path().join(".codex/skills");
    let record = fs::read(root.join("snapshot/.ayran-snapshot.json")).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o555)).unwrap();
    let out = w.json(&args, 3);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(out["skills"][0]["copy"], "failed");
    assert_eq!(
        fs::read(root.join("snapshot/.ayran-snapshot.json")).unwrap(),
        record
    );
    assert_eq!(
        fs::read_to_string(root.join("snapshot/assets/data.txt")).unwrap(),
        "fixture asset"
    );
    assert_eq!(w.json(&args, 0)["skills"][0]["copy"], "replaced");
}

#[test]
fn replacement_accepts_a_changed_source_declaration_and_reports_text_steps() {
    let w = Workspace::new("codex");
    let args = ["install", "--skill", "logical", "--codex", "--json"];
    w.json(&args, 0);
    w.write(
        "other/SKILL.md",
        "---\nname: snapshot\ndescription: New source\n---\n",
    );
    w.write("other/new.txt", "new declaration");
    w.write(
        "user.toml",
        "[skills.logical]\ncodex = { path = 'other' }\n",
    );
    let dry = w.run(&["install", "--skill", "logical", "--codex", "--dry-run"]);
    assert!(dry.status.success(), "{dry:?}");
    assert!(String::from_utf8_lossy(&dry.stdout).contains("copy: replace-planned"));
    let out = w.run(&["install", "--skill", "logical", "--codex"]);
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("copy: replaced"));
    assert!(!w.0.path().join(".codex/skills/snapshot/assets").exists());
    assert_eq!(
        fs::read_to_string(w.0.path().join(".codex/skills/snapshot/new.txt")).unwrap(),
        "new declaration"
    );
    assert_eq!(w.json(&args, 0)["skills"][0]["copy"], "unchanged");
}

#[test]
fn another_declared_skill_cannot_take_over_an_owned_native_identity() {
    let w = Workspace::new("codex");
    w.json(&["install", "--skill", "logical", "--codex", "--json"], 0);
    w.write(
        "other/SKILL.md",
        "---\nname: snapshot\ndescription: Different declared Skill\n---\n",
    );
    w.write("user.toml", "[skills.logical]\ncodex = { path = 'source' }\n[skills.other]\ncodex = { path = 'other' }\n");
    let out = w.json(&["install", "--skill", "other", "--codex", "--json"], 3);
    assert!(out.to_string().contains("skill-install-conflict"));
    assert_eq!(
        fs::read_to_string(w.0.path().join(".codex/skills/snapshot/assets/data.txt")).unwrap(),
        "fixture asset"
    );
}

#[test]
fn builtin_codex_install_has_stable_identity_and_enables_selection() {
    let w = Workspace::new("codex");
    w.write("user.toml", "");
    let args = ["install", "--skill", "ayran", "--codex", "--json"];
    let preview = w.json(
        &[
            "install",
            "--skill",
            "ayran",
            "--codex",
            "--dry-run",
            "--json",
        ],
        0,
    );
    assert_eq!(preview["skills"][0]["source"], "builtin:ayran");
    assert_eq!(preview["skills"][0]["copy"], "planned");
    assert!(!w.0.path().join("cache").exists());
    assert!(!w.0.path().join(".codex").exists());
    let out = w.json(&args, 0);
    assert_eq!(out["skills"][0]["source"], "builtin:ayran");
    assert_eq!(out["skills"][0]["copy"], "added");
    let destination = w.0.path().join(".codex/skills/ayran");
    let record: Value =
        serde_json::from_slice(&fs::read(destination.join(".ayran-snapshot.json")).unwrap())
            .unwrap();
    assert_eq!(record["source"], "builtin:ayran");
    assert!(destination.join("references/install.md").is_file());
    assert!(destination.join("agents/openai.yaml").is_file());
    let listing = w.json(&["native", "skill", "list", "--codex", "--json"], 0);
    assert!(listing.to_string().contains("off"));
    let launch = w.run(&["--codex", "--skill", "ayran", "--dry-run"]);
    assert!(launch.status.success(), "{launch:?}");
    assert!(String::from_utf8_lossy(&launch.stdout).contains(".codex/skills/ayran/SKILL.md"));
    assert!(!String::from_utf8_lossy(&launch.stderr).contains("skill-outdated"));
    assert_eq!(w.json(&args, 0)["skills"][0]["copy"], "unchanged");
}

#[test]
fn builtin_old_content_stays_usable_and_reinstall_replaces_it() {
    let w = Workspace::new("codex");
    // An independently installed fixture represents a previous binary's tree.
    w.write(
        "source/SKILL.md",
        "---\nname: ayran\ndescription: Older built-in manual\n---\nOld manual.\n",
    );
    w.json(&["install", "--skill", "logical", "--codex", "--json"], 0);
    let record_path = w.0.path().join(".codex/skills/ayran/.ayran-snapshot.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record["source"] = "builtin:ayran".into();
    fs::write(&record_path, serde_json::to_vec(&record).unwrap()).unwrap();
    w.write("user.toml", "");
    let optional = w.json(&["doctor", "--codex", "--json"], 0);
    assert!(
        !optional.to_string().contains("skill-outdated"),
        "{optional}"
    );
    w.write(
        "user.toml",
        "[skills.ayran]\nall = { builtin = 'ayran' }\ndefault = true\n",
    );
    let launch = w.run(&["--codex", "--skill", "ayran", "--dry-run"]);
    assert!(launch.status.success(), "{launch:?}");
    assert!(
        String::from_utf8_lossy(&launch.stderr).contains("skill-outdated"),
        "{launch:?}"
    );
    let doctor = w.json(&["doctor", "--codex", "--json"], 0);
    assert!(doctor.to_string().contains("skill-outdated"), "{doctor}");
    assert!(
        doctor
            .to_string()
            .contains("ayran install --skill ayran --codex")
    );
    assert!(!w.0.path().join("cache/ayran/builtin-skills").exists());
    let args = ["install", "--skill", "ayran", "--codex", "--json"];
    assert_eq!(w.json(&args, 0)["skills"][0]["copy"], "replaced");
    assert_eq!(w.json(&args, 0)["skills"][0]["copy"], "unchanged");
    let doctor = w.json(&["doctor", "--codex", "--json"], 0);
    assert!(!doctor.to_string().contains("skill-outdated"), "{doctor}");
    w.write(".codex/skills/ayran/SKILL.md", "modified by the user");
    assert!(
        w.json(&args, 3)
            .to_string()
            .contains("skill-install-conflict")
    );
    assert_eq!(
        fs::read_to_string(w.0.path().join(".codex/skills/ayran/SKILL.md")).unwrap(),
        "modified by the user"
    );
}

#[test]
fn builtin_install_on_all_harnesses_needs_no_project_trust() {
    for harness in ["claude", "codex", "copilot"] {
        let w = Workspace::new(harness);
        w.write("user.toml", "");
        w.write(
            "ayran.toml",
            "[skills.manual]\nall = { builtin = 'ayran' }\n",
        );
        let flag = format!("--{harness}");
        let args = ["install", "--skill", "manual", &flag, "--json"];
        let result = w.json(&args, 0);
        assert_eq!(result["skills"][0]["source"], "builtin:ayran");
        assert_eq!(result["skills"][0]["copy"], "added");
        assert!(!w.0.path().join("state").exists());
        if harness == "copilot" {
            assert_eq!(result["skills"][0]["disable"], "unsupported");
            assert!(result.to_string().contains("skill-install-on"));
        } else {
            assert_eq!(result["skills"][0]["disable"], "changed");
        }
        let text = w.run(&["install", "--skill", "manual", &flag]);
        assert!(text.status.success(), "{text:?}");
        assert!(String::from_utf8_lossy(&text.stdout).contains("builtin:ayran"));
        assert_eq!(w.json(&args, 0)["skills"][0]["copy"], "unchanged");
        let launch = w.run(&[&flag, "--skill", "manual", "--dry-run"]);
        assert!(launch.status.success(), "{launch:?}");
    }
}

#[test]
fn doctor_advises_install_for_reachable_builtin_without_writes() {
    let w = Workspace::new("codex");
    w.write("user.toml", "");
    let doctor = w.json(&["doctor", "--codex", "--json"], 0);
    assert!(
        !doctor.to_string().contains("skill-not-installed"),
        "{doctor}"
    );
    w.write(
        "user.toml",
        "[skills.ayran]\nall = { builtin = 'ayran' }\ndefault = true\n",
    );
    let doctor = w.json(&["doctor", "--codex", "--json"], 1);
    assert!(
        doctor.to_string().contains("skill-not-installed"),
        "{doctor}"
    );
    assert!(!w.0.path().join(".codex").exists());
    assert!(!w.0.path().join("cache/ayran/builtin-skills").exists());
}
