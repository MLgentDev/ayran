//! Persistent user-level Codex Plugin toggles, preserving the surrounding TOML.

use std::{fs, io::Write, path::Path};

use ayran_core::{diagnostic::Diagnostic, harness::Harness};
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, Item, Value};

pub fn write(path: &Path, id: &str, enabled: bool) -> Result<(), Diagnostic> {
    write_with_precommit(path, id, enabled, || {})
}

// The callback is the filesystem boundary used to exercise concurrent edits.
fn write_with_precommit(
    path: &Path,
    id: &str,
    enabled: bool,
    mut before_commit: impl FnMut(),
) -> Result<(), Diagnostic> {
    let failure = |error: &dyn std::fmt::Display| Diagnostic {
        code: "native-write-failed",
        message: format!("cannot write Codex config {}: {error}", path.display()),
        harness: Some(Harness::Codex),
        ..Default::default()
    };
    for attempt in 0..2 {
        let original = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(failure(&error)),
        };
        let mut document = original.parse::<DocumentMut>().map_err(|e| failure(&e))?;
        if let Some(plugins) = document.get("plugins") {
            let plugins = plugins
                .as_table_like()
                .ok_or_else(|| failure(&"plugins must be a table"))?;
            if let Some(plugin) = plugins.get(id) {
                let plugin = plugin
                    .as_table_like()
                    .ok_or_else(|| failure(&"Plugin must be a table"))?;
                if plugin
                    .get("enabled")
                    .is_some_and(|value| value.as_bool().is_none())
                {
                    return Err(failure(&"Plugin enabled must be a boolean"));
                }
            }
        }
        let item = &mut document["plugins"][id]["enabled"];
        let mut value = Value::from(enabled);
        if let Some(old) = item.as_value() {
            *value.decor_mut() = old.decor().clone();
        }
        *item = Item::Value(value);
        let mut temporary =
            tempfile::NamedTempFile::new_in(path.parent().unwrap()).map_err(|e| failure(&e))?;
        if let Ok(metadata) = fs::metadata(path) {
            temporary
                .as_file()
                .set_permissions(metadata.permissions())
                .map_err(|e| failure(&e))?;
        }
        temporary
            .write_all(document.to_string().as_bytes())
            .map_err(|e| failure(&e))?;
        temporary.as_file().sync_all().map_err(|e| failure(&e))?;
        before_commit();
        let current = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(failure(&error)),
        };
        if Sha256::digest(current.as_bytes()) != Sha256::digest(original.as_bytes()) {
            if attempt == 0 {
                continue;
            }
            return Err(failure(
                &"config changed concurrently twice; no toggle was written",
            ));
        }
        temporary.persist(path).map_err(|e| failure(&e))?;
        return Ok(());
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_edit_is_retained_when_the_toggle_is_retried() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("config.toml");
        fs::write(&path, "[plugins.'review@m']\nenabled = true # keep\n").unwrap();
        let mut first = true;
        write_with_precommit(&path, "review@m", false, || {
            if first {
                fs::write(&path, "# concurrent preference\nmodel = 'new'\n[plugins.'review@m']\nenabled = true # keep\n").unwrap();
                first = false;
            }
        }).unwrap_or_else(|d| panic!("{}", d.message));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "# concurrent preference\nmodel = 'new'\n[plugins.'review@m']\nenabled = false # keep\n"
        );
        assert_eq!(fs::read_dir(home.path()).unwrap().count(), 1);
    }
    #[test]
    fn repeated_concurrent_edits_fail_without_replacing_the_latest_config() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("config.toml");
        fs::write(&path, "model = 'first'\n").unwrap();
        let mut revision = 0;
        let result = write_with_precommit(&path, "review@m", false, || {
            revision += 1;
            fs::write(&path, format!("model = 'revision {revision}'\n")).unwrap();
        });
        let error = result.expect_err("a second mismatch must fail");
        assert_eq!(error.code, "native-write-failed");
        assert_eq!(fs::read_to_string(&path).unwrap(), "model = 'revision 2'\n");
        assert_eq!(fs::read_dir(home.path()).unwrap().count(), 1);
    }

    #[test]
    fn malformed_plugin_tables_are_reported_without_panicking_or_writing() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("config.toml");
        for original in [
            "plugins = 42\n",
            "[plugins]\n'review@m' = false\n",
            "[plugins.'review@m']\nenabled = 'yes'\n",
        ] {
            fs::write(&path, original).unwrap();
            let error = write(&path, "review@m", false).expect_err("invalid config must fail");
            assert_eq!(error.code, "native-write-failed");
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
        }
    }
}
