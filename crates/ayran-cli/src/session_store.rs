//! Filesystem boundary for Session records. Writes are atomic; exec failures roll back.
use ayran_core::{diagnostic::Diagnostic, session::SessionRecord};
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};

fn failure(error: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error("session-store-failed", error.to_string(), None)
}

fn directory() -> Result<PathBuf, Diagnostic> {
    #[cfg(windows)]
    let state = env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let state = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")));
    let path = state
        .ok_or_else(|| failure("cannot locate Session records"))?
        .join("ayran/sessions");
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(env::current_dir().map_err(failure)?.join(path))
    }
}

pub struct SessionWrite {
    path: PathBuf,
    previous: Option<Vec<u8>>,
}

impl SessionWrite {
    pub fn prepare(record: &SessionRecord) -> Result<Self, Diagnostic> {
        let path = directory()?.join(format!("{}.json", record.id));
        let previous = match fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(failure(error)),
        };
        Ok(Self { path, previous })
    }

    pub fn write(&self, record: &SessionRecord) -> Result<(), Diagnostic> {
        self.atomic_write(&serde_json::to_vec_pretty(record).map_err(failure)?)
    }

    fn atomic_write(&self, bytes: &[u8]) -> Result<(), Diagnostic> {
        let parent = self.path.parent().unwrap();
        fs::create_dir_all(parent).map_err(failure)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(failure)?;
        temporary.write_all(bytes).map_err(failure)?;
        temporary.as_file().sync_all().map_err(failure)?;
        temporary.persist(&self.path).map_err(failure)?;
        Ok(())
    }

    pub fn rollback(&self) -> Result<(), Diagnostic> {
        if let Some(bytes) = &self.previous {
            self.atomic_write(bytes)
        } else {
            fs::remove_file(&self.path).map_err(failure)
        }
    }
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}

pub fn find(prefix: &str) -> Result<SessionRecord, Diagnostic> {
    let entries = match fs::read_dir(directory()?) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Err(unknown(prefix)),
        Err(error) => return Err(failure(error)),
    };
    let mut matches = Vec::new();
    for entry in entries {
        let path = entry.map_err(failure)?.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        if !ayran_core::session::matches_prefix(id, prefix) {
            continue;
        }
        let record = read_record(&path)?;
        if ayran_core::session::matches_prefix(&record.id, prefix) {
            matches.push(record);
        }
    }
    match matches.len() {
        0 => Err(unknown(prefix)),
        1 => Ok(matches.pop().unwrap()),
        _ => {
            let mut ids: Vec<_> = matches
                .iter()
                .map(|record| ayran_core::session::short_id(&record.id))
                .collect();
            ids.sort();
            Err(Diagnostic::error(
                "usage",
                format!("ambiguous Session prefix {prefix}: {}", ids.join(", ")),
                Some("use a longer Session ID prefix"),
            ))
        }
    }
}

fn unknown(prefix: &str) -> Diagnostic {
    Diagnostic::error(
        "unknown-session",
        format!("no Session matches {prefix}"),
        None,
    )
}

fn read_record(path: &Path) -> Result<SessionRecord, Diagnostic> {
    let record: SessionRecord =
        serde_json::from_slice(&fs::read(path).map_err(failure)?).map_err(failure)?;
    if record.version != 1
        || uuid::Uuid::parse_str(&record.id).is_err()
        || path.file_stem().and_then(|name| name.to_str()) != Some(record.id.as_str())
        || chrono::DateTime::parse_from_rfc3339(&record.started_at).is_err()
        || chrono::DateTime::parse_from_rfc3339(&record.last_used_at).is_err()
    {
        return Err(failure(format!(
            "invalid Session record {}",
            path.display()
        )));
    }
    Ok(record)
}

/// Read valid records without allowing one damaged file to hide other Sessions.
pub fn records() -> Result<Vec<SessionRecord>, Diagnostic> {
    let entries = match fs::read_dir(directory()?) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(failure(error)),
    };
    let mut records = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        if let Ok(record) = read_record(&path) {
            records.push(record);
        }
    }
    records.sort_by(|a, b| {
        chrono::DateTime::parse_from_rfc3339(&b.last_used_at)
            .unwrap()
            .cmp(&chrono::DateTime::parse_from_rfc3339(&a.last_used_at).unwrap())
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(records)
}

/// Best-effort maintenance at launch. Never infer age from file modification time.
pub fn prune(current_id: Option<&str>) {
    let Ok(directory) = directory() else {
        return;
    };
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let cutoff = chrono::Utc::now() - chrono::Duration::days(30);
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json")
            || current_id
                .is_some_and(|id| path.file_stem().and_then(|name| name.to_str()) == Some(id))
        {
            continue;
        }
        let used = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| {
                value
                    .get("last_used_at")?
                    .as_str()
                    .and_then(|used| chrono::DateTime::parse_from_rfc3339(used).ok())
            });
        if used.is_some_and(|used| used < cutoff) {
            let _ = fs::remove_file(path);
        }
    }
}
