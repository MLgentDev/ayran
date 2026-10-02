use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use ayran_core::diagnostic::Diagnostic;

pub fn check(program: &OsStr, binary: &Path, persist_cache: bool) -> Result<String, Diagnostic> {
    check_version(program, version(program, binary, persist_cache, false)?)
}

/// Audit the binary afresh and update the optional launch cache, including old versions.
pub fn check_fresh(program: &OsStr, binary: &Path) -> Result<String, Diagnostic> {
    check_version(program, version(program, binary, true, true)?)
}

fn version(
    program: &OsStr,
    binary: &Path,
    persist_cache: bool,
    fresh: bool,
) -> Result<(u64, u64, u64), Diagnostic> {
    let cache = VersionCache::for_binary(program, binary);
    if !fresh && let Some(found) = cache.as_ref().and_then(VersionCache::read) {
        return Ok(found);
    }
    let found = probe(program, binary)?;
    if persist_cache && let Some(cache) = cache {
        cache.write(found);
    }
    Ok(found)
}

fn check_version(program: &OsStr, found: (u64, u64, u64)) -> Result<String, Diagnostic> {
    let minimum = match program.to_str() {
        Some("claude") => (2, 1, 283),
        Some("codex") => (0, 158, 0),
        Some("copilot") => (1, 0, 88),
        _ => unreachable!("resolution selects a known Harness"),
    };
    let version = format_version(found);
    if found < minimum {
        return Err(Diagnostic::error(
            "harness-too-old",
            format!("Harness {} is too old", program.to_string_lossy()),
            Some(&format!(
                "found {version}; requires {} or newer; update the Harness",
                format_version(minimum)
            )),
        ));
    }
    Ok(version)
}

fn probe(program: &OsStr, binary: &Path) -> Result<(u64, u64, u64), Diagnostic> {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .map_err(|error| version_error(program, error.to_string()))?;
    if !output.status.success() {
        return Err(version_error(
            program,
            format!(
                "--version exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace()
        // Copilot prints its version at the end of a sentence ("1.0.90.").
        .find_map(|word| parse_version(word.strip_suffix('.').unwrap_or(word)))
        .ok_or_else(|| version_error(program, format!("unparseable --version output: {text:?}")))
}

struct VersionCache {
    file: PathBuf,
    binary: String,
    modified: String,
}

impl VersionCache {
    fn for_binary(program: &OsStr, binary: &Path) -> Option<Self> {
        let binary = binary.canonicalize().ok()?;
        let modified = binary.metadata().ok()?.modified().ok()?;
        let modified = modified
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_nanos()
            .to_string();
        #[cfg(unix)]
        let root = env::var_os("XDG_CACHE_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
        #[cfg(windows)]
        let root = PathBuf::from(env::var_os("LOCALAPPDATA")?);
        Some(Self {
            file: root
                .join("ayran/versions")
                .join(program)
                .with_extension("json"),
            binary: binary.to_str()?.to_owned(),
            modified,
        })
    }

    fn read(&self) -> Option<(u64, u64, u64)> {
        let contents = fs::read(&self.file).ok()?;
        let value: serde_json::Value = serde_json::from_slice(&contents).ok()?;
        if value.get("binary")?.as_str()? != self.binary
            || value.get("modified")?.as_str()? != self.modified
        {
            return None;
        }
        parse_version(value.get("version")?.as_str()?)
    }

    fn write(&self, version: (u64, u64, u64)) {
        let value = serde_json::json!({
            "binary": self.binary,
            "modified": self.modified,
            "version": format_version(version),
        });
        // The cache is optional: unreadable, malformed or unwritable entries are misses.
        if fs::create_dir_all(self.file.parent().unwrap()).is_ok() {
            let _ = fs::write(&self.file, value.to_string());
        }
    }
}

fn parse_version(word: &str) -> Option<(u64, u64, u64)> {
    let mut parts = word.split('.');
    let mut next = || {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let version = (next()?, next()?, next()?);
    if parts.next().is_some() {
        return None;
    }
    Some(version)
}

fn format_version(version: (u64, u64, u64)) -> String {
    format!("{}.{}.{}", version.0, version.1, version.2)
}

fn version_error(program: &OsStr, reason: String) -> Diagnostic {
    Diagnostic::error(
        "harness-version-failed",
        format!(
            "cannot determine Harness {} version: {reason}",
            program.to_string_lossy()
        ),
        Some("check that the Harness supports --version and update it if needed"),
    )
}
