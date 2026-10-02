use axoupdater::{AxoUpdater, AxoupdateError, ReleaseSource, ReleaseSourceType};
use clap::{Arg, ArgAction, ArgMatches, Command};

use ayran_core::diagnostic::Diagnostic;

const MANUAL_UPDATE: &str = "rerun the installer from https://github.com/MLgentDev/ayran/releases/latest, or update using your original installation method (for cargo install, reinstall from the latest source)";

pub fn command() -> Command {
    Command::new("update")
        .about("Update ayran to the latest stable release")
        .arg(
            Arg::new("check")
                .long("check")
                .action(ArgAction::SetTrue)
                .help("Check for an update without installing it"),
        )
}

pub fn run(matches: &ArgMatches) -> i32 {
    match update(matches.get_flag("check")) {
        Ok(()) => 0,
        Err(message) => {
            crate::render(&Diagnostic::error("update", message, None), false);
            3
        }
    }
}

fn update(check: bool) -> Result<(), String> {
    // cargo-dist names the receipt and installers after the package, not the binary.
    let mut updater = AxoUpdater::new_for("ayran-cli");
    updater.load_receipt().map_err(|error| match error {
        AxoupdateError::NoReceipt { .. } => {
            format!("No cargo-dist install receipt found for ayran; {MANUAL_UPDATE}.")
        }
        _ => format!("Could not load ayran's install receipt: {error}; {MANUAL_UPDATE}."),
    })?;
    if !updater
        .check_receipt_is_for_this_executable()
        .map_err(|error| format!("Could not verify ayran's install receipt: {error}"))?
    {
        return Err(format!(
            "The install receipt belongs to a different ayran executable; {MANUAL_UPDATE}."
        ));
    }
    updater.set_release_source(ReleaseSource {
        release_type: ReleaseSourceType::GitHub,
        owner: "MLgentDev".to_owned(),
        name: "ayran".to_owned(),
        app_name: "ayran-cli".to_owned(),
    });
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        updater.set_github_token(&token);
    }
    let current = env!("CARGO_PKG_VERSION");
    if check {
        let available = updater
            .is_update_needed_sync()
            .map_err(|error| format!("Could not check for an ayran update: {error}"))?;
        if available {
            println!(
                "An ayran update is available (current version: {current}); run `ayran update` to install it."
            );
        } else {
            println!("ayran {current} is up to date.");
        }
    } else {
        match updater
            .run_sync()
            .map_err(|error| format!("Could not update ayran: {error}"))?
        {
            Some(result) => println!("ayran {current} -> {}", result.new_version),
            None => println!("ayran {current} is up to date."),
        }
    }
    Ok(())
}
