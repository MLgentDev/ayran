//! Execute a validated install plan and retain completed progress on failure.
use crate::install_plan::{Change, Outcome, Plan};
use ayran_core::{config::ConfigLayers, diagnostic::Diagnostic, harness::Harness};

pub fn execute(layers: &ConfigLayers, plan: &mut Plan) {
    let result = (|| {
        for change in &mut plan.marketplaces {
            if change.outcome != Outcome::Planned {
                continue;
            }
            change.outcome = Outcome::Failed;
            let home = crate::native_plugins::home(layers, change.harness)?;
            let definition = layers.marketplaces[&change.logical]
                .value
                .definition(change.harness)
                .unwrap();
            // Several logical Marketplaces can request the same native registration.
            let registered = crate::marketplace_state::read(change.harness, &home.directory)?;
            if let Some(native) = registered.get(&change.id) {
                if crate::marketplace_state::same_source(native, definition) {
                    change.outcome = Outcome::Unchanged;
                    change.command = None;
                    continue;
                }
                return Err(Diagnostic {
                    harness: Some(change.harness),
                    ..Diagnostic::error(
                        "marketplace-conflict",
                        format!(
                            "Marketplace {} changed since planning; refusing to replace it",
                            change.id
                        ),
                        None,
                    )
                });
            }
            run(
                change,
                &home,
                &crate::install_plan::marketplace_args(change.harness, definition),
            )?;
            let registered = crate::marketplace_state::read(change.harness, &home.directory)?;
            if !registered
                .get(&change.id)
                .is_some_and(|native| crate::marketplace_state::same_source(native, definition))
            {
                return Err(Diagnostic {
                    harness: Some(change.harness),
                    ..Diagnostic::error(
                        "marketplace-name-mismatch",
                        format!(
                            "Marketplace {} was added but its expected name/source was not registered; registered names: {}",
                            change.id,
                            registered.keys().cloned().collect::<Vec<_>>().join(", ")
                        ),
                        None,
                    )
                });
            }
            change.outcome = Outcome::Added;
        }
        for change in &mut plan.plugins {
            if change.outcome != Outcome::Planned {
                continue;
            }
            change.outcome = Outcome::Failed;
            let home = crate::native_plugins::home(layers, change.harness)?;
            // Two logical names may refer to the same native Plugin. Never reinstall it.
            if crate::marketplace_state::installed_plugins(change.harness, &home)?
                .contains(&change.id)
            {
                change.outcome = Outcome::Unchanged;
                change.command = None;
                continue;
            }
            run(
                change,
                &home,
                &crate::install_plan::plugin_args(change.harness, &change.id),
            )?;
            let disable = if change.harness == Harness::Codex {
                crate::codex_plugin_write::write(
                    &home.directory.join("config.toml"),
                    &change.id,
                    false,
                )
            } else {
                crate::native_command::write_cli(
                    change.harness,
                    &home,
                    "disable",
                    &change.id,
                    &mut None,
                )
                .map(|_| ())
            };
            disable.map_err(|mut d| {
                d.hint = Some(format!(
                    "Plugin was installed; run ayran native plugin disable --{} --id {}",
                    change.harness.binary(),
                    crate::shell_quote(change.id.as_ref())
                ));
                d
            })?;
            change.outcome = Outcome::Installed;
        }
        Ok::<_, Diagnostic>(())
    })();
    if let Err(d) = result {
        plan.diagnostics.push(d);
        for change in plan.marketplaces.iter_mut().chain(&mut plan.plugins) {
            if change.outcome == Outcome::Planned {
                change.outcome = Outcome::Skipped;
                change.command = None;
            }
        }
    }
}

fn run(
    change: &Change,
    home: &crate::harness_home::HarnessHome,
    args: &[String],
) -> Result<(), Diagnostic> {
    // A fresh shared home needs a directory too: commands run from the home.
    std::fs::create_dir_all(&home.directory).map_err(|e| {
        Diagnostic::error(
            "home-unavailable",
            format!("{}: {e}", home.directory.display()),
            None,
        )
    })?;
    let binary = crate::program_on_path(change.harness.binary().as_ref()).ok_or_else(|| {
        Diagnostic::error(
            "harness-not-found",
            format!("{} is not installed", change.harness.binary()),
            None,
        )
    })?;
    let output = std::process::Command::new(binary)
        .args(args)
        .current_dir(&home.directory)
        .env(home.variable, &home.directory)
        .output()
        .map_err(|e| Diagnostic::error("install-write-failed", e.to_string(), None))?;
    if !output.status.success() {
        return Err(Diagnostic {
            harness: Some(change.harness),
            ..Diagnostic::error(
                "install-write-failed",
                format!(
                    "{}: {}: {} {}",
                    change.command.as_deref().unwrap_or(&change.id),
                    output.status,
                    String::from_utf8_lossy(&output.stdout).trim(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
                None,
            )
        });
    }
    Ok(())
}
