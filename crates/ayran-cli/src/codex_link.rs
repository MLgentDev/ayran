//! Read Codex rollouts at the filesystem boundary, without consulting its private database.
use crate::{harness_home::HarnessHome, session_store};
use ayran_core::{diagnostic::Diagnostic, harness::Harness, session::SessionRecord};
use chrono::{DateTime, FixedOffset};
use serde_json::Value;
use std::{
    env, fs,
    io::{self, BufRead, BufReader, IsTerminal, Write},
    path::{Path, PathBuf},
};

pub struct Candidate {
    pub id: String,
    pub timestamp: DateTime<FixedOffset>,
    pub message: String,
}

pub enum Link {
    Linked(String),
    Unlinked,
    Ambiguous(Vec<Candidate>),
}

impl Link {
    pub fn status(&self) -> &'static str {
        match self {
            Self::Linked(_) => "linked",
            Self::Unlinked => "unlinked",
            Self::Ambiguous(_) => "ambiguous",
        }
    }
}

fn home(record: &SessionRecord) -> Result<PathBuf, Diagnostic> {
    let real = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    Ok(HarnessHome::resolve(Harness::Codex, record.home, real.as_deref())?.directory)
}

pub fn discover(record: &SessionRecord, records: &[SessionRecord]) -> Result<Link, Diagnostic> {
    if let Some(id) = &record.native_id {
        return Ok(Link::Linked(id.clone()));
    }
    let directory = home(record)?;
    let start = DateTime::parse_from_rfc3339(&record.started_at).unwrap();
    let end = records
        .iter()
        .filter(|other| {
            other.harness == Harness::Codex && other.cwd == record.cwd && other.id != record.id
        })
        .filter(|other| home(other).is_ok_and(|path| path == directory))
        .filter_map(|other| DateTime::parse_from_rfc3339(&other.started_at).ok())
        .filter(|time| *time > start)
        .min();
    let mut paths = Vec::new();
    rollout_paths(&directory.join("sessions"), &mut paths);
    let mut candidates: Vec<_> = paths
        .iter()
        .filter_map(|path| read_candidate(path, record, start, end))
        .filter(|candidate| {
            !records.iter().any(|other| {
                other.id != record.id
                    && other.harness == Harness::Codex
                    && other.native_id.as_deref() == Some(&candidate.id)
            })
        })
        .collect();
    candidates.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then_with(|| a.id.cmp(&b.id)));
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|candidate| seen.insert(candidate.id.clone()));
    Ok(match candidates.len() {
        0 => Link::Unlinked,
        1 => Link::Linked(candidates.pop().unwrap().id),
        _ => Link::Ambiguous(candidates),
    })
}

fn rollout_paths(directory: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            rollout_paths(&path, paths);
        } else if kind.is_file()
            && path.extension().is_some_and(|ext| ext == "jsonl")
            && entry.file_name().to_string_lossy().starts_with("rollout-")
        {
            paths.push(path);
        }
    }
}

fn read_candidate(
    path: &Path,
    record: &SessionRecord,
    start: DateTime<FixedOffset>,
    end: Option<DateTime<FixedOffset>>,
) -> Option<Candidate> {
    let file = fs::File::open(path).ok()?;
    let mut candidate = None;
    let mut first_message = None;
    let mut legacy_message = None;
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else {
            break;
        };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let payload = &value["payload"];
        if value["type"] == "session_meta" && candidate.is_none() {
            let source = &payload["source"];
            if !payload["parent_thread_id"].is_null()
                || source.get("subagent").is_some()
                || source.get("internal").is_some()
                || payload["cwd"].as_str()? != record.cwd.to_str()?
            {
                return None;
            }
            let timestamp = DateTime::parse_from_rfc3339(payload["timestamp"].as_str()?).ok()?;
            if timestamp < start || end.is_some_and(|end| timestamp >= end) {
                return None;
            }
            let id = uuid::Uuid::parse_str(payload["id"].as_str()?)
                .ok()?
                .to_string();
            candidate = Some(Candidate {
                id,
                timestamp,
                message: String::new(),
            });
        }
        if value["type"] == "event_msg" {
            if first_message.is_none()
                && payload["type"] == "item_completed"
                && payload["item"]["type"] == "UserMessage"
            {
                first_message = Some(
                    payload["item"]["content"]
                        .as_array()?
                        .iter()
                        .filter(|part| part["type"] == "text")
                        .filter_map(|part| part["text"].as_str())
                        .collect::<Vec<_>>()
                        .join(""),
                );
            }
            if candidate.is_some() && first_message.is_some() {
                break;
            }
            if legacy_message.is_none() && payload["type"] == "user_message" {
                legacy_message = payload["message"].as_str().map(str::to_owned);
            }
        }
    }
    let mut candidate = candidate?;
    candidate.message = first_message
        .or(legacy_message)
        .unwrap_or_else(|| "(no user message)".into())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Some(candidate)
}

pub fn cache(record: &mut SessionRecord, id: String) -> Result<(), Diagnostic> {
    record.native_id = Some(id);
    session_store::SessionWrite::prepare(record)?.write(record)
}

pub fn resolve_resume(record: &SessionRecord, native: Option<&str>) -> Result<String, Diagnostic> {
    if let Some(native) = native {
        return uuid::Uuid::parse_str(native)
            .map(|id| id.to_string())
            .map_err(|_| Diagnostic::error("usage", "--native requires a UUID", None));
    }
    match discover(record, &session_store::records()?)? {
        Link::Linked(id) => Ok(id),
        Link::Unlinked => Err(Diagnostic::error(
            "codex-link-not-found",
            "no Codex rollout found for this Session (did it exit before the first prompt?)",
            None,
        )),
        Link::Ambiguous(candidates) if io::stdin().is_terminal() && io::stderr().is_terminal() => {
            pick(&candidates)
        }
        Link::Ambiguous(candidates) => Err(Diagnostic::error(
            "codex-link-ambiguous",
            format!(
                "several Codex rollouts match: {}",
                candidates
                    .iter()
                    .map(|candidate| candidate.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Some("use --native <uuid> to choose a thread"),
        )),
    }
}

fn pick(candidates: &[Candidate]) -> Result<String, Diagnostic> {
    for (index, candidate) in candidates.iter().enumerate() {
        eprintln!(
            "{}. {} {} {}",
            index + 1,
            candidate.id,
            candidate
                .timestamp
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S %:z"),
            candidate.message
        );
    }
    eprint!("Choose a Codex thread (number, empty to cancel): ");
    io::stderr()
        .flush()
        .map_err(|error| Diagnostic::error("codex-link-ambiguous", error.to_string(), None))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| Diagnostic::error("codex-link-ambiguous", error.to_string(), None))?;
    answer
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|number| number.checked_sub(1))
        .and_then(|index| candidates.get(index))
        .map(|candidate| candidate.id.clone())
        .ok_or_else(|| {
            Diagnostic::error(
                "codex-link-ambiguous",
                "no Codex thread selected",
                Some("use --native <uuid> to choose a thread"),
            )
        })
}
