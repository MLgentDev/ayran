//! Read user-level Marketplace registrations without invoking mutating Harness commands.
use ayran_core::{
    diagnostic::Diagnostic,
    harness::Harness,
    marketplace::{Definition, Source},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub fn read(harness: Harness, home: &Path) -> Result<BTreeMap<String, Definition>, Diagnostic> {
    let mut result = BTreeMap::new();
    if harness == Harness::Codex {
        let path = home.join("config.toml");
        let Some(text) = text(&path)? else {
            return Ok(result);
        };
        let config: toml::Value = toml::from_str(&text).map_err(|e| failure(&path, e))?;
        if let Some(markets) = config.get("marketplaces") {
            let markets = markets
                .as_table()
                .ok_or_else(|| failure(&path, "marketplaces must be a table"))?;
            for (name, market) in markets {
                let source = market
                    .get("source")
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| failure(&path, "Marketplace source must be a string"))?;
                let kind = market
                    .get("source_type")
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| failure(&path, "Marketplace source_type missing"))?;
                result.insert(
                    name.clone(),
                    Definition {
                        source: source_value(kind, source, &path)?,
                        reference: market
                            .get("ref")
                            .map(|v| {
                                v.as_str()
                                    .map(str::to_owned)
                                    .ok_or_else(|| failure(&path, "ref must be a string"))
                            })
                            .transpose()?,
                        name: None,
                    },
                );
            }
        }
        return Ok(result);
    }
    let path = home.join(if harness == Harness::Claude {
        "plugins/known_marketplaces.json"
    } else {
        "settings.json"
    });
    let Some(text) = text(&path)? else {
        return Ok(result);
    };
    let config: Value = if harness == Harness::Copilot {
        serde_json_lenient::from_str(&text).map_err(|e| failure(&path, e))?
    } else {
        serde_json::from_str(&text).map_err(|e| failure(&path, e))?
    };
    let markets = if harness == Harness::Claude {
        Some(&config)
    } else {
        config.get("extraKnownMarketplaces")
    };
    if let Some(markets) = markets {
        for (name, market) in markets
            .as_object()
            .ok_or_else(|| failure(&path, "Marketplace state must be an object"))?
        {
            let source = market
                .get("source")
                .ok_or_else(|| failure(&path, "Marketplace source missing"))?;
            let kind = source
                .get("source")
                .and_then(Value::as_str)
                .ok_or_else(|| failure(&path, "Marketplace source kind missing"))?;
            let value = source
                .get(match kind {
                    "github" => "repo",
                    "directory" => "path",
                    _ => "url",
                })
                .and_then(Value::as_str)
                .ok_or_else(|| failure(&path, "Marketplace source value missing"))?;
            result.insert(
                name.clone(),
                Definition {
                    source: source_value(kind, value, &path)?,
                    reference: source
                        .get("ref")
                        .map(|v| {
                            v.as_str()
                                .map(str::to_owned)
                                .ok_or_else(|| failure(&path, "ref must be a string"))
                        })
                        .transpose()?,
                    name: None,
                },
            );
        }
    }
    Ok(result)
}
fn source_value(kind: &str, value: &str, path: &Path) -> Result<Source, Diagnostic> {
    match kind {
        "github" => Ok(Source::Github(value.to_owned())),
        "git" => Ok(Source::Git(value.to_owned())),
        "directory" | "local" => Ok(Source::Path(PathBuf::from(value))),
        _ => Err(failure(
            path,
            format!("unsupported native Marketplace source type {kind}"),
        )),
    }
}
pub fn same_source(left: &Definition, right: &Definition) -> bool {
    let equal = match (&left.source, &right.source) {
        (Source::Path(a), Source::Path(b)) => {
            a.canonicalize().unwrap_or_else(|_| a.clone())
                == b.canonicalize().unwrap_or_else(|_| b.clone())
        }
        (a, b) => a == b,
    };
    equal && left.reference == right.reference
}
fn text(path: &Path) -> Result<Option<String>, Diagnostic> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(failure(path, e)),
    }
}
fn failure(path: &Path, error: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error(
        "enumeration-failed",
        format!("{}: {error}", path.display()),
        None,
    )
}

pub fn installed_plugins(
    harness: Harness,
    home: &crate::harness_home::HarnessHome,
) -> Result<std::collections::BTreeSet<String>, Diagnostic> {
    if harness != Harness::Copilot {
        let real_home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from);
        return Ok(crate::plugin_enumeration::read(harness, real_home.as_deref(), home)?.user);
    }
    let mut installed = std::collections::BTreeSet::new();
    for file in ["config.json", "settings.json"] {
        let path = home.directory.join(file);
        let Some(config) = crate::plugin_enumeration::copilot_json(&path)? else {
            continue;
        };
        if let Some(plugins) = config.get("installedPlugins") {
            for plugin in plugins
                .as_array()
                .ok_or_else(|| failure(&path, "installedPlugins must be an array"))?
            {
                let name = plugin
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failure(&path, "installed Plugin name missing"))?;
                if let Some(market) = plugin.get("marketplace").and_then(Value::as_str) {
                    installed.insert(format!("{name}@{market}"));
                }
            }
        }
        if let Some(enabled) = config.get("enabledPlugins") {
            for (id, state) in enabled
                .as_object()
                .ok_or_else(|| failure(&path, "enabledPlugins must be an object"))?
            {
                if !state.is_boolean() {
                    return Err(failure(&path, "enabledPlugins entries must be booleans"));
                }
                if ayran_core::resolve::valid_copilot_native_id(id) {
                    installed.insert(id.clone());
                }
            }
        }
    }
    Ok(installed)
}
