//! Persistent user-level toggles for verified standalone Skill identities.
use std::{fs, io::Write, path::Path};

use ayran_core::{diagnostic::Diagnostic, harness::Harness};
use toml_edit::{DocumentMut, Item, Table, Value};

use crate::native_plugins::NativeState;

fn failure(harness: Harness, path: &Path, error: impl std::fmt::Display) -> Diagnostic {
    Diagnostic {
        code: "native-write-failed",
        message: format!(
            "cannot write {} Skill state {}: {error}",
            harness.binary(),
            path.display()
        ),
        harness: Some(harness),
        ..Default::default()
    }
}

fn contents(harness: Harness, path: &Path) -> Result<String, Diagnostic> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(failure(harness, path, e)),
    }
}

/// Validate and preview without creating a home or changing any file.
pub(crate) fn prepare(
    harness: Harness,
    config: &Path,
    name: &str,
    skill: Option<&Path>,
    enabled: bool,
) -> Result<NativeState, Diagnostic> {
    edit(
        harness,
        config,
        &contents(harness, config)?,
        name,
        skill,
        enabled,
    )
    .map(|(state, _)| state)
}

/// Call only after discovery has verified the Skill's native name and file identity.
pub(crate) fn write(
    harness: Harness,
    config: &Path,
    name: &str,
    skill: Option<&Path>,
    enabled: bool,
) -> Result<&'static str, Diagnostic> {
    for attempt in 0..2 {
        let original = contents(harness, config)?;
        let (before, edited) = edit(harness, config, &original, name, skill, enabled)?;
        if before == NativeState::from_enabled(enabled) {
            return Ok("unchanged");
        }
        fs::create_dir_all(config.parent().unwrap()).map_err(|e| failure(harness, config, e))?;
        let mut temp = tempfile::NamedTempFile::new_in(config.parent().unwrap())
            .map_err(|e| failure(harness, config, e))?;
        if let Ok(metadata) = fs::metadata(config) {
            temp.as_file()
                .set_permissions(metadata.permissions())
                .map_err(|e| failure(harness, config, e))?;
        }
        temp.write_all(edited.as_bytes())
            .map_err(|e| failure(harness, config, e))?;
        temp.as_file()
            .sync_all()
            .map_err(|e| failure(harness, config, e))?;
        if contents(harness, config)? != original {
            if attempt == 0 {
                continue;
            }
            return Err(failure(
                harness,
                config,
                "config changed concurrently twice; no toggle was written",
            ));
        }
        temp.persist(config)
            .map_err(|e| failure(harness, config, e))?;
        return Ok("changed");
    }
    unreachable!()
}

fn edit(
    harness: Harness,
    config: &Path,
    original: &str,
    name: &str,
    skill: Option<&Path>,
    enabled: bool,
) -> Result<(NativeState, String), Diagnostic> {
    let invalid = |message| failure(harness, config, message);
    match harness {
        Harness::Claude => {
            let mut json: serde_json::Value = if original.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(original).map_err(|e| failure(harness, config, e))?
            };
            let root = json
                .as_object_mut()
                .ok_or_else(|| invalid("settings must be an object"))?;
            let overrides = root
                .entry("skillOverrides")
                .or_insert_with(|| serde_json::json!({}))
                .as_object_mut()
                .ok_or_else(|| invalid("skillOverrides must be an object"))?;
            let before = match overrides.get(name).and_then(serde_json::Value::as_str) {
                Some("off") => NativeState::Off,
                Some("on") => NativeState::On,
                None if !overrides.contains_key(name) => NativeState::On,
                Some("name-only" | "user-invocable-only") => NativeState::Unknown,
                _ => return Err(invalid("Skill override has an invalid visibility mode")),
            };
            overrides.insert(
                name.into(),
                serde_json::json!(if enabled { "on" } else { "off" }),
            );
            Ok((
                before,
                format!("{}\n", serde_json::to_string_pretty(&json).unwrap()),
            ))
        }
        Harness::Codex => {
            let skill =
                skill.ok_or_else(|| invalid("Codex Skill needs a verified SKILL.md path"))?;
            let skill_text = skill
                .to_str()
                .ok_or_else(|| invalid("Skill path is not UTF-8"))?;
            let parsed: toml::Table = original.parse().map_err(|e| failure(harness, config, e))?;
            let mut before = NativeState::On;
            let mut last = None;
            if let Some(skills) = parsed.get("skills") {
                let skills = skills
                    .as_table()
                    .ok_or_else(|| invalid("skills must be a table"))?;
                if let Some(rules) = skills.get("config") {
                    let rules = rules
                        .as_array()
                        .ok_or_else(|| invalid("skills.config must be an array"))?;
                    for (index, rule) in rules.iter().enumerate() {
                        let rule = rule
                            .as_table()
                            .ok_or_else(|| invalid("Skill rule must be a table"))?;
                        let selector = |key| -> Result<Option<&str>, Diagnostic> {
                            rule.get(key)
                                .map(|v| {
                                    v.as_str()
                                        .ok_or_else(|| invalid("Skill selector must be a string"))
                                })
                                .transpose()
                        };
                        let path = selector("path")?;
                        let rule_name = selector("name")?.map(str::trim);
                        if path.is_some() && rule_name.is_some() || rule_name == Some("") {
                            continue;
                        }
                        if path.is_none() && rule_name.is_none() {
                            return Err(invalid("Skill rule needs path or name"));
                        }
                        let on = rule
                            .get("enabled")
                            .and_then(toml::Value::as_bool)
                            .ok_or_else(|| invalid("Skill enabled must be a boolean"))?;
                        let exact = path.is_some_and(|p| {
                            crate::skill_enumeration::same_root(Path::new(p), skill)
                        });
                        if exact || rule_name == Some(name) {
                            before = NativeState::from_enabled(on);
                            last = exact.then_some(index);
                        }
                    }
                }
            }
            let mut doc = original
                .parse::<DocumentMut>()
                .map_err(|e| failure(harness, config, e))?;
            if doc.get("skills").is_none() {
                doc["skills"] = Item::Table(Table::new());
            }
            if doc["skills"].get("config").is_none() {
                doc["skills"]["config"] = Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
            }
            let rules = &mut doc["skills"]["config"];
            if let Some(index) = last {
                let item = &mut rules[index]["enabled"];
                let mut value = Value::from(enabled);
                if let Some(old) = item.as_value() {
                    *value.decor_mut() = old.decor().clone();
                }
                *item = Item::Value(value);
            } else if let Some(array) = rules.as_array_of_tables_mut() {
                let mut rule = Table::new();
                rule["path"] = toml_edit::value(skill_text);
                rule["enabled"] = toml_edit::value(enabled);
                array.push(rule);
            } else if let Some(array) = rules.as_array_mut() {
                let mut rule = toml_edit::InlineTable::new();
                rule.insert("path", Value::from(skill_text));
                rule.insert("enabled", Value::from(enabled));
                array.push(Value::InlineTable(rule));
            } else {
                return Err(invalid("skills.config must be an array"));
            }
            Ok((before, doc.to_string()))
        }
        Harness::Copilot => Err(Diagnostic::error(
            "native-unsupported",
            "Copilot Skill selection cannot override native off",
            None,
        )),
    }
}
