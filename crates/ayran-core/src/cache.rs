//! Pure descriptions of immutable generated Session directories.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::harness::Harness;

pub struct GeneratedSkills {
    pub harness: Harness,
    pub directory: PathBuf,
    pub targets: BTreeMap<String, PathBuf>,
    /// Embedded source trees to publish before linking the Session directory.
    pub builtin: Vec<PathBuf>,
}

impl GeneratedSkills {
    pub fn new(root: &Path, harness: Harness, targets: BTreeMap<String, PathBuf>) -> Self {
        let mut hash = Sha256::new();
        // Length-prefix every field so names and targets cannot have ambiguous boundaries.
        let mut field = |bytes: &[u8]| {
            hash.update((bytes.len() as u64).to_be_bytes());
            hash.update(bytes);
        };
        field(harness.binary().as_bytes());
        for (name, target) in &targets {
            field(name.as_bytes());
            field(target.as_os_str().as_encoded_bytes());
        }
        let directory = root
            .join(harness.binary())
            .join(format!("{:x}", hash.finalize()));
        Self {
            harness,
            directory,
            targets,
            builtin: Vec::new(),
        }
    }
}

pub struct GeneratedMcp {
    pub directory: PathBuf,
    pub contents: String,
}

impl GeneratedMcp {
    pub fn new(root: &Path, harness: Harness, config: &serde_json::Value) -> Self {
        let contents = config.to_string();
        let mut hash = Sha256::new();
        hash.update(b"ayran-mcp-v1\0");
        hash.update(contents.as_bytes());
        Self {
            directory: root
                .join(harness.binary())
                .join(format!("{:x}", hash.finalize())),
            contents,
        }
    }

    pub fn file(&self) -> PathBuf {
        self.directory.join("mcp.json")
    }
}
