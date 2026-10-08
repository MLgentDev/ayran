use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct TestHome(tempfile::TempDir);

impl TestHome {
    fn new() -> Self {
        let home = Self(tempfile::tempdir().unwrap());
        let bin = home.0.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        for harness in ["claude", "copilot"] {
            #[cfg(windows)]
            fs::write(
                bin.join(format!("{harness}.cmd")),
                "@echo off\r\necho 9.0.0\r\n",
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let path = bin.join(harness);
                fs::write(&path, "#!/bin/sh\necho 9.0.0\n").unwrap();
                fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        fs::create_dir_all(home.0.path().join("lint")).unwrap();
        fs::write(
            home.0.path().join("lint/SKILL.md"),
            "---\nname: lint\ndescription: Fixture Skill\n---\nInstructions\n",
        )
        .unwrap();
        // A local layer avoids project-source Trust in these cache tests.
        fs::write(
            home.0.path().join("ayran.local.toml"),
            "[skills.lint]\nall = { path = 'lint' }\n[mcp.extra]\nall = { command = 'server' }\n",
        )
        .unwrap();
        home
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ayran"));
        command
            .current_dir(self.0.path())
            .env("PATH", self.0.path().join("bin"))
            .env("PATHEXT", ".EXE;.CMD")
            .env("USERPROFILE", self.0.path())
            .env("HOME", self.0.path())
            .env("XDG_CONFIG_HOME", self.0.path().join("config"))
            .env("XDG_STATE_HOME", self.0.path().join("state"))
            .env("XDG_CACHE_HOME", self.0.path().join("cache"))
            .env("LOCALAPPDATA", self.0.path().join("local"))
            .env("CLAUDE_CODE_DISABLE_POLICY_SKILLS", "1")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("COPILOT_HOME");
        #[cfg(windows)]
        command.env_remove("HOME");
        command
    }

    fn cache_root(&self) -> PathBuf {
        self.0.path().join(if cfg!(windows) {
            "local/ayran"
        } else {
            "cache/ayran"
        })
    }
}

#[test]
fn dry_run_places_path_skills_and_mcp_definitions_under_the_platform_cache_root() {
    let home = TestHome::new();
    for (harness, flag) in [("claude", "--claude"), ("copilot", "--copilot")] {
        let output = home
            .command()
            .args([
                flag,
                "--skill",
                "lint",
                "--mcp",
                "extra",
                "--dry-run",
                "--json",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let args: Vec<_> = plan["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect();
        let skill_flag = if harness == "claude" {
            "--add-dir"
        } else {
            "--plugin-dir"
        };
        let skill_index = args.iter().position(|arg| *arg == skill_flag).unwrap();
        let mut skill_directory = PathBuf::from(args[skill_index + 1]);
        if harness == "copilot" {
            assert_eq!(skill_directory.file_name().unwrap(), "ayran");
            skill_directory.pop();
        }
        assert_eq!(
            skill_directory.parent().unwrap(),
            home.cache_root().join(harness)
        );
        let mcp_flag = if harness == "claude" {
            "--mcp-config"
        } else {
            "--additional-mcp-config"
        };
        let mcp_index = args.iter().position(|arg| *arg == mcp_flag).unwrap();
        let mcp_file = PathBuf::from(args[mcp_index + 1].trim_start_matches('@'));
        assert_eq!(mcp_file.file_name().unwrap(), "mcp.json");
        assert_eq!(
            mcp_file.parent().unwrap().parent().unwrap(),
            home.cache_root().join(harness)
        );
        assert!(
            !home.cache_root().exists(),
            "dry-run must not create the cache"
        );
    }
}

#[cfg(windows)]
#[test]
fn windows_cache_ignores_home_and_xdg_cache_home() {
    let home = TestHome::new();
    let output = home
        .command()
        .env("HOME", home.0.path())
        .args(["--claude", "--skill", "lint", "--dry-run", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let args = plan["argv"].as_array().unwrap();
    let index = args.iter().position(|arg| arg == "--add-dir").unwrap();
    let directory = PathBuf::from(args[index + 1].as_str().unwrap());
    assert_eq!(
        directory.parent().unwrap(),
        home.cache_root().join("claude")
    );
}

#[test]
fn missing_cache_root_diagnostic_names_the_platform_variables() {
    let home = TestHome::new();
    for (selection, name, cache) in [("--skill", "lint", "Skill"), ("--mcp", "extra", "MCP")] {
        let mut command = home.command();
        #[cfg(windows)]
        command
            .env_remove("LOCALAPPDATA")
            .env("HOME", home.0.path());
        #[cfg(not(windows))]
        command.env_remove("HOME").env_remove("XDG_CACHE_HOME");
        let output = command
            .args(["--claude", selection, name, "--dry-run"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        let variables = if cfg!(windows) {
            "LOCALAPPDATA"
        } else {
            "XDG_CACHE_HOME or HOME"
        };
        assert!(
            stderr.contains(&format!("cannot locate {cache} cache without {variables}")),
            "{stderr}"
        );
        #[cfg(windows)]
        assert!(!stderr.contains("XDG_CACHE_HOME or HOME"), "{stderr}");
    }
}

#[test]
fn launch_prunes_the_platform_cache_root_and_preserves_git_staging() {
    let home = TestHome::new();
    for harness in ["claude", "codex", "copilot"] {
        let old = home.cache_root().join(harness).join("old");
        fs::create_dir_all(&old).unwrap();
        filetime::set_file_mtime(old, filetime::FileTime::from_unix_time(1, 0)).unwrap();
        fs::create_dir_all(home.cache_root().join(harness).join("recent")).unwrap();
    }
    let git_staging = home.cache_root().join("git-skills");
    fs::create_dir_all(&git_staging).unwrap();
    filetime::set_file_mtime(&git_staging, filetime::FileTime::from_unix_time(1, 0)).unwrap();
    let output = home.command().args(["--claude"]).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    for harness in ["claude", "codex", "copilot"] {
        assert!(!home.cache_root().join(harness).join("old").exists());
        assert!(home.cache_root().join(harness).join("recent").exists());
    }
    assert!(git_staging.exists());
}

#[test]
fn builtin_skill_dry_run_uses_platform_cache_without_creating_it() {
    let home = TestHome::new();
    for flag in ["--claude", "--copilot"] {
        let output = home
            .command()
            .args([flag, "--skill", "ayran", "--dry-run"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let trace = String::from_utf8_lossy(&output.stderr);
        assert!(trace.contains("builtin ayran, built-in"), "{trace}");
        assert!(
            trace.contains(home.cache_root().join("builtin-skills").to_str().unwrap()),
            "{trace}"
        );
        assert!(trace.contains("not created yet"), "{trace}");
        assert!(!home.cache_root().exists());
    }
}
