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

pub struct Registration {
    pub definition: Option<Definition>,
    pub source: String,
    pub reference: Option<String>,
}

pub fn read(harness: Harness, home: &Path) -> Result<BTreeMap<String, Definition>, Diagnostic> {
    read_registrations(harness, home, false).map(|rows| {
        rows.into_iter()
            .map(|(name, row)| {
                (
                    name,
                    row.definition
                        .expect("strict read requires a supported source"),
                )
            })
            .collect()
    })
}

pub fn read_listing(
    harness: Harness,
    home: &Path,
) -> Result<BTreeMap<String, Registration>, Diagnostic> {
    read_registrations(harness, home, true)
}

fn read_registrations(
    harness: Harness,
    home: &Path,
    listing: bool,
) -> Result<BTreeMap<String, Registration>, Diagnostic> {
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
                    registration(
                        kind,
                        source,
                        &path,
                        listing,
                        market
                            .get("ref")
                            .map(|v| {
                                v.as_str()
                                    .map(str::to_owned)
                                    .ok_or_else(|| failure(&path, "ref must be a string"))
                            })
                            .transpose()?,
                    )?,
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
            let value = if listing && source_value(kind, "").is_none() {
                ["url", "repo", "path", "package"]
                    .into_iter()
                    .find_map(|key| source.get(key))
            } else {
                source.get(match kind {
                    "github" => "repo",
                    "directory" => "path",
                    _ => "url",
                })
            };
            let value = match value {
                Some(value) => value
                    .as_str()
                    .ok_or_else(|| failure(&path, "Marketplace source value must be a string"))?,
                None if listing && source_value(kind, "").is_none() => "",
                None => return Err(failure(&path, "Marketplace source value missing")),
            };
            result.insert(
                name.clone(),
                registration(
                    kind,
                    value,
                    &path,
                    listing,
                    source
                        .get("ref")
                        .map(|v| {
                            v.as_str()
                                .map(str::to_owned)
                                .ok_or_else(|| failure(&path, "ref must be a string"))
                        })
                        .transpose()?,
                )?,
            );
        }
    }
    Ok(result)
}
fn registration(
    kind: &str,
    value: &str,
    path: &Path,
    listing: bool,
    reference: Option<String>,
) -> Result<Registration, Diagnostic> {
    let source = source_value(kind, value);
    if source.is_none() && !listing {
        return Err(failure(
            path,
            format!("unsupported native Marketplace source type {kind}"),
        ));
    }
    Ok(Registration {
        source: source.as_ref().map_or_else(
            || {
                if value.is_empty() {
                    kind.to_owned()
                } else {
                    format!("{kind}:{value}")
                }
            },
            Source::label,
        ),
        definition: source.map(|source| Definition {
            source,
            reference: reference.clone(),
            name: None,
        }),
        reference,
    })
}
fn source_value(kind: &str, value: &str) -> Option<Source> {
    match kind {
        "github" => Some(Source::Github(value.to_owned())),
        "git" => Some(Source::Git(value.to_owned())),
        "directory" | "local" => Some(Source::Path(PathBuf::from(value))),
        _ => None,
    }
}
pub fn same_source(left: &Definition, right: &Definition) -> bool {
    let equal = match (&left.source, &right.source) {
        (Source::Path(a), Source::Path(b)) => {
            a.canonicalize().unwrap_or_else(|_| a.clone())
                == b.canonicalize().unwrap_or_else(|_| b.clone())
        }
        (a, b) => a == b || github_repo(a).is_some_and(|repo| github_repo(b) == Some(repo)),
    };
    equal && left.reference == right.reference
}
/// Codex records `github:owner/repo` as `https://github.com/owner/repo.git`.
fn github_repo(source: &Source) -> Option<String> {
    let repo = match source {
        Source::Github(repo) => repo.as_str(),
        Source::Git(url) => [
            "https://github.com/",
            "ssh://git@github.com/",
            "git@github.com:",
        ]
        .into_iter()
        .find_map(|prefix| url.strip_prefix(prefix))?,
        Source::Path(_) => return None,
    };
    let repo = repo.trim_end_matches('/');
    Some(
        repo.strip_suffix(".git")
            .unwrap_or(repo)
            .to_ascii_lowercase(),
    )
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
    // Directory-only installs are also native installs, even without a toggle entry.
    let mut installed = crate::plugin_enumeration::read(harness, None, home)?.user;
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
