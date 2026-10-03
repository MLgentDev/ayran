#![cfg(unix)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

struct Workspace(TempDir);
impl Workspace {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }
    fn path(&self) -> &Path {
        self.0.path()
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ayran"));
        command
            .current_dir(self.path())
            .env("AYRAN_CONFIG", self.path().join("user.toml"));
        command
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    fn git(&self, args: &[&str]) {
        assert!(
            Command::new("git")
                .current_dir(self.path())
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

#[test]
fn paths_stop_at_git_root_and_local_creation_follows_nearest_project() {
    let workspace = Workspace::new();
    workspace.git(&["init", "-q"]);
    fs::create_dir_all(workspace.path().join("sub/deep")).unwrap();
    fs::write(workspace.path().join("sub/ayran.toml"), "").unwrap();
    for (scope, expected) in [
        ("-p", "sub/ayran.toml"),
        ("-l", "sub/ayran.local.toml"),
        ("-g", "user.toml"),
    ] {
        let output = workspace
            .command()
            .current_dir(workspace.path().join("sub/deep"))
            .args(["config", "path", scope])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            workspace.path().join(expected).to_str().unwrap()
        );
    }
}

#[test]
fn list_reports_invalid_layers_without_loading_a_merged_view() {
    let workspace = Workspace::new();
    fs::write(workspace.path().join("user.toml"), "bad = [").unwrap();
    fs::write(
        workspace.path().join("ayran.toml"),
        "[aliases.x]\nharness = 'codex'",
    )
    .unwrap();
    fs::write(workspace.path().join("ayran.local.toml"), "").unwrap();
    let output = workspace.run(&["config", "ls", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = rows.as_array().unwrap();
    assert_eq!(rows[0]["scope"], "user");
    assert_eq!(rows[0]["valid"], false);
    for (name, scope, valid) in [
        ("ayran.toml", "project", false),
        ("ayran.local.toml", "local", true),
    ] {
        let row = rows
            .iter()
            .find(|row| row["path"] == workspace.path().join(name).to_str().unwrap())
            .unwrap();
        assert_eq!(row["scope"], scope);
        assert_eq!(row["exists"], true);
        assert_eq!(row["valid"], valid);
    }
}

#[test]
fn editor_success_validates_only_the_saved_file_and_notes_alias_names() {
    let workspace = Workspace::new();
    fs::write(workspace.path().join("ayran.toml"), "broken = [").unwrap();
    let editor = workspace.path().join("editor.sh");
    fs::write(
        &editor,
        "printf \"[aliases.work]\\nharness = 'codex'\\n\" > \"$1\"\n",
    )
    .unwrap();
    let output = workspace
        .command()
        .env("VISUAL", format!("sh '{}'", editor.display()))
        .args(["config", "edit", "-u"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("aliases-changed"));
    let output = workspace.run(&["activate", "bash"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("work()"));
}

#[test]
fn scope_usage_lists_paths_and_rejects_conflicting_options() {
    let workspace = Workspace::new();
    for operation in ["edit", "path"] {
        let output = workspace.run(&["config", operation]);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        let message = String::from_utf8_lossy(&output.stderr);
        for (flag, file) in [
            ("--user", "user.toml"),
            ("--project", "ayran.toml"),
            ("--local", "ayran.local.toml"),
        ] {
            assert!(message.contains(flag));
            assert!(message.contains(workspace.path().join(file).to_str().unwrap()));
        }
    }
    for args in [
        vec!["config", "path", "-u", "-p"],
        vec!["config", "edit", "-l", "--file", "custom.toml"],
        vec!["config", "path", "--file", "custom.toml"],
    ] {
        assert_eq!(workspace.run(&args).status.code(), Some(2));
    }
    let help = workspace.run(&["config", "path", "--help"]);
    assert!(!String::from_utf8_lossy(&help.stdout).contains("--global"));
    assert_eq!(
        workspace.run(&["config", "path", "--global"]).status.code(),
        Some(0)
    );
}

#[test]
fn missing_paths_use_git_root_but_outside_git_use_only_cwd() {
    let workspace = Workspace::new();
    fs::write(workspace.path().join("ayran.toml"), "").unwrap();
    let cwd = workspace.path().join("repo/sub");
    fs::create_dir_all(&cwd).unwrap();
    for repo in [false, true] {
        if repo {
            assert!(
                Command::new("git")
                    .current_dir(cwd.parent().unwrap())
                    .args(["init", "-q"])
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        for (scope, name) in [("-p", "ayran.toml"), ("-l", "ayran.local.toml")] {
            let output = workspace
                .command()
                .current_dir(&cwd)
                .args(["config", "path", scope])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            let base = if repo { cwd.parent().unwrap() } else { &cwd };
            assert_eq!(
                String::from_utf8(output.stdout).unwrap().trim(),
                base.join(name).to_str().unwrap()
            );
        }
    }
}

#[test]
fn user_path_uses_environment_override_then_platform_default() {
    let workspace = Workspace::new();
    let output = workspace
        .command()
        .env_remove("AYRAN_CONFIG")
        .env("XDG_CONFIG_HOME", workspace.path().join("xdg"))
        .args(["config", "path", "-u"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        workspace
            .path()
            .join("xdg/ayran/ayran.toml")
            .to_str()
            .unwrap()
    );
    let output = workspace
        .command()
        .env_remove("AYRAN_CONFIG")
        .env_remove("XDG_CONFIG_HOME")
        .env("HOME", workspace.path())
        .args(["config", "path", "-u"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        workspace
            .path()
            .join(".config/ayran/ayran.toml")
            .to_str()
            .unwrap()
    );
}

#[test]
fn untouched_templates_are_removed_on_editor_success_and_failure() {
    let workspace = Workspace::new();
    for scope in ["-u", "-p", "-l"] {
        for (editor, status) in [("true", 0), ("false", 1)] {
            let output = workspace
                .command()
                .env("VISUAL", editor)
                .args(["config", "edit", scope])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(status), "{output:?}");
            let path = workspace.run(&["config", "path", scope]);
            assert!(!Path::new(String::from_utf8_lossy(&path.stdout).trim()).exists());
            if status == 1 {
                assert!(String::from_utf8_lossy(&output.stderr).contains("editor-failed"));
            }
        }
    }
}

#[test]
fn editor_failure_keeps_changed_files_without_validating() {
    let workspace = Workspace::new();
    let editor = workspace.path().join("editor.sh");
    fs::write(&editor, "printf 'broken = [' > \"$1\"\nexit 7\n").unwrap();
    let output = workspace
        .command()
        .env("VISUAL", format!("sh '{}'", editor.display()))
        .args(["config", "edit", "-p"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(message.contains("editor-failed"));
    assert!(!message.contains("config-invalid"));
    assert_eq!(
        fs::read_to_string(workspace.path().join("ayran.toml")).unwrap(),
        "broken = ["
    );
}

#[test]
fn saved_file_errors_preserve_content_and_validate_explicit_file_scope() {
    let workspace = Workspace::new();
    let editor = workspace.path().join("editor.sh");
    fs::write(&editor, "printf '%s' \"$CONTENT\" > \"$1\"\n").unwrap();
    for (name, content, status, code) in [
        ("other.toml", "bad = [", 3, "config-invalid"),
        (
            "other.toml",
            "[aliases.work]\nharness='codex'",
            3,
            "user-level-only",
        ),
        (
            "ayran.local.toml",
            "[harnesses.codex]\nhome='isolated'",
            3,
            "user-level-only",
        ),
        (
            "user.toml",
            "[aliases.'bad.name']\nharness='codex'",
            3,
            "invalid-name",
        ),
        (
            "./user.toml",
            "[aliases.work]\nharness='codex'",
            0,
            "aliases-changed",
        ),
    ] {
        let output = workspace
            .command()
            .env("VISUAL", format!("sh '{}'", editor.display()))
            .env("CONTENT", content)
            .args(["config", "edit", "--file", name])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(status), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(code),
            "{output:?}"
        );
        assert_eq!(
            fs::read_to_string(workspace.path().join(name)).unwrap(),
            content
        );
    }
}

#[test]
fn local_edit_warns_only_when_not_ignored_and_never_changes_gitignore() {
    let workspace = Workspace::new();
    workspace.git(&["init", "-q"]);
    for ignored in [false, true] {
        let ignore = if ignored {
            "ayran.local.toml\n"
        } else {
            "unrelated\n"
        };
        fs::write(workspace.path().join(".gitignore"), ignore).unwrap();
        let output = workspace
            .command()
            .env("VISUAL", "printf '# saved\\n' >")
            .args(["config", "edit", "-l"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).contains("local-not-ignored"),
            !ignored,
            "{output:?}"
        );
        assert_eq!(
            fs::read_to_string(workspace.path().join(".gitignore")).unwrap(),
            ignore
        );
    }
}

#[test]
fn changing_alias_contents_needs_no_note_but_removing_alias_does() {
    let workspace = Workspace::new();
    fs::write(
        workspace.path().join("user.toml"),
        "[aliases.work]\nharness='codex'",
    )
    .unwrap();
    let output = workspace
        .command()
        .env(
            "VISUAL",
            "printf \"[aliases.work]\\nharness='claude'\\n\" >",
        )
        .args(["config", "edit", "-u"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("aliases-changed"));
    let output = workspace
        .command()
        .env("VISUAL", "printf '# removed\\n' >")
        .args(["config", "edit", "-u"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains("aliases-changed"));
}

#[test]
fn editor_receives_filename_as_one_argument_and_visual_takes_priority() {
    let workspace = Workspace::new();
    let name = "space ' $(touch injected).toml";
    let editor = workspace.path().join("editor.sh");
    fs::write(
        &editor,
        "test \"$1\" = --wait || exit 9\ntest \"$#\" = 2 || exit 8\nprintf '# saved\\n' > \"$2\"\n",
    )
    .unwrap();
    let output = workspace
        .command()
        .env("VISUAL", format!("sh '{}' --wait", editor.display()))
        .env("EDITOR", "false")
        .args(["config", "edit", "--file", name])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(!workspace.path().join("injected").exists());
    assert!(workspace.path().join(name).exists());
    let output = workspace
        .command()
        .env_remove("VISUAL")
        .env("EDITOR", "true")
        .args(["config", "edit", "--file", name])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

#[test]
fn json_list_handles_non_utf8_layer_paths_without_panicking() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let workspace = Workspace::new();
    let cwd = workspace
        .path()
        .join(OsString::from_vec(b"project-\xff".to_vec()));
    fs::create_dir(&cwd).unwrap();
    fs::write(cwd.join("ayran.toml"), "").unwrap();
    let output = workspace
        .command()
        .current_dir(cwd)
        .args(["config", "list", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let layers: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        layers
            .as_array()
            .unwrap()
            .iter()
            .any(|layer| layer["scope"] == "project")
    );
}

#[test]
fn git_root_paths_preserve_non_utf8_bytes_and_trailing_spaces() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let workspace = Workspace::new();
    let root = workspace
        .path()
        .join(OsString::from_vec(b"repo-\xff ".to_vec()));
    let cwd = root.join("sub");
    fs::create_dir_all(&cwd).unwrap();
    assert!(
        Command::new("git")
            .current_dir(&root)
            .args(["init", "-q"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let output = workspace
        .command()
        .current_dir(&cwd)
        .args(["config", "edit", "-p"])
        .env("VISUAL", "printf '# saved\\n' >")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        root.join("ayran.toml").exists(),
        "editor must save in the actual git root"
    );
    assert!(!cwd.join("ayran.toml").exists());
}

#[test]
fn list_includes_layers_above_git_root_and_skips_missing_layers_and_duplicate_user() {
    let workspace = Workspace::new();
    let repo = workspace.path().join("repo");
    fs::create_dir(&repo).unwrap();
    assert!(
        Command::new("git")
            .current_dir(&repo)
            .args(["init", "-q"])
            .output()
            .unwrap()
            .status
            .success()
    );
    fs::write(workspace.path().join("ayran.toml"), "").unwrap();
    fs::write(repo.join("ayran.local.toml"), "").unwrap();
    let output = workspace
        .command()
        .current_dir(&repo)
        .args(["config", "list", "--json"])
        .output()
        .unwrap();
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = rows.as_array().unwrap();
    assert_eq!(rows[0]["exists"], false);
    assert_eq!(rows[0]["scope"], "user");
    let above = rows
        .iter()
        .position(|row| row["path"] == workspace.path().join("ayran.toml").to_str().unwrap())
        .unwrap();
    let local = rows
        .iter()
        .position(|row| row["path"] == repo.join("ayran.local.toml").to_str().unwrap())
        .unwrap();
    assert!(above < local);
    assert!(
        !rows
            .iter()
            .any(|row| row["path"] == repo.join("ayran.toml").to_str().unwrap())
    );
    let output = workspace
        .command()
        .current_dir(&repo)
        .env("AYRAN_CONFIG", workspace.path().join("ayran.toml"))
        .args(["config", "list", "--json"])
        .output()
        .unwrap();
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        rows.as_array()
            .unwrap()
            .iter()
            .filter(|row| row["path"] == workspace.path().join("ayran.toml").to_str().unwrap())
            .count(),
        1
    );
    let output = workspace
        .command()
        .current_dir(repo)
        .args(["config", "list"])
        .output()
        .unwrap();
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.starts_with("scope\tpath\texists\tvalid\n"));
    assert!(output.contains("user\t"));
}

#[test]
fn templates_offer_scope_appropriate_examples_and_existing_files_are_preserved() {
    let workspace = Workspace::new();
    let editor = workspace.path().join("capture.sh");
    fs::write(&editor, "cat \"$1\" > \"$CAPTURE\"\n").unwrap();
    for scope in ["-u", "-p", "-l"] {
        let capture = workspace.path().join("captured");
        let output = workspace
            .command()
            .env("VISUAL", format!("sh '{}'", editor.display()))
            .env("CAPTURE", &capture)
            .args(["config", "edit", scope])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let template = fs::read_to_string(capture).unwrap();
        assert!(template.lines().all(|line| line.starts_with('#')));
        assert_eq!(template.contains("[aliases."), scope == "-u");
        assert_eq!(template.contains("home ="), scope == "-u");
        if scope != "-u" {
            let examples: String = template
                .lines()
                .filter_map(|line| line.strip_prefix("# "))
                .filter(|line| line.starts_with('[') || line.contains(" = "))
                .map(|line| format!("{line}\n"))
                .collect();
            assert!(
                toml::from_str::<toml::Value>(&examples).is_ok(),
                "{examples}"
            );
        }
    }
    fs::write(workspace.path().join("ayran.toml"), "# existing\n").unwrap();
    let output = workspace
        .command()
        .env("VISUAL", "true")
        .args(["config", "edit", "-p"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        fs::read_to_string(workspace.path().join("ayran.toml")).unwrap(),
        "# existing\n"
    );
}

#[test]
fn harness_args_are_valid_only_in_private_layers_in_list_and_edit() {
    let workspace = Workspace::new();
    for name in ["user.toml", "ayran.toml", "ayran.local.toml"] {
        fs::write(
            workspace.path().join(name),
            "[harnesses.copilot]\nargs = ['--allow-all']\n",
        )
        .unwrap();
    }
    let output = workspace.run(&["config", "list", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    for row in rows.as_array().unwrap() {
        assert_eq!(row["valid"], row["scope"] != "project", "{row}");
    }
    for (scope, code) in [("-u", 0), ("-l", 0), ("-p", 3)] {
        let output = workspace
            .command()
            .env("VISUAL", "true")
            .args(["config", "edit", scope])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(code), "{output:?}");
        if code == 3 {
            assert!(String::from_utf8_lossy(&output.stderr).contains("private-layer-only"));
        }
    }
}

#[test]
fn private_templates_offer_harness_args() {
    let workspace = Workspace::new();
    for (scope, private) in [("-u", true), ("-l", true), ("-p", false)] {
        let output = workspace
            .command()
            .env("VISUAL", "cat")
            .args(["config", "edit", scope])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let template = String::from_utf8_lossy(&output.stdout);
        assert_eq!(template.contains("# args = ["), private, "{template}");
    }
}
