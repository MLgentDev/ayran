//! Harness-neutral Marketplace declarations. Paths are resolved at their declaring layer.
use crate::{diagnostic::Diagnostic, harness::Harness};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Github(String),
    Git(String),
    Path(PathBuf),
}
impl Source {
    pub fn label(&self) -> String {
        match self {
            Self::Github(repo) => format!("github:{repo}"),
            Self::Git(url) => url.clone(),
            Self::Path(path) => path.display().to_string(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Definition {
    pub source: Source,
    pub reference: Option<String>,
    pub name: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Binding {
    Source(Definition),
    Absent,
}
#[derive(Default, serde::Serialize)]
pub struct Marketplace {
    pub all: Option<Binding>,
    pub claude: Option<Binding>,
    pub codex: Option<Binding>,
    pub copilot: Option<Binding>,
    pub project: bool,
}
impl Marketplace {
    pub fn binding(&self, harness: Harness) -> Option<&Binding> {
        match harness {
            Harness::Claude => self.claude.as_ref(),
            Harness::Codex => self.codex.as_ref(),
            Harness::Copilot => self.copilot.as_ref(),
        }
        .or(self.all.as_ref())
    }
    pub fn definition(&self, harness: Harness) -> Option<&Definition> {
        match self.binding(harness) {
            Some(Binding::Source(definition)) => Some(definition),
            _ => None,
        }
    }
    pub fn parse(value: &toml::Value, path: &Path, is_user: bool) -> Result<Self, Diagnostic> {
        let mut marketplace = Self {
            project: !is_user && path.file_name().is_none_or(|n| n != "ayran.local.toml"),
            ..Self::default()
        };
        let fields = value
            .as_table()
            .ok_or_else(|| invalid(path, "Marketplace must be a table"))?;
        for (key, value) in fields {
            let slot = match key.as_str() {
                "all" => &mut marketplace.all,
                "claude" => &mut marketplace.claude,
                "codex" => &mut marketplace.codex,
                "copilot" => &mut marketplace.copilot,
                _ => return Err(invalid(path, format!("unknown Marketplace key {key}"))),
            };
            if value.as_bool() == Some(false) {
                *slot = Some(Binding::Absent);
                continue;
            }
            let fields = value
                .as_table()
                .ok_or_else(|| invalid(path, "Marketplace Binding must be false or a table"))?;
            if fields
                .keys()
                .any(|k| !["source", "ref", "name"].contains(&k.as_str()))
            {
                return Err(invalid(path, "unknown Marketplace Binding key"));
            }
            let source = fields
                .get("source")
                .ok_or_else(|| invalid(path, "Marketplace Binding requires source"))?;
            let source = if let Some(text) = source.as_str() {
                if let Some(repo) = text.strip_prefix("github:") {
                    let parts: Vec<_> = repo.split('/').collect();
                    if parts.len() != 2
                        || parts.iter().any(|p| {
                            p.is_empty()
                                || !p
                                    .bytes()
                                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                        })
                    {
                        return Err(invalid(path, "github source must be github:owner/repo"));
                    }
                    Source::Github(repo.to_owned())
                } else if valid_git_url(text) {
                    // Refs belong in the explicit ref field, so native comparisons are unambiguous.
                    Source::Git(text.to_owned())
                } else {
                    return Err(invalid(
                        path,
                        "source must be github:owner/repo, an https/ssh/git@ git URL, or { path = … }",
                    ));
                }
            } else {
                let fields = source
                    .as_table()
                    .ok_or_else(|| invalid(path, "invalid Marketplace source"))?;
                let text = fields
                    .get("path")
                    .and_then(toml::Value::as_str)
                    .filter(|p| !p.is_empty())
                    .ok_or_else(|| invalid(path, "path source requires a nonempty path"))?;
                if fields.len() != 1 {
                    return Err(invalid(path, "path source accepts only path"));
                }
                let source = PathBuf::from(text);
                Source::Path(if source.is_absolute() {
                    source
                } else {
                    path.parent().unwrap_or(Path::new(".")).join(source)
                })
            };
            let string = |key| -> Result<Option<String>, Diagnostic> {
                fields
                    .get(key)
                    .map(|v| {
                        v.as_str()
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .ok_or_else(|| {
                                invalid(
                                    path,
                                    format!("Marketplace {key} must be a nonempty string"),
                                )
                            })
                    })
                    .transpose()
            };
            let name = string("name")?;
            if name.as_deref().is_some_and(|n| !valid_name(n)) {
                return Err(invalid(path, "invalid native Marketplace name"));
            }
            if key == "all" && name.is_some() {
                return Err(invalid(
                    path,
                    "Marketplace name override is per-Harness only",
                ));
            }
            *slot = Some(Binding::Source(Definition {
                source,
                reference: string("ref")?,
                name,
            }));
        }
        Ok(marketplace)
    }
}
fn invalid(path: &Path, message: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error(
        "config-invalid",
        format!("{}: {message}", path.display()),
        None,
    )
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        && name != "."
        && name != ".."
}
fn valid_git_url(text: &str) -> bool {
    if text.chars().any(|c| c.is_whitespace() || c.is_control()) || text.contains('#') {
        return false;
    }
    if let Some(address) = text.strip_prefix("git@") {
        return address
            .split_once(':')
            .is_some_and(|(host, repo)| !host.is_empty() && !repo.is_empty());
    }
    url::Url::parse(text).is_ok_and(|url| {
        matches!(url.scheme(), "https" | "ssh")
            && url.host_str().is_some()
            && !url.path().trim_matches('/').is_empty()
    })
}
