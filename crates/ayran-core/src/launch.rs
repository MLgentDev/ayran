use crate::diagnostic::Diagnostic;
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HomeMode {
    #[default]
    Shared,
    Isolated,
}

pub struct LaunchPlan {
    pub home_mode: HomeMode,
    pub program: OsString,
    pub args: Vec<OsString>,
    pub env_set: Vec<(OsString, OsString)>,
    pub env_remove: Vec<OsString>,
    pub trace: ResolutionTrace,
    /// Path Bindings the caller must validate before launching the Harness.
    pub plugin_paths: Vec<PathBuf>,
    /// Surviving native Plugin selections, for read-only payload enumeration at the edge.
    pub native_plugins: Vec<String>,
    pub generated_skills: Option<crate::cache::GeneratedSkills>,
    pub mcp: crate::mcp::ResolvedMcp,
    pub diagnostics: Vec<Diagnostic>,
}

pub struct ResolutionTrace {
    pub harness: String,
    pub model: String,
    pub effort: String,
    pub plugins: Vec<String>,
    pub skills: Vec<String>,
    pub profiles: Vec<String>,
}
