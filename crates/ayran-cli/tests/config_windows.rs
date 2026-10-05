#![cfg(windows)]

use std::process::Command;

#[test]
fn user_config_path_uses_native_separators_before_file_exists() {
    let workspace = tempfile::tempdir().unwrap();
    let appdata = workspace.path().join("AppData").join("Roaming");
    let expected = appdata.join("ayran").join("ayran.toml");
    assert!(!expected.exists());

    let output = Command::new(env!("CARGO_BIN_EXE_ayran"))
        .current_dir(workspace.path())
        .env_remove("AYRAN_CONFIG")
        .env("APPDATA", &appdata)
        .args(["config", "path", "--user"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let path = String::from_utf8(output.stdout).unwrap();
    assert_eq!(path.trim(), expected.to_str().unwrap());
    assert!(!path.contains('/'), "{path}");
}
