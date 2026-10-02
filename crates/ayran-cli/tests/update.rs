use std::process::Command;

#[test]
fn update_without_an_install_receipt_explains_manual_update() {
    let home = tempfile::tempdir().unwrap();
    for args in [vec!["update"], vec!["update", "--check"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_ayran"))
            .args(args)
            .current_dir(home.path())
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("LOCALAPPDATA", home.path())
            .env("XDG_CONFIG_HOME", home.path())
            .env("AXOUPDATER_CONFIG_PATH", home.path())
            .env_remove("AXOUPDATER_CONFIG_WORKING_DIR")
            .env_remove("GITHUB_TOKEN")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("install receipt"), "{stderr}");
        assert!(stderr.contains("rerun the installer"), "{stderr}");
        assert!(stderr.contains("cargo install"), "{stderr}");
        assert!(
            stderr.contains("https://github.com/MLgentDev/ayran/releases/latest"),
            "{stderr}"
        );
        assert!(output.stdout.is_empty(), "{output:?}");
    }
}
