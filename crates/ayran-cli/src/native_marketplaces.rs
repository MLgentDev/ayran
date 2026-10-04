//! Read-only listings of registered user-level Marketplaces in the launch home.
use ayran_core::{
    config::ConfigLayers,
    diagnostic::{Diagnostic, Severity},
    harness::Harness,
};
use clap::ArgMatches;

#[derive(serde::Serialize)]
struct Marketplace {
    harness: Harness,
    name: String,
    source: String,
    #[serde(rename = "ref")]
    reference: Option<String>,
    logical: Vec<String>,
    #[serde(rename = "match")]
    status: &'static str,
    installed_plugins: usize,
}

pub fn run(matches: &ArgMatches) -> i32 {
    let mut diagnostics = Vec::new();
    let mut rows = Vec::new();
    let result = ConfigLayers::load().and_then(|layers| {
        let selected = crate::native_command::selected(matches);
        let harnesses: Vec<_> = crate::native_plugins::installed()
            .into_iter()
            .filter(|h| selected.is_none_or(|s| s == *h))
            .collect();
        if let Some(h) = selected
            && !harnesses.contains(&h)
        {
            return Err(Diagnostic::error(
                "harness-not-found",
                format!("{} is not installed", h.binary()),
                None,
            ));
        }
        for h in harnesses {
            let home = crate::native_plugins::home(&layers, h)?;
            let registered = crate::marketplace_state::read_listing(h, &home.directory)?;
            let installed = crate::marketplace_state::installed_plugins(h, &home)?;
            for (name, native) in registered {
                let declared: Vec<_> = layers
                    .marketplaces
                    .iter()
                    .filter_map(|(logical, market)| {
                        market
                            .value
                            .definition(h)
                            .filter(|d| d.name.as_deref().unwrap_or(logical) == name)
                            .map(|d| (logical, d))
                    })
                    .collect();
                let status = if declared.is_empty() {
                    "undeclared"
                } else if declared.iter().all(|(_, d)| {
                    native
                        .definition
                        .as_ref()
                        .is_some_and(|native| crate::marketplace_state::same_source(native, d))
                }) {
                    "matches"
                } else {
                    "conflict"
                };
                let installed_plugins = installed
                    .iter()
                    .filter(|id| {
                        id.rsplit_once('@')
                            .is_some_and(|(_, market)| market == name)
                    })
                    .count();
                rows.push(Marketplace {
                    harness: h,
                    name,
                    source: native.source,
                    reference: native.reference,
                    logical: declared.iter().map(|(n, _)| (*n).clone()).collect(),
                    status,
                    installed_plugins,
                });
            }
        }
        Ok(())
    });
    if let Err(d) = result {
        diagnostics.push(d);
    }
    if matches.get_flag("json") {
        let mut output = crate::list_command::json_envelope(&diagnostics);
        output["marketplaces"] = serde_json::json!(rows);
        println!("{output}");
    } else {
        crate::list_command::print_row(
            [
                "harness",
                "name",
                "source",
                "ref",
                "logical",
                "match",
                "installed_plugins",
            ]
            .map(str::to_owned)
            .to_vec(),
        );
        for row in rows {
            crate::list_command::print_row(vec![
                row.harness.binary().into(),
                row.name,
                row.source,
                row.reference.unwrap_or_default(),
                row.logical.join(","),
                row.status.into(),
                row.installed_plugins.to_string(),
            ]);
        }
        for d in &diagnostics {
            crate::render(d, false);
        }
    }
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        3
    } else {
        0
    }
}
