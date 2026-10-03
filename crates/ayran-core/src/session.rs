//! Stored Session facts and pure request operations.
use std::path::PathBuf;

use crate::{harness::Harness, launch::HomeMode, resolve::Request};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionRecord {
    pub version: u32,
    pub id: String,
    pub native_id: Option<String>,
    pub harness: Harness,
    pub home: HomeMode,
    pub cwd: PathBuf,
    pub started_at: String,
    pub last_used_at: String,
    pub request: Request,
    pub forked_from: Option<String>,
}

/// Normalize only the typed request; never expand config or Aliases here.
pub fn normalize(mut request: Request) -> Request {
    for list in [
        &mut request.plugins,
        &mut request.skills,
        &mut request.mcp,
        &mut request.profiles,
        &mut request.no_plugins,
        &mut request.no_skills,
        &mut request.no_mcp,
        &mut request.no_profiles,
    ] {
        let mut seen = std::collections::HashSet::new();
        list.retain(|name| seen.insert(name.clone()));
    }
    request.harness = None;
    request.passthrough.clear();
    request
}

/// Replay the stored request with new CLI flags, preserving typed order.
pub fn merge(stored: &Request, flags: Request) -> Request {
    let mut merged = stored.clone();
    if flags.model.is_some() {
        merged.model = flags.model;
    }
    if flags.effort.is_some() {
        merged.effort = flags.effort;
    }
    fn pair(
        selected: &mut Vec<String>,
        disabled: &mut Vec<String>,
        additions: Vec<String>,
        removals: Vec<String>,
    ) {
        selected.retain(|name| !removals.contains(name));
        disabled.retain(|name| !additions.contains(name));
        selected.extend(additions);
        disabled.extend(removals);
    }
    pair(
        &mut merged.plugins,
        &mut merged.no_plugins,
        flags.plugins,
        flags.no_plugins,
    );
    pair(
        &mut merged.skills,
        &mut merged.no_skills,
        flags.skills,
        flags.no_skills,
    );
    pair(&mut merged.mcp, &mut merged.no_mcp, flags.mcp, flags.no_mcp);
    pair(
        &mut merged.profiles,
        &mut merged.no_profiles,
        flags.profiles,
        flags.no_profiles,
    );
    merged.no_defaults |= flags.no_defaults;
    merged.no_harness_args |= flags.no_harness_args;
    let mut merged = normalize(merged);
    merged.passthrough = flags.passthrough;
    merged
}

pub fn short_id(id: &str) -> String {
    id.chars()
        .filter(|character| *character != '-')
        .take(8)
        .collect()
}

pub fn matches_prefix(id: &str, prefix: &str) -> bool {
    let normalized = |value: &str| value.replace('-', "").to_ascii_lowercase();
    let prefix = normalized(prefix);
    !prefix.is_empty() && normalized(id).starts_with(&prefix)
}
