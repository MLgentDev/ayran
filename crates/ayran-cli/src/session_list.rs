//! Session listing and request summaries for list and completion output.
use crate::{
    codex_link::{self, Link},
    session_store,
};
use ayran_core::{diagnostic::Diagnostic, harness::Harness, resolve::Request, session};
use clap::ArgMatches;
use std::env;

/// Display only the typed request, in the flag order defined by the spec.
pub fn summary(request: &Request) -> String {
    let mut words = Vec::new();
    for (flag, value) in [
        ("--alias", request.alias.as_deref()),
        ("--preset", request.preset.as_deref()),
        ("-m", request.model.as_deref()),
        ("-e", request.effort.map(|effort| effort.as_str())),
    ] {
        if let Some(value) = value {
            words.push(flag.to_owned());
            words.push(crate::shell_quote(value.as_ref()));
        }
    }
    for (flag, values) in [
        ("-p", &request.profiles),
        ("--plugin", &request.plugins),
        ("--skill", &request.skills),
        ("--mcp", &request.mcp),
        ("--no-profile", &request.no_profiles),
        ("--no-plugin", &request.no_plugins),
        ("--no-skill", &request.no_skills),
        ("--no-mcp", &request.no_mcp),
    ] {
        if !values.is_empty() {
            words.push(flag.to_owned());
            words.push(crate::shell_quote(values.join(",").as_ref()));
        }
    }
    if request.no_harness_args {
        words.push("--no-harness-args".into());
    }
    if request.no_defaults {
        words.push("--no-defaults".into());
    }
    words.join(" ")
}

pub fn run(matches: &ArgMatches) -> i32 {
    let result = (|| {
        let cwd = env::current_dir()
            .map_err(|error| Diagnostic::error("cwd-not-found", error.to_string(), None))?;
        let selected = matches
            .get_one::<String>("harness")
            .map(String::as_str)
            .or_else(|| {
                [Harness::Claude, Harness::Codex, Harness::Copilot]
                    .into_iter()
                    .find(|harness| matches.get_flag(harness.binary()))
                    .map(Harness::binary)
            });
        Ok::<_, Diagnostic>(
            session_store::records()?
                .into_iter()
                .filter(|record| {
                    (matches.get_flag("all") || record.cwd == cwd)
                        && selected.is_none_or(|name| name == record.harness.binary())
                })
                .collect::<Vec<_>>(),
        )
    })();
    let (mut records, diagnostics) = match result {
        Ok(records) => (records, Vec::new()),
        Err(diagnostic) => (Vec::new(), vec![diagnostic]),
    };
    let mut all_records = session_store::records().unwrap_or_default();
    let links: Vec<_> = records
        .iter_mut()
        .map(|record| {
            if record.harness != Harness::Codex || record.native_id.is_some() {
                return record
                    .native_id
                    .clone()
                    .map(Link::Linked)
                    .unwrap_or(Link::Unlinked);
            }
            match codex_link::discover(record, &all_records) {
                Ok(Link::Linked(id)) => {
                    if codex_link::cache(record, id).is_ok() {
                        if let Some(stored) =
                            all_records.iter_mut().find(|stored| stored.id == record.id)
                        {
                            stored.native_id = record.native_id.clone();
                        }
                        Link::Linked(record.native_id.clone().unwrap())
                    } else {
                        record.native_id = None;
                        Link::Unlinked
                    }
                }
                Ok(link @ Link::Ambiguous(_)) => link,
                _ => Link::Unlinked,
            }
        })
        .collect();
    if matches.get_flag("json") {
        let sessions: Vec<_> = records.iter().zip(&links).map(|(record, link)| serde_json::json!({
            "id":record.id, "native_id":record.native_id,
            "link":link.status(),
            "harness":record.harness, "home":record.home, "cwd":record.cwd,
            "started_at":record.started_at, "last_used_at":record.last_used_at,
            "alias":record.request.alias, "request":record.request, "forked_from":record.forked_from
        })).collect();
        let mut output = crate::list_command::json_envelope(&diagnostics);
        output["sessions"] = serde_json::json!(sessions);
        println!("{output}");
    } else if diagnostics.is_empty() {
        let mut header = vec![
            "id",
            "harness",
            "last_used_at",
            "alias",
            "request",
            "parent",
        ];
        if matches.get_flag("all") {
            header.push("cwd");
        }
        println!("{}", header.join("\t"));
        for (record, link) in records.into_iter().zip(links) {
            let mut row = vec![
                session::short_id(&record.id),
                record.harness.binary().into(),
                record.last_used_at,
                record.request.alias.clone().unwrap_or_default(),
                if matches!(link, Link::Ambiguous(_)) {
                    format!("? {}", summary(&record.request))
                } else {
                    summary(&record.request)
                },
                record
                    .forked_from
                    .as_deref()
                    .map(session::short_id)
                    .unwrap_or_default(),
            ];
            if matches.get_flag("all") {
                row.push(record.cwd.display().to_string());
            }
            crate::list_command::print_row(row);
        }
    } else {
        for diagnostic in &diagnostics {
            crate::render(diagnostic, false);
        }
    }
    if diagnostics.is_empty() { 0 } else { 3 }
}
