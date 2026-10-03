use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use ayran_core::config::{ConfigLayers, directory_config_paths, user_config_path};
use ayran_core::diagnostic::{Diagnostic, Severity};
use clap::{Arg, ArgAction, ArgGroup, ArgMatches, Command, builder::OsStringValueParser};

#[derive(Clone, Copy, PartialEq)]
enum Scope {
    User,
    Project,
    Local,
}
impl Scope {
    fn name(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Local => "local",
        }
    }
}

fn scope_args(command: Command) -> Command {
    command
        .arg(
            Arg::new("user")
                .help("Use the user Config layer")
                .short('u')
                .long("user")
                .alias("global")
                .short_alias('g')
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("project")
                .help("Use the project Config layer")
                .short('p')
                .long("project")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("local")
                .help("Use the local Config layer")
                .short('l')
                .long("local")
                .action(ArgAction::SetTrue),
        )
        .group(ArgGroup::new("scope").args(["user", "project", "local"]))
}

pub fn command() -> Command {
    Command::new("config")
        .about("Find, list and edit Config layer files")
        .subcommand_required(true)
        .subcommand(scope_args(
            Command::new("path").about("Print a Config layer path"),
        ))
        .subcommand(
            scope_args(Command::new("edit").about("Edit a Config layer file")).arg(
                Arg::new("file")
                    .help("Edit the specified file")
                    .long("file")
                    .value_hint(clap::ValueHint::FilePath)
                    .value_parser(OsStringValueParser::new())
                    .conflicts_with("scope"),
            ),
        )
        .subcommand(
            Command::new("list")
                .about("List Config layer files")
                .alias("ls")
                .arg(
                    Arg::new("json")
                        .help("Print JSON")
                        .long("json")
                        .action(ArgAction::SetTrue),
                ),
        )
}

fn git_root(cwd: &Path) -> Option<PathBuf> {
    let output = ProcessCommand::new("git")
        .current_dir(cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut bytes = output.stdout;
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }
    #[cfg(windows)]
    {
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        String::from_utf8(bytes).ok().map(PathBuf::from)
    }
}

struct Paths {
    user: PathBuf,
    project: PathBuf,
    local: PathBuf,
}
impl Paths {
    fn resolve() -> Result<Self, Diagnostic> {
        let cwd = env::current_dir().map_err(|error| io_error(Path::new("."), error))?;
        let root = git_root(&cwd).unwrap_or_else(|| cwd.clone());
        let directories: Vec<_> = cwd
            .ancestors()
            .take_while(|directory| directory.starts_with(&root))
            .collect();
        let nearest = |name: &str| {
            directories
                .iter()
                .map(|directory| directory.join(name))
                .find(|path| path.exists())
        };
        let project = nearest("ayran.toml").unwrap_or_else(|| root.join("ayran.toml"));
        let local = nearest("ayran.local.toml")
            .unwrap_or_else(|| project.with_file_name("ayran.local.toml"));
        Ok(Self {
            user: user_config_path()?,
            project,
            local,
        })
    }
    fn get(&self, scope: Scope) -> &Path {
        match scope {
            Scope::User => &self.user,
            Scope::Project => &self.project,
            Scope::Local => &self.local,
        }
    }
}
fn io_error(path: &Path, error: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error(
        "config-invalid",
        format!("{}: {error}", path.display()),
        None,
    )
}
fn scope(matches: &ArgMatches) -> Option<Scope> {
    [Scope::User, Scope::Project, Scope::Local]
        .into_iter()
        .find(|scope| matches.get_flag(scope.name()))
}

pub fn run(matches: &ArgMatches) -> i32 {
    let result = execute(matches);
    match result {
        Ok(status) => status,
        Err(diagnostic) => {
            let status = if diagnostic.code == "usage" { 2 } else { 3 };
            super::render(&diagnostic, false);
            status
        }
    }
}
fn execute(matches: &ArgMatches) -> Result<i32, Diagnostic> {
    let (operation, args) = matches.subcommand().expect("clap requires operation");
    let paths = Paths::resolve()?;
    if operation == "list" {
        return list(&paths.user, args.get_flag("json"));
    }
    if operation == "edit"
        && let Some(file) = args.get_one::<std::ffi::OsString>("file")
    {
        let file = PathBuf::from(file);
        let scope = if file
            .file_name()
            .is_some_and(|name| name == "ayran.local.toml")
        {
            Scope::Local
        } else if same_path(&file, &paths.user)? {
            Scope::User
        } else {
            Scope::Project
        };
        return edit(&file, scope);
    }
    let Some(scope) = scope(args) else {
        return Err(super::usage(format!(
            "choose a scope: --user ({}), --project ({}), --local ({})",
            paths.user.display(),
            paths.project.display(),
            paths.local.display()
        )));
    };
    if operation == "path" {
        println!("{}", paths.get(scope).display());
        Ok(0)
    } else {
        edit(paths.get(scope), scope)
    }
}

struct LayerRow {
    scope: Scope,
    path: PathBuf,
    exists: bool,
    valid: bool,
}

fn list(user: &Path, json: bool) -> Result<i32, Diagnostic> {
    let mut layers = vec![(Scope::User, user.to_path_buf())];
    for path in directory_config_paths(user)? {
        if path.exists() {
            let scope = if path
                .file_name()
                .is_some_and(|name| name == "ayran.local.toml")
            {
                Scope::Local
            } else {
                Scope::Project
            };
            layers.push((scope, path));
        }
    }
    let rows: Vec<_> = layers
        .into_iter()
        .map(|(scope, path)| LayerRow {
            exists: path.exists(),
            valid: ConfigLayers::read(&path, scope == Scope::User).is_ok(),
            scope,
            path,
        })
        .collect();
    if json {
        let output: Vec<_> = rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "scope": row.scope.name(), "path": row.path.to_string_lossy(),
                    "exists": row.exists, "valid": row.valid,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string(&output).expect("layer rows serialize")
        );
    } else {
        println!("scope\tpath\texists\tvalid");
        for row in rows {
            println!(
                "{}\t{}\t{}\t{}",
                row.scope.name(),
                row.path.display(),
                row.exists,
                row.valid
            );
        }
    }
    Ok(0)
}

fn same_path(left: &Path, right: &Path) -> Result<bool, Diagnostic> {
    let cwd = env::current_dir().map_err(|error| io_error(Path::new("."), error))?;
    let normalize = |path: &Path| {
        path.canonicalize().unwrap_or_else(|_| {
            let absolute = cwd.join(path);
            let mut normalized = PathBuf::new();
            for component in absolute.components() {
                match component {
                    std::path::Component::CurDir => {}
                    std::path::Component::ParentDir => {
                        normalized.pop();
                    }
                    component => normalized.push(component.as_os_str()),
                }
            }
            normalized
        })
    };
    Ok(normalize(left) == normalize(right))
}

fn template(scope: Scope) -> &'static str {
    match scope {
        Scope::User => {
            "# User layer: settings for every directory.\n# default_harness = \"codex\"\n#\n# [harnesses.codex]\n# model = \"gpt-5\"\n# effort = \"high\"\n# home = \"isolated\"\n#\n# [harnesses.copilot]\n# args = [\"--no-experimental\", \"--allow-all\"]\n#\n# [aliases.work]\n# harness = \"codex\"\n# model = \"gpt-5\"\n# effort = \"high\"\n# description = \"My coding session\"\n#\n# Re-evaluate ayran activate after adding or removing Aliases.\n"
        }
        Scope::Project => {
            "# Project layer: shared settings for this directory.\n# default_harness = \"codex\"\n#\n# [harnesses.codex]\n# model = \"gpt-5\"\n# effort = \"high\"\n#\n# [harnesses.claude]\n# model = \"sonnet\"\n# effort = \"high\"\n#\n# [harnesses.copilot]\n# effort = \"medium\"\n#\n# Nearer Config layers override these settings.\n"
        }
        Scope::Local => {
            "# Local layer: private settings; add this file to .gitignore.\n# [harnesses.codex]\n# effort = \"high\"\n# model = \"gpt-5\"\n#\n# [harnesses.claude]\n# effort = \"high\"\n#\n# [harnesses.copilot]\n# effort = \"medium\"\n# args = [\"--no-experimental\", \"--allow-all\"]\n#\n# Models and effort override the project layer.\n# Aliases belong in the user layer.\n#\n# This layer takes precedence over the project layer beside it.\n"
        }
    }
}

fn alias_names(path: &Path) -> BTreeSet<String> {
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| toml::from_str::<toml::Value>(&contents).ok())
        .and_then(|value| {
            value
                .get("aliases")
                .and_then(toml::Value::as_table)
                .map(|table| table.keys().cloned().collect())
        })
        .unwrap_or_default()
}

fn launch_editor(path: &Path) -> io::Result<std::process::ExitStatus> {
    let editor = env::var_os("VISUAL")
        .filter(|value| !value.is_empty())
        .or_else(|| env::var_os("EDITOR").filter(|value| !value.is_empty()));
    #[cfg(unix)]
    {
        let editor = editor.unwrap_or_else(|| "vi".into());
        let mut script = editor;
        script.push(" \"$1\"");
        ProcessCommand::new("sh")
            .arg("-c")
            .arg(script)
            .arg("ayran-editor")
            .arg(path)
            .status()
    }
    #[cfg(windows)]
    {
        let editor = editor.unwrap_or_else(|| "notepad".into());
        // Keep the filename in an environment variable so shell metacharacters
        // in it are data, rather than commands appended to the editor.
        let mut script = editor;
        script.push(" \"%AYRAN_EDIT_FILE%\"");
        ProcessCommand::new("cmd")
            .args(["/d", "/v:off", "/c"])
            .arg(script)
            .env("AYRAN_EDIT_FILE", path)
            .status()
    }
}

fn notify(code: &'static str, severity: Severity, message: &str, hint: &str) {
    super::render(
        &Diagnostic {
            code,
            severity,
            message: message.into(),
            hint: Some(hint.into()),
            layer: None,
            ..Diagnostic::default()
        },
        false,
    );
}

fn edit(path: &Path, scope: Scope) -> Result<i32, Diagnostic> {
    let before_aliases = if scope == Scope::User {
        alias_names(path)
    } else {
        BTreeSet::new()
    };
    let mut created = false;
    if !path.exists() {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| io_error(path, error))?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| io_error(path, error))?;
        file.write_all(template(scope).as_bytes())
            .map_err(|error| io_error(path, error))?;
        created = true;
    }
    loop {
        let status = launch_editor(path);
        let unchanged =
            created && fs::read(path).is_ok_and(|contents| contents == template(scope).as_bytes());
        if unchanged {
            fs::remove_file(path).map_err(|error| io_error(path, error))?;
        }
        if !status.as_ref().is_ok_and(|status| status.success()) {
            let message = match status {
                Ok(status) => format!("editor exited with {status}"),
                Err(error) => format!("could not start editor: {error}"),
            };
            super::render(&Diagnostic::error("editor-failed", message, None), false);
            return Ok(1);
        }
        if let Err(diagnostic) = ConfigLayers::read(path, scope == Scope::User) {
            if io::stdin().is_terminal() && io::stderr().is_terminal() {
                eprint!("reopen editor? [Y/n] ");
                io::stderr()
                    .flush()
                    .map_err(|error| io_error(path, error))?;
                let mut answer = String::new();
                let read = io::stdin()
                    .read_line(&mut answer)
                    .map_err(|error| io_error(path, error))?;
                if read > 0
                    && matches!(
                        answer.trim().to_ascii_lowercase().as_str(),
                        "" | "y" | "yes"
                    )
                {
                    continue;
                }
            }
            return Err(diagnostic);
        }
        break;
    }
    if scope == Scope::Local && path.exists() {
        let absolute = env::current_dir()
            .map_err(|error| io_error(path, error))?
            .join(path);
        let ignored = ProcessCommand::new("git")
            .current_dir(absolute.parent().unwrap())
            .args(["check-ignore", "-q", "--"])
            .arg(&absolute)
            .output()
            .is_ok_and(|output| output.status.success());
        if !ignored {
            notify(
                "local-not-ignored",
                Severity::Warning,
                "local layer is not ignored by git",
                "add ayran.local.toml to .gitignore",
            );
        }
    }
    if scope == Scope::User && before_aliases != alias_names(path) {
        notify(
            "aliases-changed",
            Severity::Note,
            "Alias names changed",
            "re-eval ayran activate or open a new shell",
        );
    }
    Ok(0)
}
