//! Read-only Harness state supplied by the caller to the pure resolver.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Default)]
pub struct InstalledPlugins {
    pub user: BTreeSet<String>,
    /// Project and local installs active for the current directory.
    /// User-level overrides must not hide these, even if also installed for the user.
    pub project: BTreeSet<String>,
    /// Copilot native install paths, including shared-home Bindings in isolated mode.
    pub paths: BTreeMap<String, PathBuf>,
    /// Copilot direct installs have no native name@marketplace Binding.
    pub direct: BTreeSet<PathBuf>,
    /// Filesystem path identity, resolved at the edge for direct installs and selections.
    pub canonical_paths: BTreeMap<PathBuf, PathBuf>,
}
