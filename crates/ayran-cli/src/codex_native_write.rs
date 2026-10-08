//! Persistent user-level Codex Plugin and MCP toggles, preserving surrounding TOML.

use std::{fs, io::Write, path::Path};

use crate::native_plugins::NativeState;
use ayran_core::{diagnostic::Diagnostic, harness::Harness};
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, Item, Value};

pub fn write(path: &Path, id: &str, enabled: bool) -> Result<(), Diagnostic> {
    write_toggle(path, "plugins", id, Some(enabled), || {})
}

/// Restore a Plugin's original enabled value, including an omitted native default.
pub fn restore_plugin(path: &Path, id: &str, enabled: Option<bool>) -> Result<(), Diagnostic> {
    write_toggle(path, "plugins", id, enabled, || {})
}

pub(crate) fn prepare_mcp(path: &Path, id: &str) -> Result<NativeState, Diagnostic> {
    let original = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(Diagnostic::error(
                "native-write-failed",
                e.to_string(),
                None,
            ));
        }
    };
    let doc = original
        .parse::<DocumentMut>()
        .map_err(|e| Diagnostic::error("native-write-failed", e.to_string(), None))?;
    validate(&doc, "mcp_servers", id).map_err(|e| {
        Diagnostic::error(
            "native-write-failed",
            format!("{}: {e}", path.display()),
            None,
        )
    })
}

pub(crate) fn write_mcp(path: &Path, id: &str, enabled: bool) -> Result<&'static str, Diagnostic> {
    if prepare_mcp(path, id)? == NativeState::from_enabled(enabled) {
        return Ok("unchanged");
    }
    fs::create_dir_all(path.parent().unwrap())
        .map_err(|e| Diagnostic::error("native-write-failed", e.to_string(), None))?;
    write_toggle(path, "mcp_servers", id, Some(enabled), || {})?;
    Ok("changed")
}

fn validate(doc: &DocumentMut, key: &str, id: &str) -> Result<NativeState, String> {
    let Some(items) = doc.get(key) else {
        return Ok(NativeState::On);
    };
    let items = items
        .as_table_like()
        .ok_or_else(|| format!("{key} must be a table"))?;
    let Some(item) = items.get(id) else {
        return Ok(NativeState::On);
    };
    let item = item
        .as_table_like()
        .ok_or_else(|| format!("{key}.{id} must be a table"))?;
    match item.get("enabled") {
        None => Ok(NativeState::On),
        Some(value) => value
            .as_bool()
            .map(NativeState::from_enabled)
            .ok_or_else(|| format!("{key}.{id}.enabled must be a boolean")),
    }
}

#[cfg(test)]
fn write_with_precommit(
    path: &Path,
    id: &str,
    enabled: bool,
    before_commit: impl FnMut(),
) -> Result<(), Diagnostic> {
    write_toggle(path, "plugins", id, Some(enabled), before_commit)
}

// The callback is the filesystem boundary used to exercise concurrent edits.
fn write_toggle(
    path: &Path,
    key: &str,
    id: &str,
    enabled: Option<bool>,
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
        validate(&document, key, id).map_err(|e| failure(&e))?;
        if let Some(enabled) = enabled {
            let item = &mut document[key][id]["enabled"];
            let mut value = Value::from(enabled);
            if let Some(old) = item.as_value() {
                *value.decor_mut() = old.decor().clone();
            }
            *item = Item::Value(value);
        } else if let Some(item) = document
            .get_mut(key)
            .and_then(Item::as_table_like_mut)
            .and_then(|items| items.get_mut(id))
            .and_then(Item::as_table_like_mut)
        {
            item.remove("enabled");
        }
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
