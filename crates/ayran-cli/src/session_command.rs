//! Prepare and persist Sessions at the CLI boundary; resolution stays in the core.
use std::{env, ffi::OsString};

use crate::session_store::{self, SessionWrite};
use ayran_core::{
    config::ConfigLayers,
    diagnostic::{Diagnostic, Severity},
    harness::Harness,
    launch::LaunchPlan,
    resolve::Request,
    session::{self, SessionRecord},
};
use clap::ArgMatches;

/// Select the record and enter its directory before loading Config layers.
pub fn prepare_resume(
    matches: &ArgMatches,
    chosen: Option<Harness>,
) -> Result<SessionRecord, Diagnostic> {
    if matches.get_one::<String>("alias").is_some() {
        return Err(Diagnostic::error(
            "usage",
            "resume cannot change the Alias",
            None,
        ));
    }
    let mut record = if matches.get_flag("last") {
        let cwd = env::current_dir()
            .map_err(|error| Diagnostic::error("cwd-not-found", error.to_string(), None))?;
        session_store::records()?
            .into_iter()
            .find(|record| {
                (matches.get_flag("all") || record.cwd == cwd)
                    && chosen.is_none_or(|harness| harness == record.harness)
            })
            .ok_or_else(|| {
                Diagnostic::error("unknown-session", "no Session matches --last", None)
            })?
    } else {
        session_store::find(matches.get_one::<String>("id").unwrap())?
    };
    if chosen.is_some_and(|harness| harness != record.harness) {
        return Err(Diagnostic::error(
            "usage",
            "resume cannot change the Harness",
            None,
        ));
    }
    if matches.get_flag("fork") && record.harness == Harness::Copilot {
        return Err(Diagnostic::error(
            "fork-unsupported",
            "Copilot CLI can't fork at launch; use /fork inside the session",
            None,
        ));
    }
    if matches.get_one::<String>("native").is_some() && record.harness != Harness::Codex {
        return Err(Diagnostic::error(
            "usage",
            "--native is valid only for Codex Sessions",
            None,
        ));
    }
    env::set_current_dir(&record.cwd).map_err(|error| {
        Diagnostic::error(
            "cwd-not-found",
            format!("cannot enter {}: {error}", record.cwd.display()),
            None,
        )
    })?;
    if record.harness == Harness::Codex {
        record.native_id = Some(crate::codex_link::resolve_resume(
            &record,
            matches.get_one::<String>("native").map(String::as_str),
        )?);
    }
    Ok(record)
}

pub fn replay(
    record: &SessionRecord,
    flags: Request,
    layers: &ConfigLayers,
) -> Result<Request, Diagnostic> {
    if let Some(name) = &record.request.alias {
        match layers.aliases.get(name) {
            None => {
                return Err(Diagnostic::error(
                    "unknown-alias",
                    format!("recorded Alias {name} no longer exists"),
                    Some("restore the Alias, or resume natively with the Harness"),
                ));
            }
            Some(alias) if alias.harness != record.harness => {
                return Err(Diagnostic::error(
                    "alias-harness-changed",
                    format!("recorded Alias {name} now names another Harness"),
                    None,
                ));
            }
            _ => {}
        }
    }
    let mut request = session::merge(&record.request, flags);
    request.harness = Some(record.harness);
    Ok(request)
}

/// Add the native identity arguments and retain only the request as typed.
pub fn prepare_record(
    resumed: Option<SessionRecord>,
    fork: bool,
    request: &Request,
    layers: &ConfigLayers,
    harness: Harness,
    plan: &mut LaunchPlan,
) -> Result<Option<SessionRecord>, Diagnostic> {
    if let Some(mut record) = resumed {
        if layers.settings(record.harness).home_mode != record.home {
            plan.diagnostics.push(Diagnostic {
                code: "home-mode-changed",
                harness: Some(record.harness),
                severity: Severity::Note,
                message: "using the recorded Session home mode; current config chooses another"
                    .into(),
                hint: None,
                layer: None,
                ..Diagnostic::default()
            });
        }
        plan.home_mode = record.home;
        let (command, native) = if record.harness == Harness::Codex {
            // Passthrough is appended by resolution and remains opaque.
            let generated = &plan.args[..plan.args.len() - request.passthrough.len()];
            if generated.iter().any(|arg| {
                arg == "-m" || arg.to_string_lossy().starts_with("model_reasoning_effort=")
            }) {
                plan.diagnostics.push(Diagnostic {
                    code: "codex-resume-override",
                    harness: Some(record.harness),
                    severity: Severity::Note,
                    message: "reapplying model and Effort overrides on Codex resume".into(),
                    hint: None,
                    layer: None,
                    ..Diagnostic::default()
                });
            }
            (
                if fork { "fork" } else { "resume" },
                record.native_id.as_ref().unwrap(),
            )
        } else {
            ("--resume", &record.id)
        };
        plan.args
            .splice(0..0, [OsString::from(command), native.into()]);
        if fork {
            let id = uuid::Uuid::new_v4().to_string();
            if harness == Harness::Claude {
                plan.args.splice(
                    2..2,
                    [
                        OsString::from("--fork-session"),
                        OsString::from("--session-id"),
                        id.clone().into(),
                    ],
                );
            }
            record.forked_from = Some(record.id);
            record.id = id.clone();
            record.native_id = (harness != Harness::Codex).then_some(id);
            record.started_at = session_store::now();
        }
        record.request = session::normalize(request.clone());
        Ok(Some(record))
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        if harness != Harness::Codex {
            plan.args
                .splice(0..0, [OsString::from("--session-id"), id.clone().into()]);
        }
        let now = session_store::now();
        Ok(Some(SessionRecord {
            version: 1,
            native_id: (harness != Harness::Codex).then(|| id.clone()),
            id,
            harness,
            home: plan.home_mode,
            cwd: env::current_dir()
                .map_err(|error| Diagnostic::error("cwd-not-found", error.to_string(), None))?,
            started_at: now.clone(),
            last_used_at: now,
            request: session::normalize(request.clone()),
            forked_from: None,
        }))
    }
}

pub fn persist(
    record: Option<&mut SessionRecord>,
    fork: bool,
) -> Result<Vec<SessionWrite>, Diagnostic> {
    let Some(record) = record else {
        return Ok(Vec::new());
    };
    record.last_used_at = session_store::now();
    let write = SessionWrite::prepare(record)?;
    write.write(record)?;
    let mut writes = vec![write];
    if let Some(parent_id) = &record.forked_from
        && fork
    {
        let touch_parent = || {
            let mut parent = session_store::find(parent_id)?;
            parent.last_used_at = record.last_used_at.clone();
            let write = SessionWrite::prepare(&parent)?;
            write.write(&parent)?;
            Ok(write)
        };
        match touch_parent() {
            Ok(write) => writes.push(write),
            Err(error) => {
                writes[0].rollback()?;
                return Err(error);
            }
        }
    }
    Ok(writes)
}

/// Suggest dropping only recorded selections that have vanished from config.
pub fn recovery_hint(record: &SessionRecord, layers: &ConfigLayers, diagnostic: &mut Diagnostic) {
    let (flag, names): (&str, Vec<&String>) = match diagnostic.code {
        "unknown-skill" => (
            "--no-skill",
            record
                .request
                .skills
                .iter()
                .filter(|name| !layers.skills.contains_key(*name))
                .collect(),
        ),
        "unknown-plugin" => (
            "--no-plugin",
            record
                .request
                .plugins
                .iter()
                .filter(|name| !layers.plugins.contains_key(*name))
                .collect(),
        ),
        "unknown-mcp" => (
            "--no-mcp",
            record
                .request
                .mcp
                .iter()
                .filter(|name| !layers.mcp.contains_key(*name))
                .collect(),
        ),
        "unknown-profile" => (
            "--no-profile",
            record
                .request
                .profiles
                .iter()
                .filter(|name| !layers.profiles.contains_key(*name))
                .collect(),
        ),
        _ => return,
    };
    if !names.is_empty() {
        let names = names
            .into_iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(",");
        diagnostic.hint = Some(format!(
            "run ayran resume {} {flag} {names} to drop vanished selections",
            record.id
        ));
    }
}
