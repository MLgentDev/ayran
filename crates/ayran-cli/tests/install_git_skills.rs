#![cfg(unix)]
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
};
use tempfile::TempDir;

struct Workspace {
    root: TempDir,
    git: PathBuf,
    harness: &'static str,
}
impl Workspace {
    fn new(harness: &'static str) -> Self {
        let git = Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap();
        let w = Self {
            root: tempfile::tempdir().unwrap(),
            git: PathBuf::from(String::from_utf8(git.stdout).unwrap().trim()),
            harness,
        };
        w.write(
            "repo/skills/fixture/SKILL.md",
            "---\nname: snapshot\ndescription: Git fixture\n---\nUse assets.\n",
        );
        w.write("repo/skills/fixture/asset.txt", "first");
        w.git(&["init", "--initial-branch=main"]);
        w.git(&["config", "user.name", "Fixture"]);
        w.git(&["config", "user.email", "fixture@example.com"]);
        w.commit();
        w.git(&["tag", "v1"]);
        w.write(
            ".gitconfig",
            &format!(
                "[url \"file://{}/repo\"]\n insteadOf = https://github.com/fixture/skills.git\n",
                w.root.path().display()
            ),
        );
        w.configure(Some("v1"), false, false);
        let version = match harness {
            "claude" => "2.1.288",
            "codex" => "0.160.0",
            _ => "1.0.91",
        };
        w.script(
            &format!("bin/{harness}"),
            &format!("#!/bin/sh\necho {version}\n"),
        );
        w.script("bin/git", &format!("#!/bin/sh\nprintf '%s %s %s\\n' \"$GIT_TERMINAL_PROMPT\" \"$GIT_SSH_COMMAND\" \"$*\" >> '{}/git-calls'\nexec '{}' \"$@\"\n", w.root.path().display(), w.git.display()));
        w
    }
    fn write(&self, path: &str, text: &str) {
        let path = self.root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn script(&self, path: &str, text: &str) {
        self.write(path, text);
        fs::set_permissions(
            self.root.path().join(path),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    fn git(&self, args: &[&str]) -> String {
        let out = Command::new(&self.git)
            .current_dir(self.root.path().join("repo"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap().trim().into()
    }
    fn commit(&self) -> String {
        self.git(&["add", "."]);
        self.git(&["commit", "-m", "fixture"]);
        self.git(&["rev-parse", "HEAD"])
    }
    fn configure(&self, reference: Option<&str>, isolated: bool, project: bool) {
        let settings = if isolated {
            format!("[harnesses.{}]\nhome = 'isolated'\n", self.harness)
        } else {
            String::new()
        };
        self.write("user.toml", &settings);
        let binding = format!(
            "[skills.logical]\nall = {{ source = 'github:fixture/skills', subdir = 'skills/fixture'{} }}\n",
            reference
                .map(|r| format!(", ref = '{r}'"))
                .unwrap_or_default()
        );
        self.write(
            if project { "ayran.toml" } else { "user.toml" },
            &(if project {
                binding
            } else {
                settings + &binding
            }),
        );
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ayran"))
            .current_dir(self.root.path())
            .args(args)
            .env("HOME", self.root.path())
            .env("USERPROFILE", self.root.path())
            .env("AYRAN_CONFIG", self.root.path().join("user.toml"))
            .env("PATH", self.root.path().join("bin"))
            .env("XDG_STATE_HOME", self.root.path().join("state"))
            .env("XDG_CACHE_HOME", self.root.path().join("cache"))
            .env("GIT_CONFIG_GLOBAL", self.root.path().join(".gitconfig"))
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
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
    fn install(&self, code: i32) -> Value {
        self.json(
            &[
                "install",
                "--skill",
                "logical",
                "--harness",
                self.harness,
                "--json",
            ],
            code,
        )
    }
    fn snapshot(&self, isolated: bool) -> PathBuf {
        self.root.path().join(if isolated {
            format!("state/ayran/homes/{}/skills/snapshot", self.harness)
        } else {
            format!(".{}/skills/snapshot", self.harness)
        })
    }
    fn calls(&self) -> String {
        fs::read_to_string(self.root.path().join("git-calls")).unwrap_or_default()
    }
}
#[test]
fn git_tag_sha_branch_install_in_every_launch_home_and_repeat_unchanged() {
    for harness in ["claude", "codex", "copilot"] {
        for isolated in [false, true] {
            let w = Workspace::new(harness);
            let sha = w.git(&["rev-parse", "HEAD"]);
            for reference in ["v1", sha.as_str(), "main"] {
                w.configure(Some(reference), isolated, false);
                let out = w.install(0);
                assert_eq!(out["skills"][0]["commit"], sha);
                assert_eq!(out["skills"][0]["source"], "github:fixture/skills");
                assert_eq!(out["skills"][0]["subdir"], "skills/fixture");
                assert_eq!(out["skills"][0]["ref"], reference);
                assert_eq!(
                    fs::read_to_string(w.snapshot(isolated).join("asset.txt")).unwrap(),
                    "first"
                );
                assert_eq!(w.install(0)["skills"][0]["copy"], "unchanged");
                let record: Value = serde_json::from_slice(
                    &fs::read(w.snapshot(isolated).join(".ayran-snapshot.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(record["git"]["commit"], sha);
                let before = w.calls();
                let launch = w.run(&["--harness", harness, "--skill", "logical", "--dry-run"]);
                assert!(launch.status.success(), "{launch:?}");
                assert_eq!(w.calls(), before);
                if harness == "copilot" {
                    assert!(out.to_string().contains("skill-install-on"));
                } else {
                    let native = w.json(
                        &["native", "skill", "list", "--harness", harness, "--json"],
                        0,
                    );
                    assert!(native.to_string().contains("off"));
                }
            }
        }
    }
}
#[test]
fn moving_branch_refreshes_even_when_tree_is_identical_and_tag_stays_stable() {
    let w = Workspace::new("codex");
    w.configure(Some("main"), false, false);
    let first = w.install(0)["skills"][0]["commit"].clone();
    w.write("repo/unrelated.txt", "another commit, identical skill");
    let second = w.commit();
    let out = w.install(0);
    assert_eq!(out["skills"][0]["copy"], "replaced");
    assert_ne!(first, second);
    assert_eq!(out["skills"][0]["commit"], second);
    w.write("repo/skills/fixture/asset.txt", "updated");
    w.commit();
    assert_eq!(w.install(0)["skills"][0]["copy"], "replaced");
    w.configure(Some("v1"), false, false);
    w.install(0);
    assert_eq!(
        fs::read_to_string(w.snapshot(false).join("asset.txt")).unwrap(),
        "first"
    );
    assert_eq!(w.install(0)["skills"][0]["copy"], "unchanged");
    w.write(".codex/skills/snapshot/asset.txt", "edited installed copy");
    assert!(w.install(3).to_string().contains("skill-install-conflict"));
    assert_eq!(
        fs::read_to_string(w.snapshot(false).join("asset.txt")).unwrap(),
        "edited installed copy"
    );
}
#[test]
fn project_trust_precedes_fetch_and_commit_movement_keeps_trust() {
    let w = Workspace::new("codex");
    w.configure(Some("main"), false, true);
    assert!(w.install(3).to_string().contains("untrusted-layer"));
    assert!(w.calls().is_empty());
    assert!(!w.root.path().join("cache").exists());
    assert!(w.run(&["trust"]).status.success());
    w.install(0);
    w.write("repo/skills/fixture/asset.txt", "updated");
    w.commit();
    assert_eq!(w.install(0)["skills"][0]["copy"], "replaced");
    let before = w.calls();
    w.configure(Some("v1"), false, true);
    assert!(w.install(3).to_string().contains("untrusted-layer"));
    assert_eq!(w.calls(), before);
    let launch = w.run(&["--codex", "--skill", "logical", "--dry-run"]);
    assert!(launch.status.success(), "{launch:?}");
    assert!(String::from_utf8_lossy(&launch.stderr).contains("skill-outdated"));
    assert_eq!(w.calls(), before);
}
#[test]
fn inspection_and_dry_run_never_fetch_and_doctor_reports_reachability() {
    let w = Workspace::new("codex");
    w.configure(None, false, false);
    let dry = w.json(
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
    assert_eq!(dry["skills"][0]["copy"], "fetch-planned");
    let launch = w.run(&["--codex", "--skill", "logical", "--dry-run"]);
    assert_eq!(launch.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&launch.stderr).contains("skill-not-installed"));
    assert!(!w.root.path().join("cache").exists());
    let doctor = w.json(&["doctor", "--codex", "--json"], 0);
    assert!(doctor.to_string().contains("skill-unpinned"));
    assert!(doctor.to_string().contains("skill-not-installed"));
    assert!(w.calls().is_empty());
    assert!(!w.root.path().join("cache/ayran/git-skills").exists());
    w.install(0);
    let before = w.calls();
    let doctor = w.json(&["doctor", "--codex", "--json"], 0);
    assert!(doctor.to_string().contains("skill-unpinned"));
    assert!(!doctor.to_string().contains("skill-not-installed"));
    assert_eq!(w.calls(), before);
    assert!(
        before
            .lines()
            .all(|l| l.starts_with("0 ssh -o BatchMode=yes "))
    );
}
#[test]
fn invalid_repository_trees_are_refused_and_previous_snapshot_survives() {
    for invalid in ["lfs", "submodule", "symlink"] {
        let w = Workspace::new("codex");
        w.configure(Some("main"), false, false);
        w.install(0);
        match invalid {
            "lfs" => w.write(
                "repo/skills/fixture/asset.txt",
                "version https://git-lfs.github.com/spec/v1\noid sha256:012345\nsize 42\n",
            ),
            "submodule" => {
                let sha = w.git(&["rev-parse", "HEAD"]);
                w.git(&[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("160000,{sha},skills/fixture/sub"),
                ]);
            }
            _ => {
                std::os::unix::fs::symlink(
                    "asset.txt",
                    w.root.path().join("repo/skills/fixture/link"),
                )
                .unwrap();
            }
        }
        if invalid == "submodule" {
            w.git(&["commit", "-m", "submodule"]);
        } else {
            w.commit();
        }
        assert!(w.install(3).to_string().contains("skill-install-invalid"));
        assert_eq!(
            fs::read_to_string(w.snapshot(false).join("asset.txt")).unwrap(),
            "first"
        );
    }
}
#[test]
fn missing_credentials_fail_fast_with_noninteractive_git_and_hint() {
    let w = Workspace::new("codex");
    w.script("bin/git", &format!("#!/bin/sh\nprintf '%s %s\\n' \"$GIT_TERMINAL_PROMPT\" \"$GIT_SSH_COMMAND\" > '{}/prompt-mode'\necho 'credentials unavailable' >&2\nexit 128\n", w.root.path().display()));
    let out = w.install(3);
    assert!(out.to_string().contains("skill-fetch-failed"));
    assert!(out.to_string().contains("credential"));
    assert_eq!(
        fs::read_to_string(w.root.path().join("prompt-mode")).unwrap(),
        "0 ssh -o BatchMode=yes\n"
    );
    let out = w.run(&["install", "--skill", "logical", "--codex"]);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(
        fs::read_to_string(w.root.path().join("prompt-mode")).unwrap(),
        "0 ssh -o BatchMode=yes\n"
    );
    assert!(!w.snapshot(false).exists());
}
#[test]
fn git_binding_rejects_local_sources_unknown_keys_and_escaping_subdirs() {
    let w = Workspace::new("codex");
    for fields in [
        "source = 'file:///tmp/repo'",
        "source = 'git@foo'",
        "source = 'git@../repo'",
        "source = '/tmp/repo'",
        "source = 'github:fixture/skills', path = 'repo'",
        "source = 'github:fixture/skills', typo = 'x'",
        "source = 'github:fixture/skills', subdir = '../outside'",
        "source = 'github:fixture/skills', subdir = '/outside'",
        "source = 'github:fixture/skills', ref = '--upload-pack=x'",
    ] {
        w.write(
            "user.toml",
            &format!("[skills.logical]\nall = {{ {fields} }}\n"),
        );
        assert!(w.install(3).to_string().contains("config-invalid"));
        assert!(w.calls().is_empty());
    }
}

#[test]
fn root_skill_defaults_to_remote_head_and_omits_git_metadata() {
    let w = Workspace::new("claude");
    w.write(
        "repo/SKILL.md",
        "---\nname: root-skill\ndescription: Root skill\n---\n",
    );
    let sha = w.commit();
    w.write(
        "user.toml",
        "[skills.logical]\nclaude = { source = 'github:fixture/skills' }\n",
    );
    let out = w.install(0);
    assert_eq!(out["skills"][0]["id"], "root-skill");
    assert_eq!(out["skills"][0]["commit"], sha);
    assert_eq!(out["skills"][0]["subdir"], ".");
    let root = w.root.path().join(".claude/skills/root-skill");
    assert!(!root.join(".git").exists());
    w.write("repo/extra.txt", "branch moves");
    w.commit();
    assert_eq!(w.install(0)["skills"][0]["copy"], "replaced");
}

#[test]
fn git_completion_and_reachable_doctor_gap_are_read_only() {
    let w = Workspace::new("codex");
    for words in [
        vec![
            "__complete",
            "--",
            "ayran",
            "install",
            "--codex",
            "--skill",
            "l",
        ],
        vec!["__complete", "--", "ayran", "--codex", "--skill", "l"],
    ] {
        let out = w.run(&words);
        assert!(out.status.success(), "{out:?}");
        assert_eq!(String::from_utf8(out.stdout).unwrap(), "logical\n");
    }
    let doctor = w.json(&["doctor", "--codex", "--json"], 0);
    let gap = doctor["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "skill-not-installed")
        .unwrap();
    assert_eq!(gap["severity"], "warning");
    w.write("user.toml", "[skills.logical]\ndefault = true\nall = { source = 'github:fixture/skills', ref = 'v1', subdir = 'skills/fixture' }\n");
    let doctor = w.json(&["doctor", "--codex", "--json"], 1);
    let gap = doctor["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "skill-not-installed")
        .unwrap();
    assert_eq!(gap["severity"], "error");
    assert!(w.calls().is_empty());
}

#[test]
fn git_native_identity_cannot_be_taken_from_another_declared_skill() {
    let w = Workspace::new("codex");
    w.install(0);
    w.write("user.toml", "[skills.logical]\nall = { source = 'github:fixture/skills', ref = 'v1', subdir = 'skills/fixture' }\n[skills.other]\nall = { source = 'github:fixture/skills', ref = 'main', subdir = 'skills/fixture' }\n");
    let out = w.json(&["install", "--skill", "other", "--codex", "--json"], 3);
    assert!(out.to_string().contains("skill-install-conflict"));
    assert_eq!(
        fs::read_to_string(w.snapshot(false).join("asset.txt")).unwrap(),
        "first"
    );
}

#[test]
fn doctor_and_dry_run_preflight_known_git_snapshot_without_fetching() {
    for harness in ["claude", "codex", "copilot"] {
        let w = Workspace::new(harness);
        w.install(0);
        let before = w.calls();
        if harness == "copilot" {
            w.write(
                ".copilot/settings.json",
                "{\"disabledSkills\":[\"snapshot\"]}",
            );
            assert!(
                w.json(
                    &[
                        "install",
                        "--skill",
                        "logical",
                        "--copilot",
                        "--dry-run",
                        "--json"
                    ],
                    3
                )
                .to_string()
                .contains("skill-install-conflict")
            );
            assert!(
                w.json(&["doctor", "--copilot", "--json"], 0)
                    .to_string()
                    .contains("skill-install-conflict")
            );
        } else {
            w.write(
                if harness == "codex" {
                    ".codex/config.toml"
                } else {
                    ".claude/settings.json"
                },
                if harness == "codex" { "" } else { "{}" },
            );
            let doctor = w.json(&["doctor", "--harness", harness, "--json"], 0);
            assert!(doctor.to_string().contains("skill-not-installed"));
            let dry = w.json(
                &[
                    "install",
                    "--skill",
                    "logical",
                    "--harness",
                    harness,
                    "--dry-run",
                    "--json",
                ],
                0,
            );
            assert_eq!(dry["skills"][0]["disable"], "planned");
        }
        assert_eq!(w.calls(), before);
    }
    let w = Workspace::new("codex");
    w.configure(Some("v1"), false, true);
    assert!(
        w.json(&["doctor", "--codex", "--json"], 0)
            .to_string()
            .contains("untrusted-layer")
    );
    assert!(w.calls().is_empty());
    assert!(w.run(&["trust"]).status.success());
    w.install(0);
    let before = w.calls();
    w.write(
        ".codex/skills/other/SKILL.md",
        "---\nname: snapshot\ndescription: Collision\n---\n",
    );
    let dry = w.json(
        &[
            "install",
            "--skill",
            "logical",
            "--codex",
            "--dry-run",
            "--json",
        ],
        3,
    );
    assert!(dry.to_string().contains("skill-install-conflict"));
    assert!(
        w.json(&["doctor", "--codex", "--json"], 0)
            .to_string()
            .contains("skill-install-conflict")
    );
    assert_eq!(w.calls(), before);
}
