//! Resolve Harness homes once, without changing the process environment.

use std::env;
use std::path::{Path, PathBuf};

use ayran_core::diagnostic::Diagnostic;
use ayran_core::harness::Harness;
use ayran_core::launch::HomeMode;

pub struct HarnessHome {
    pub directory: PathBuf,
    pub variable: &'static str,
    pub claude_mcp: PathBuf,
    pub mode: HomeMode,
}

pub fn variable(harness: Harness) -> &'static str {
    match harness {
        Harness::Claude => "CLAUDE_CONFIG_DIR",
        Harness::Codex => "CODEX_HOME",
        Harness::Copilot => "COPILOT_HOME",
    }
}

pub fn directory(harness: Harness, mode: HomeMode, real_home: Option<&Path>) -> Option<PathBuf> {
    if mode == HomeMode::Isolated {
        #[cfg(windows)]
        let state = env::var_os("LOCALAPPDATA").map(PathBuf::from);
        #[cfg(not(windows))]
        let state = env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| real_home.map(|home| home.join(".local/state")));
        state.map(|state| state.join("ayran/homes").join(harness.binary()))
    } else {
        env::var_os(variable(harness))
            .map(PathBuf::from)
            .or_else(|| real_home.map(|home| home.join(format!(".{}", harness.binary()))))
    }
}

impl HarnessHome {
    pub fn resolve(
        harness: Harness,
        mode: HomeMode,
        real_home: Option<&Path>,
    ) -> Result<Self, Diagnostic> {
        let directory = directory(harness, mode, real_home).ok_or_else(|| {
            Diagnostic::error(
                "enumeration-failed",
                format!("cannot locate {}'s home", harness.binary()),
                None,
            )
        })?;
        let directory = if directory.is_absolute() {
            directory
        } else {
            env::current_dir()
                .map_err(|error| Diagnostic::error("enumeration-failed", error.to_string(), None))?
                .join(directory)
        };
        let claude_mcp = if mode == HomeMode::Shared && env::var_os("CLAUDE_CONFIG_DIR").is_none() {
            real_home
                .map(|home| home.join(".claude.json"))
                .unwrap_or_else(|| directory.join(".claude.json"))
        } else {
            directory.join(".claude.json")
        };
        Ok(Self {
            directory,
            variable: variable(harness),
            claude_mcp,
            mode,
        })
    }

    pub fn materialize(&self) -> Result<(), Diagnostic> {
        if self.mode == HomeMode::Isolated {
            std::fs::create_dir_all(&self.directory).map_err(|error| {
                Diagnostic::error(
                    "home-unavailable",
                    format!(
                        "cannot create Isolated home {}: {error}",
                        self.directory.display()
                    ),
                    None,
                )
            })?;
        }
        Ok(())
    }
}
