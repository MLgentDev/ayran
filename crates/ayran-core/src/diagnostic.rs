use std::fmt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    #[default]
    Error,
    Warning,
    Note,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
        })
    }
}

#[derive(Default, serde::Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub hint: Option<String>,
    pub layer: Option<String>,
    pub harness: Option<crate::harness::Harness>,
    pub capability: Option<Box<Capability>>,
    pub item: Option<Box<String>>,
    pub cause: Option<Box<String>>,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>, hint: Option<&str>) -> Self {
        Self {
            code,
            severity: Severity::Error,
            message: message.into(),
            hint: hint.map(str::to_owned),
            layer: None,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Capability {
    pub kind: CapabilityKind,
    pub name: String,
}

impl Diagnostic {
    pub fn with_item(mut self, item: impl AsRef<str>) -> Self {
        self.item = Some(Box::new(item.as_ref().to_owned()));
        self
    }

    pub fn for_capability(
        mut self,
        harness: crate::harness::Harness,
        kind: CapabilityKind,
        name: &str,
        layer: &std::path::Path,
    ) -> Self {
        self.harness = Some(harness);
        self.capability = Some(Box::new(Capability {
            kind,
            name: name.into(),
        }));
        self.layer = Some(crate::config::layer_name(layer).into_owned());
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CapabilityKind {
    Plugin,
    Skill,
    Mcp,
    Profile,
}
impl fmt::Display for CapabilityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Plugin => "plugin",
            Self::Skill => "skill",
            Self::Mcp => "mcp",
            Self::Profile => "profile",
        })
    }
}
