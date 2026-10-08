use std::fs;
use std::process::{Command, Output};

use tempfile::TempDir;

struct TestHome {
    dir: TempDir,
}

#[test]
fn skill_completion_tracks_profile_defaults_and_explicit_profile_precedence() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        r#"
[profiles.far]
default = true
skills = ['far']
[skills.far]
all = 'far'
[skills.member]
all = 'member'
[skills.path]
all = { path = 'path' }
[aliases.code]
harness = 'claude'
profiles = ['team']
disable = { profiles = ['team'] }
"#,
    );
    write_fixture(
        &home,
        "ayran.toml",
        r#"
default_harness = 'claude'
[disable]
defaults = true
profiles = ['team']
[profiles.team]
default = true
profiles = ['child']
[profiles.child]
skills = ['member', 'path']
"#,
    );
    for (words, expected) in [
        (vec!["ayran", "--no-skill", ""], ""),
        (
            vec!["ayran", "--alias", "code", "--no-skill", ""],
            "member\npath\n",
        ),
        (
            vec!["ayran", "--profile", "team", "--no-skill", ""],
            "member\npath\n",
        ),
        (
            vec![
                "ayran",
                "--profile",
                "team",
                "--no-defaults",
                "--no-skill",
                "",
            ],
            "member\npath\n",
        ),
        (
            vec![
                "ayran",
                "--profile",
                "team",
                "--no-profile",
                "team",
                "--no-skill",
                "",
            ],
            "",
        ),
        (
            vec!["ayran", "--codex", "--profile", "team", "--no-skill", ""],
            "member\n",
        ),
        (vec!["ayran", "--codex", "--skill", ""], "far\nmember\n"),
        (
            vec!["ayran", "--alias", "code", "--codex", "--skill", ""],
            "",
        ),
    ] {
        assert_completion(home.complete(&words), expected);
    }
    fs::remove_file(home.dir.path().join("ayran.toml")).unwrap();
    write_fixture(
        &home,
        "ayran.toml",
        "[profiles.near]\ndefault=true\nprofiles=['child']\n[profiles.child]\nskills=['member']\n",
    );
    assert_completion(home.complete(&["ayran", "--no-skill", ""]), "far\nmember\n");
    assert_completion(
        home.complete(&["ayran", "--no-defaults", "--no-skill", ""]),
        "",
    );
}

#[test]
fn no_skill_completion_follows_defaults_disables_and_recursive_profile_members() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        r#"
default_harness = 'codex'
[aliases.code]
harness = 'codex'
skills = ['explicit']
profiles = ['team']
disable = { skills = ['alias-disabled'] }
[aliases.bare]
harness = 'codex'
skills = ['explicit']
defaults = false
[skills.far]
all = 'far'
default = true
[skills.explicit]
all = 'explicit'
[skills.member]
all = 'member'
description = 'Profile member'
[skills.cli]
all = 'cli'
[profiles.team]
profiles = ['child']
[profiles.child]
skills = ['member', 'alias-disabled']
"#,
    );
    write_fixture(
        &home,
        "ayran.toml",
        r#"
[disable]
defaults = true
skills = ['disabled', 'explicit']
[skills.near]
all = 'near'
default = true
[skills.disabled]
all = 'disabled'
default = true
[skills.alias-disabled]
all = 'alias-disabled'
default = true
[skills.absent]
codex = false
default = true
[skills.missing]
claude = 'missing'
default = true
[skills.path]
all = { path = 'working-copy' }
default = true
"#,
    );
    for (words, expected) in [
        (vec!["ayran", "--no-skill", ""], "alias-disabled\nnear\n"),
        (
            vec!["ayran", "--alias", "code", "--no-skill", ""],
            "explicit\nmember\nnear\n",
        ),
        (
            vec!["ayran", "--alias", "bare", "--no-skill", ""],
            "explicit\n",
        ),
        (
            vec![
                "ayran",
                "--alias",
                "code",
                "--no-defaults",
                "--no-skill",
                "",
            ],
            "explicit\nmember\n",
        ),
        (
            vec!["ayran", "--profile", "team", "--no-skill", ""],
            "alias-disabled\nmember\nnear\n",
        ),
        (
            vec![
                "ayran",
                "--profile",
                "team",
                "--no-profile",
                "child",
                "--no-skill",
                "",
            ],
            "alias-disabled\nnear\n",
        ),
        (
            vec![
                "ayran",
                "--alias",
                "code",
                "--no-skill",
                "explicit",
                "--no-skill",
                "",
            ],
            "member\nnear\n",
        ),
        (
            vec!["ayran", "--alias", "code", "--no-skill=explicit,m"],
            "--no-skill=explicit,member\n",
        ),
        (
            vec!["ayran", "--no-defaults", "--skill", "cli", "--no-skill", ""],
            "cli\n",
        ),
    ] {
        assert_completion(home.complete(&words), expected);
    }
}

#[test]
fn pwsh_activation_registers_and_runs_completion_with_descriptions() {
    if Command::new("pwsh").arg("-Version").output().is_err() {
        eprintln!("skipping: pwsh is not installed");
        return;
    }
    let home = TestHome::new();
    let binary_dir = std::path::Path::new(env!("CARGO_BIN_EXE_ayran"))
        .parent()
        .unwrap();
    let path = std::env::join_paths(
        std::iter::once(binary_dir.to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let output = Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", r#"
$ErrorActionPreference = 'Stop'
$PSNativeCommandArgumentPassing = 'Legacy'
New-Item -ItemType File -Name completion-probe.toml | Out-Null
$originalTabExpansion2 = (Get-Item function:TabExpansion2).ScriptBlock
$global:customCompletionCalls = 0
# Install a pre-existing customization to verify that activation delegates to it.
$customBody = $originalTabExpansion2.Ast.ParamBlock.Extent.Text + "`n`$global:customCompletionCalls++`n& `$originalTabExpansion2 @PSBoundParameters"
Set-Item function:TabExpansion2 ([scriptblock]::Create($customBody))
$otherLine = 'Get-ChildItem completion-probe'
$otherBefore = (TabExpansion2 $otherLine $otherLine.Length).CompletionMatches.CompletionText -join '|'
Invoke-Expression (ayran activate pwsh | Out-String)
function Complete($line, $cursor = $line.Length) {
    @((TabExpansion2 $line $cursor).CompletionMatches)
}
$matches = @(Complete 'ayran config ed ignored' 15)
if ($matches.Count -ne 1 -or $matches[0].CompletionText -cne 'edit' -or $matches[0].ToolTip -cne 'Edit a Config layer file') {
    throw "edit completion: $($matches | Out-String)"
}
foreach ($line in 'ayran --harness co', 'ayran --effort=hi', 'ayran activate p', 'ayran config --j') {
    $matches = @(Complete $line)
    $words = $line.Split(' ')
    $expected = @(ayran __complete -- @words)
    if (($matches.CompletionText -join '|') -cne ($expected -join '|')) { throw "candidate parity: $line" }
    foreach ($match in $matches) { if (!$match.ToolTip) { throw "missing description: $line" } }
}
$matches = @(Complete "ayran --model 'a model with spaces' con")
if ($matches[0].CompletionText -cne 'config') { throw 'quoted model' }
foreach ($line in 'ayran -- ', 'ayran -- comp', 'ayran unknown ', 'ayran --model comp') {
    if (@(Complete $line).Count) { throw "unexpected completion: $line" }
}
$parseTokens = $null
$parseErrors = $null
foreach ($line in 'ayran -- ', 'ayran config ed') {
    $parsed = [System.Management.Automation.Language.Parser]::ParseInput($line, [ref]$parseTokens, [ref]$parseErrors)
    $matches = (TabExpansion2 -ast $parsed -tokens $parseTokens -positionOfCursor $parsed.Extent.EndScriptPosition -options @{}).CompletionMatches
    if ($line -eq 'ayran -- ' -and $matches.Count) { throw 'AST fallback' }
    if ($line -eq 'ayran config ed' -and $matches[0].CompletionText -cne 'edit') { throw 'AST candidate' }
}
New-Item -ItemType Directory -Force config/ayran | Out-Null
Set-Content config/ayran/ayran.toml "[aliases.cr]`nharness = 'claude'`nprofiles = ['base']`n[aliases.my-task]`nharness = 'codex'`nprofiles = ['base']`n[profiles.base]`nprofiles = ['tools']`n[profiles.tools]`ndefault = true`ndescription = 'Profile tools'`n[plugins.review]`nall = 'review@m'`ndescription = 'Review tools'`n[skills.tdd]`nall = 'tdd'`ndefault = true`ndescription = 'Test first'`n[skills.lint]`nall = 'lint'`ndescription = 'Lint code'`n[mcp.alpha]`nall = 'native-alpha'`ndefault = true`ndescription = 'Alpha server'`n[mcp.beta]`nall = { command = 'npx' }`ndescription = 'Beta server'"
Invoke-Expression (ayran activate pwsh | Out-String)
$matches = @(Complete 'ayran --codex --plugin re')
if ($matches.Count -ne 1 -or $matches[0].CompletionText -cne 'review' -or $matches[0].ToolTip -cne 'Review tools') { throw 'Plugin description' }
foreach ($line in 'ayran --profile to', 'ayran --no-profile to') {
    $matches = @(Complete $line)
    if ($matches.Count -ne 1 -or $matches[0].CompletionText -cne 'tools' -or $matches[0].ToolTip -cne 'Profile tools') { throw 'Profile description' }
}
foreach ($name in 'cr', 'my-task') {
    $matches = @(Complete "$name ")
    if (($matches.CompletionText -join ' ') -cne '--dry-run --effort --help --json --mcp --model --no-defaults --no-harness-args --no-mcp --no-plugin --no-profile --no-skill --plugin --preset --profile --quiet --skill --version -V -e -h -m -p -q') {
        throw "Alias flags: $name $($matches.CompletionText -join ' ')"
    }
    foreach ($match in $matches) { if (!$match.ToolTip) { throw "missing Alias description: $name" } }
    foreach ($suffix in '--effort hi', '--effort=hi') {
        $matches = @(Complete "$name $suffix")
        if ($matches.Count -ne 1 -or $matches[0].ToolTip -cne 'Choose the reasoning Effort: high') { throw "Alias effort: $name $suffix" }
    }
    $matches = @(Complete "$name --plugin re")
    if ($matches.Count -ne 1 -or $matches[0].CompletionText -cne 'review' -or $matches[0].ToolTip -cne 'Review tools') { throw 'Alias Plugin completion' }
    $matches = @(Complete "$name --profile base,t")
    if ($matches.Count -ne 1 -or $matches[0].CompletionText -cne 'base,tools' -or $matches[0].ToolTip -cne 'Profile tools') { throw 'Alias Profile completion' }
    $matches = @(Complete "$name --no-defaults --no-profile=ba")
    if ($matches.Count -ne 1 -or $matches[0].CompletionText -cne '--no-profile=base') { throw 'Alias Profile Disable' }
    foreach ($line in "$name --skill tdd,l", "$name --no-skill t") {
        $matches = @(Complete $line)
        $expected = if ($line.Contains('--skill')) { 'tdd,lint' } else { 'tdd' }
        $description = if ($expected -eq 'tdd') { 'Test first' } else { 'Lint code' }
        if ($matches.Count -ne 1 -or $matches[0].CompletionText -cne $expected -or $matches[0].ToolTip -cne $description) { throw "Alias Skill completion: $line" }
    }
    foreach ($suffix in 'config ', 'unknown ', '-- ') {
        if (@(Complete "$name $suffix").Count) { throw "unexpected Alias completion: $name $suffix" }
    }
}
foreach ($name in 'ayran', 'cr', 'my-task') {
    $matches = @(Complete "$name --mcp alpha,b")
    if ($matches.Count -ne 1 -or $matches[0].CompletionText -cne 'alpha,beta' -or $matches[0].ToolTip -cne 'Beta server') { throw 'MCP comma completion' }
    $matches = @(Complete "$name --no-mcp=a")
    if ($matches.Count -ne 1 -or $matches[0].CompletionText -cne '--no-mcp=alpha' -or $matches[0].ToolTip -cne 'Alpha server') { throw 'MCP Disable completion' }
    if (@(Complete "$name --mcp alpha,beta --mcp b").Count) { throw 'MCP duplicate' }
}
New-Item -ItemType File -Name "my config'one.toml" | Out-Null
$matches = @(Complete 'ayran config edit --file my')
if ($matches.Count -ne 1 -or $matches[0].CompletionText -cne "'my config''one.toml'" -or $matches[0].ToolTip -cne 'File') { throw 'quoted filename completion' }
$matches = @(Complete "ayran config edit --file 'my config")
if ($matches.Count -ne 1 -or $matches[0].CompletionText -cne "'my config''one.toml'") { throw 'quoted filename prefix' }
$matches = @(Complete "ayran config edit --file 'my ")
if ($matches.Count -ne 1 -or $matches[0].CompletionText -cne "'my config''one.toml'") { throw 'space in quoted filename prefix' }
$otherAfter = (TabExpansion2 $otherLine $otherLine.Length).CompletionMatches.CompletionText -join '|'
if (!$otherBefore -or $otherAfter -cne $otherBefore) { throw 'other command completion changed' }
if ($global:customCompletionCalls -lt 2) { throw 'previous completion function was bypassed' }
if ((TabExpansion2 -inputScript 'ayran config ed' -options @{}).CompletionMatches[0].CompletionText -cne 'edit') { throw 'default cursor' }
Set-Content ayran.toml '[broken'
if (@(Complete 'ayran comp').Count) { throw 'invalid config completion' }
function ayran { throw 'unavailable executable' }
if (@(Complete 'ayran comp').Count) { throw 'failed command completion' }
"#])
        .current_dir(home.dir.path())
        .env("HOME", home.dir.path())
        .env("XDG_CONFIG_HOME", home.dir.path().join("config"))
        .env("APPDATA", home.dir.path().join("config"))
        .env("LOCALAPPDATA", home.dir.path().join("state"))
        .env("USERPROFILE", home.dir.path())
        .env("PATH", path)
        .output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        output.stdout.is_empty() && output.stderr.is_empty(),
        "{output:?}"
    );
}

impl TestHome {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ayran"));
        command
            .current_dir(self.dir.path())
            .env("HOME", self.dir.path())
            .env("USERPROFILE", self.dir.path())
            .env("APPDATA", self.dir.path().join("config"))
            .env("LOCALAPPDATA", self.dir.path().join("state"))
            .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
            .env("CODEX_HOME", self.dir.path().join("codex"))
            .env("XDG_STATE_HOME", self.dir.path().join("state"))
            .env_remove("AYRAN_CONFIG");
        command
    }

    fn complete(&self, words: &[&str]) -> Output {
        self.command()
            .args(["__complete", "--"])
            .args(words)
            .output()
            .unwrap()
    }
}

#[test]
fn file_values_complete_relative_absolute_and_equals_paths() {
    let home = TestHome::new();
    fs::create_dir(home.dir.path().join("configs")).unwrap();
    fs::write(home.dir.path().join("configs/my config.toml"), "").unwrap();
    fs::write(home.dir.path().join("configs/other.toml"), "").unwrap();
    for (value, expected) in [
        ("configs/my", "configs/my config.toml".to_owned()),
        ("./configs/my", "./configs/my config.toml".to_owned()),
        ("conf", "configs/".to_owned()),
        ("missing/", String::new()),
    ] {
        for equals in [false, true] {
            let flag = format!("--file={value}");
            let words = if equals {
                vec!["ayran", "config", "edit", flag.as_str()]
            } else {
                vec!["ayran", "config", "edit", "--file", value]
            };
            let output = home.complete(&words);
            assert!(
                output.status.success() && output.stderr.is_empty(),
                "{output:?}"
            );
            let expected = if expected.is_empty() {
                String::new()
            } else if equals {
                format!("--file={expected}\n")
            } else {
                format!("{expected}\n")
            };
            assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        }
    }
    let absolute = home.dir.path().join("configs/my");
    let output = home.complete(&[
        "ayran",
        "config",
        "edit",
        "--file",
        absolute.to_str().unwrap(),
    ]);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{} config.toml\n", absolute.display())
    );
    assert!(
        home.complete(&["ayran", "--model", "conf"])
            .stdout
            .is_empty()
    );
}

#[test]
fn hidden_command_returns_descriptions_when_requested() {
    let home = TestHome::new();
    let output = home
        .command()
        .args([
            "__complete",
            "--descriptions",
            "--",
            "ayran",
            "config",
            "ed",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(output.stdout, b"edit\tEdit a Config layer file\n");
}

#[test]
fn hidden_command_returns_plain_candidates() {
    let output = TestHome::new().complete(&["ayran", "config", "ed"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(output.stdout, b"edit\n");
}

#[test]
fn update_command_and_check_flag_complete() {
    let home = TestHome::new();
    assert_completion(home.complete(&["ayran", "up"]), "update\n");
    assert_completion(home.complete(&["ayran", "update", "--ch"]), "--check\n");
}

#[test]
fn completion_is_silent_on_invalid_config_or_malformed_requests() {
    let home = TestHome::new();
    for args in [
        vec!["__complete"],
        vec!["__complete", "--bad"],
        vec!["__complete", "--", "ayran", "unknown", ""],
    ] {
        let output = home.command().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(
            output.stdout.is_empty() && output.stderr.is_empty(),
            "{output:?}"
        );
    }
    let user = home.dir.path().join("config/ayran/ayran.toml");
    fs::create_dir_all(user.parent().unwrap()).unwrap();
    fs::write(&user, "[broken").unwrap();
    let output = home.complete(&["ayran", "con"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        output.stdout.is_empty() && output.stderr.is_empty(),
        "{output:?}"
    );
    fs::remove_file(user).unwrap();
    fs::write(home.dir.path().join("ayran.toml"), "[broken").unwrap();
    let output = home.complete(&["ayran", "con"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        output.stdout.is_empty() && output.stderr.is_empty(),
        "{output:?}"
    );
}

#[cfg(unix)]
#[test]
fn bash_activation_registers_and_runs_completion_without_aliases() {
    if Command::new("bash").arg("--version").output().is_err() {
        eprintln!("skipping: bash is not installed");
        return;
    }
    let home = TestHome::new();
    let output = Command::new("bash")
        .args(["--noprofile", "--norc", "-c", r#"
set -e
ayran() { "$AYRAN_BIN" "$@"; }
eval "$(ayran activate bash)"
registration=$(complete -p ayran)
completion_function=${registration#complete -F }
completion_function=${completion_function% ayran}
COMP_WORDS=(ayran config ed ignored)
COMP_CWORD=2
"$completion_function"
[[ ${#COMPREPLY[@]} == 1 && ${COMPREPLY[0]} == edit ]]
COMP_WORDS=(ayran --harness co)
COMP_CWORD=2
"$completion_function"
[[ ${COMPREPLY[*]} == 'codex copilot' ]]
# Bash splits equals at its default word break before calling the hook.
COMP_WORDS=(ayran --effort = hi)
COMP_CWORD=3
"$completion_function"
[[ ${COMPREPLY[*]} == high ]]
COMP_WORDS=(ayran --effort = high --e)
COMP_CWORD=4
"$completion_function"
[[ ${#COMPREPLY[@]} == 0 ]]
COMP_WORDS=(ayran --effort =hi)
COMP_CWORD=2
"$completion_function"
[[ ${COMPREPLY[*]} == high ]]
COMP_WORDS=(ayran --model 'a model with spaces' con)
COMP_CWORD=3
"$completion_function"
[[ ${COMPREPLY[*]} == config ]]
COMP_WORDS=(ayran config '')
COMP_CWORD=2
"$completion_function"
[[ " ${COMPREPLY[*]} " == *' edit '* && " ${COMPREPLY[*]} " == *' path '* && " ${COMPREPLY[*]} " == *' list '* ]]
COMP_WORDS=(ayran -- con)
COMP_CWORD=2
"$completion_function"
[[ ${#COMPREPLY[@]} == 0 ]]
"#])
        .current_dir(home.dir.path())
        .env("HOME", home.dir.path())
        .env("XDG_CONFIG_HOME", home.dir.path().join("config"))
        .env("AYRAN_BIN", env!("CARGO_BIN_EXE_ayran"))
        .output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        output.stdout.is_empty() && output.stderr.is_empty(),
        "{output:?}"
    );
}

#[cfg(unix)]
#[test]
fn bash_alias_completion_offers_only_launch_flags() {
    if Command::new("bash").arg("--version").output().is_err() {
        eprintln!("skipping: bash is not installed");
        return;
    }
    let home = TestHome::new();
    let user = home.dir.path().join("config/ayran/ayran.toml");
    fs::create_dir_all(user.parent().unwrap()).unwrap();
    fs::write(
        user,
        "[aliases.cr]\nharness = 'claude'\nprofiles = ['base']\n[aliases.my-task]\nharness = 'codex'\nprofiles = ['base']\n[profiles.base]\nprofiles = ['tools']\n[profiles.tools]\ndefault = true\ndescription = 'Profile tools'\n[plugins.review]\nall = 'review@m'\n[skills.tdd]\nall = 'tdd'\ndefault = true\n[skills.lint]\nall = 'lint'\n[mcp.alpha]\nall = 'native-alpha'\ndefault = true\n[mcp.beta]\nall = { command = 'npx' }\n",
    )
    .unwrap();
    let output = Command::new("bash")
        .args([
            "--noprofile",
            "--norc",
            "-c",
            r#"
set -e
ayran() { "$AYRAN_BIN" "$@"; }
eval "$(ayran activate bash)"
complete -p ayran >/dev/null
for name in cr my-task; do
    registration=$(complete -p "$name")
    completion_function=${registration#complete -F }
    completion_function=${completion_function% "$name"}
    COMP_WORDS=("$name" '' ignored)
    COMP_CWORD=1
    "$completion_function"
    [[ ${COMPREPLY[*]} == '--dry-run --effort --help --json --mcp --model --no-defaults --no-harness-args --no-mcp --no-plugin --no-profile --no-skill --plugin --preset --profile --quiet --skill --version -V -e -h -m -p -q' ]]
    expected=("${COMPREPLY[@]}")
    direct=()
    while IFS= read -r candidate; do direct+=("$candidate"); done < <(ayran __complete -- ayran --alias "$name" '')
    [[ ${direct[*]} == "${expected[*]}" ]]
    COMP_WORDS=("$name" --effort hi)
    COMP_CWORD=2
    "$completion_function"
    [[ ${COMPREPLY[*]} == high ]]
    COMP_WORDS=("$name" --effort = hi)
    COMP_CWORD=3
    "$completion_function"
    [[ ${COMPREPLY[*]} == high ]]
    COMP_WORDS=("$name" --effort high --e)
    COMP_CWORD=3
    "$completion_function"
    [[ ${#COMPREPLY[@]} == 0 ]]
    COMP_WORDS=("$name" --model 'a model with spaces' --d)
    COMP_CWORD=3
    "$completion_function"
    [[ ${COMPREPLY[*]} == --dry-run ]]
    COMP_WORDS=("$name" --plugin review,r)
    COMP_CWORD=2
    "$completion_function"
    [[ ${#COMPREPLY[@]} == 0 ]]
    COMP_WORDS=("$name" --plugin re)
    COMP_CWORD=2
    "$completion_function"
    [[ ${COMPREPLY[*]} == review ]]
    COMP_WORDS=("$name" --profile base,t)
    COMP_CWORD=2
    "$completion_function"
    [[ ${COMPREPLY[*]} == base,tools ]]
    COMP_WORDS=("$name" --no-defaults --no-profile = ba)
    COMP_CWORD=4
    "$completion_function"
    [[ ${COMPREPLY[*]} == base ]]
    COMP_WORDS=(ayran --profile = to)
    COMP_CWORD=3
    "$completion_function"
    [[ ${COMPREPLY[*]} == tools ]]
    COMP_WORDS=(ayran --no-profile to)
    COMP_CWORD=2
    "$completion_function"
    [[ ${COMPREPLY[*]} == tools ]]
    COMP_WORDS=("$name" --skill tdd,l)
    COMP_CWORD=2
    "$completion_function"
    [[ ${COMPREPLY[*]} == tdd,lint ]]
    COMP_WORDS=("$name" --no-skill = t)
    COMP_CWORD=3
    "$completion_function"
    [[ ${COMPREPLY[*]} == tdd ]]
    COMP_WORDS=(ayran --skill t)
    COMP_CWORD=2
    "$completion_function"
    [[ ${COMPREPLY[*]} == tdd ]]
    for mcp_command in ayran "$name"; do
        COMP_WORDS=("$mcp_command" --mcp alpha,b)
        COMP_CWORD=2
        "$completion_function"
        [[ ${COMPREPLY[*]} == alpha,beta ]]
        COMP_WORDS=("$mcp_command" --no-mcp = a)
        COMP_CWORD=3
        "$completion_function"
        [[ ${COMPREPLY[*]} == alpha ]]
        COMP_WORDS=("$mcp_command" --mcp alpha,beta --mcp b)
        COMP_CWORD=4
        "$completion_function"
        [[ ${#COMPREPLY[@]} == 0 ]]
    done
    for word in config unknown --; do
        COMP_WORDS=("$name" "$word" '')
        COMP_CWORD=2
        "$completion_function"
        [[ ${#COMPREPLY[@]} == 0 ]]
    done
done
"#,
        ])
        .current_dir(home.dir.path())
        .env("HOME", home.dir.path())
        .env("XDG_CONFIG_HOME", home.dir.path().join("config"))
        .env("AYRAN_BIN", env!("CARGO_BIN_EXE_ayran"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        output.stdout.is_empty() && output.stderr.is_empty(),
        "{output:?}"
    );
}

#[cfg(unix)]
#[test]
fn completion_is_silent_when_the_working_directory_is_gone() {
    if Command::new("bash").arg("--version").output().is_err() {
        eprintln!("skipping: bash is not installed");
        return;
    }
    let home = TestHome::new();
    let cwd = home.dir.path().join("removed");
    fs::create_dir(&cwd).unwrap();
    let output = Command::new("bash")
        .args([
            "--noprofile",
            "--norc",
            "-c",
            r#"
cd "$REMOVED_CWD" || exit 1
rmdir "$REMOVED_CWD" || exit 1
"$AYRAN_BIN" __complete -- ayran con
"#,
        ])
        .current_dir(home.dir.path())
        .env("HOME", home.dir.path())
        .env("XDG_CONFIG_HOME", home.dir.path().join("config"))
        .env("AYRAN_BIN", env!("CARGO_BIN_EXE_ayran"))
        .env("REMOVED_CWD", cwd)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        output.stdout.is_empty() && output.stderr.is_empty(),
        "{output:?}"
    );
}

#[cfg(unix)]
#[test]
fn zsh_activation_registers_and_runs_completion_with_descriptions() {
    if Command::new("zsh").arg("--version").output().is_err() {
        eprintln!("skipping: zsh is not installed");
        return;
    }
    let home = TestHome::new();
    let output = Command::new("zsh")
        .args([
            "-f",
            "-c",
            r#"
set -e
ayran() { "$AYRAN_BIN" "$@"; }
eval "$(ayran activate zsh)"
[[ ${_comps[ayran]} == _ayran_complete ]]
# Capture the public zsh display boundary outside an interactive ZLE session.
_describe() { captured=("${(@P)2}"); }
words=(ayran config ed ignored)
CURRENT=3
"${_comps[ayran]}"
[[ ${#captured} == 1 && ${captured[1]} == 'edit:Edit a Config layer file' ]]

for request in '--harness co' '--effort=hi' 'activate z' 'config --j'; do
    words=(ayran ${=request})
    CURRENT=${#words}
    captured=()
    "${_comps[ayran]}"
    plain=("${(@f)$(ayran __complete -- "${words[@]}")}")
    actual=()
    for candidate in "${captured[@]}"; do
        actual+=("${candidate%%:*}")
        [[ ${candidate#*:} != '' ]]
    done
    [[ ${actual[*]} == "${plain[*]}" ]]
done
words=(ayran --model 'a model with spaces' con)
CURRENT=4
"${_comps[ayran]}"
[[ ${captured[1]} == 'config:Find, list and edit Config layer files' ]]
for word in unknown --; do
    words=(ayran "$word" '')
    CURRENT=3
    captured=()
    "${_comps[ayran]}"
    [[ ${#captured} == 0 ]]
done
mkdir -p config/ayran
print -r -- "[aliases.cr]
harness = 'claude'
profiles = ['base']
[aliases.my-task]
harness = 'codex'
profiles = ['base']
[profiles.base]
profiles = ['tools']
[profiles.tools]
default = true
description = 'Profile tools'
[plugins.review]
all = 'review@m'
description = 'Review tools'
[skills.tdd]
all = 'tdd'
default = true
description = 'Test first'
[skills.lint]
all = 'lint'
description = 'Lint code'
[mcp.alpha]
all = 'native-alpha'
default = true
description = 'Alpha server'
[mcp.beta]
all = { command = 'npx' }
description = 'Beta server'" > config/ayran/ayran.toml
eval "$(ayran activate zsh)"
for flag in --profile --no-profile; do
    words=(ayran "$flag" to)
    CURRENT=3
    "${_comps[ayran]}"
    [[ ${captured[*]} == 'tools:Profile tools' ]]
done
for name in cr my-task; do
    [[ ${_comps[$name]} == _ayran_complete ]]
    words=("$name" '' ignored)
    CURRENT=2
    "${_comps[$name]}"
    actual=()
    for candidate in "${captured[@]}"; do
        actual+=("${candidate%%:*}")
        [[ ${candidate#*:} != '' ]]
    done
    [[ ${actual[*]} == '--dry-run --effort --help --json --mcp --model --no-defaults --no-harness-args --no-mcp --no-plugin --no-profile --no-skill --plugin --preset --profile --quiet --skill --version -V -e -h -m -p -q' ]]
    words=("$name" --effort hi)
    CURRENT=3
    "${_comps[$name]}"
    [[ ${captured[*]} == 'high:Choose the reasoning Effort: high' ]]
    words=("$name" --effort=hi)
    CURRENT=2
    "${_comps[$name]}"
    [[ ${captured[*]} == '--effort=high:Choose the reasoning Effort: high' ]]
    words=("$name" --plugin re)
    CURRENT=3
    "${_comps[$name]}"
    [[ ${captured[*]} == 'review:Review tools' ]]
    words=("$name" --profile base,t)
    CURRENT=3
    "${_comps[$name]}"
    [[ ${captured[*]} == 'base,tools:Profile tools' ]]
    words=("$name" --no-defaults --no-profile=ba)
    CURRENT=3
    "${_comps[$name]}"
    [[ ${captured[*]} == '--no-profile=base:Profile' ]]
    words=("$name" --skill tdd,l)
    CURRENT=3
    "${_comps[$name]}"
    [[ ${captured[*]} == 'tdd,lint:Lint code' ]]
    words=("$name" --no-skill=t)
    CURRENT=2
    "${_comps[$name]}"
    [[ ${captured[*]} == '--no-skill=tdd:Test first' ]]
    words=(ayran --skill t)
    CURRENT=3
    "${_comps[ayran]}"
    [[ ${captured[*]} == 'tdd:Test first' ]]
    for mcp_command in ayran "$name"; do
        words=("$mcp_command" --mcp alpha,b)
        CURRENT=3
        "${_comps[$mcp_command]}"
        [[ ${captured[*]} == 'alpha,beta:Beta server' ]]
        words=("$mcp_command" --no-mcp=a)
        CURRENT=2
        "${_comps[$mcp_command]}"
        [[ ${captured[*]} == '--no-mcp=alpha:Alpha server' ]]
        words=("$mcp_command" --mcp alpha,beta --mcp b)
        CURRENT=5
        captured=()
        "${_comps[$mcp_command]}"
        [[ ${#captured} == 0 ]]
    done
    for word in config unknown --; do
        words=("$name" "$word" '')
        CURRENT=3
        captured=()
        "${_comps[$name]}"
        [[ ${#captured} == 0 ]]
    done
done
touch 'my config:one\two.toml'
words=(ayran config edit --file my)
CURRENT=5
"${_comps[ayran]}"
[[ ${captured[1]} == 'my config\:one\\two.toml:File' ]]
touch $'tab\tfile.toml'
words=(ayran config edit --file tab)
CURRENT=5
"${_comps[ayran]}"
[[ ${captured[1]} == $'tab\tfile.toml:File' ]]
print -r -- '[broken' > ayran.toml
words=(ayran con)
CURRENT=2
captured=()
"${_comps[ayran]}"
[[ ${#captured} == 0 ]]
"#,
        ])
        .current_dir(home.dir.path())
        .env("HOME", home.dir.path())
        .env("XDG_CONFIG_HOME", home.dir.path().join("config"))
        .env("AYRAN_BIN", env!("CARGO_BIN_EXE_ayran"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        output.stdout.is_empty() && output.stderr.is_empty(),
        "{output:?}"
    );
}

fn write_fixture(home: &TestHome, path: &str, contents: &str) {
    let path = home.dir.path().join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn assert_completion(output: Output, expected: &str) {
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{output:?}"
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
}

#[test]
fn model_hints_include_overridden_layers_and_only_matching_aliases() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        "default_harness = 'copilot'\n[harnesses.claude]\nmodel = 'sonnet'\n[harnesses.codex]\nmodel = 'codex-user'\n[aliases.review]\nharness = 'claude'\nmodel = 'opus'\n[aliases.code]\nharness = 'codex'\nmodel = 'codex-alias'\n",
    );
    write_fixture(
        &home,
        "ayran.toml",
        "default_harness = 'claude'\n[harnesses.claude]\nmodel = 'haiku'\n",
    );
    write_fixture(
        &home,
        "project/ayran.local.toml",
        "[harnesses.claude]\nmodel = 'sonnet-local'\n",
    );
    let complete = |words: &[&str]| {
        home.command()
            .current_dir(home.dir.path().join("project"))
            .args(["__complete", "--"])
            .args(words)
            .output()
            .unwrap()
    };
    for words in [
        vec!["ayran", "-m", ""],
        vec!["ayran", "--claude", "--model", ""],
        vec!["ayran", "--harness=claude", "--model", ""],
        vec!["ayran", "--alias", "review", "--model", ""],
    ] {
        assert_completion(complete(&words), "haiku\nopus\nsonnet\nsonnet-local\n");
    }
    assert_completion(
        complete(&["ayran", "--model=so"]),
        "--model=sonnet\n--model=sonnet-local\n",
    );
    assert_completion(
        complete(&["ayran", "--model", "=", "so"]),
        "sonnet\nsonnet-local\n",
    );
    assert_completion(
        complete(&["ayran", "--harness", "codex", "-m", ""]),
        "codex-alias\ncodex-user\n",
    );
    assert_completion(complete(&["ayran", "--copilot", "--model", ""]), "");
    assert_completion(
        complete(&["ayran", "--alias", "missing", "--model", ""]),
        "",
    );
    assert_completion(
        complete(&["ayran", "--alias", "review", "--codex", "--model", ""]),
        "",
    );
    assert_completion(
        complete(&["ayran", "-m", "arbitrary", "--effort", "hi"]),
        "high\n",
    );
    let output = home
        .command()
        .current_dir(home.dir.path().join("project"))
        .args([
            "__complete",
            "--descriptions",
            "--",
            "ayran",
            "--model",
            "ha",
        ])
        .output()
        .unwrap();
    assert_completion(
        output,
        &format!("haiku\t{}\n", home.dir.path().join("ayran.toml").display()),
    );
}

#[test]
fn codex_cache_hints_are_visible_deduplicated_and_described() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        "[harnesses.codex]\nmodel = 'shared'\n[aliases.code]\nharness = 'codex'\nmodel = 'alias-model'\n",
    );
    write_fixture(
        &home,
        "codex/models_cache.json",
        r#"{"models":[
        {"slug":"shared","visibility":"list","description":"Cache description"},
        {"slug":"shared","visibility":"list"},
        {"slug":"alias-model","visibility":"list"},
        {"slug":"cache-only","visibility":"list"},
        {"slug":"hidden","visibility":"hide","description":"Hidden"}
    ]}"#,
    );
    assert_completion(home.complete(&["ayran", "--model", ""]), "");
    assert_completion(
        home.complete(&["ayran", "--codex", "-m", ""]),
        "alias-model\ncache-only\nshared\n",
    );
    assert_completion(
        home.complete(&["ayran", "--alias", "code", "--model=ca"]),
        "--model=cache-only\n",
    );
    assert_completion(
        home.command()
            .args([
                "__complete",
                "--descriptions",
                "--",
                "ayran",
                "--codex",
                "-m",
                "",
            ])
            .output()
            .unwrap(),
        "alias-model\tAlias code\ncache-only\tCodex model cache\nshared\tCache description\n",
    );
    for harness in ["--claude", "--copilot"] {
        assert_completion(home.complete(&["ayran", harness, "-m", ""]), "");
    }
}

#[test]
fn unusable_codex_cache_preserves_config_hints_silently() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        "[harnesses.codex]\nmodel = 'configured'\n",
    );
    assert_completion(
        home.complete(&["ayran", "--codex", "-m", ""]),
        "configured\n",
    );
    for cache in [
        "broken json",
        "[]",
        r#"{"models":{}}"#,
        r#"{"models":[{"slug":"cache","visibility":"list"}, {"slug":42,"visibility":"list"}]}"#,
    ] {
        write_fixture(&home, "codex/models_cache.json", cache);
        assert_completion(
            home.complete(&["ayran", "--codex", "-m", ""]),
            "configured\n",
        );
    }
    fs::remove_file(home.dir.path().join("codex/models_cache.json")).unwrap();
    fs::create_dir(home.dir.path().join("codex/models_cache.json")).unwrap();
    assert_completion(
        home.complete(&["ayran", "--codex", "-m", ""]),
        "configured\n",
    );
}

#[test]
fn codex_cache_location_follows_isolated_then_codex_home_then_home() {
    let home = TestHome::new();
    for (directory, model) in [
        ("state/ayran/homes/codex", "isolated"),
        ("codex", "override"),
        (".codex", "fallback"),
        (".local/state/ayran/homes/codex", "isolated-fallback"),
    ] {
        write_fixture(
            &home,
            &format!("{directory}/models_cache.json"),
            &format!(r#"{{"models":[{{"slug":"{model}","visibility":"list"}}]}}"#),
        );
    }
    assert_completion(home.complete(&["ayran", "--codex", "-m", ""]), "override\n");
    assert_completion(
        home.command()
            .env_remove("CODEX_HOME")
            .args(["__complete", "--", "ayran", "--codex", "-m", ""])
            .output()
            .unwrap(),
        "fallback\n",
    );
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        "[harnesses.codex]\nhome = 'isolated'\n",
    );
    assert_completion(home.complete(&["ayran", "--codex", "-m", ""]), "isolated\n");
    // Unix falls back to HOME; Windows requires LOCALAPPDATA for Isolated homes.
    #[cfg(not(windows))]
    let without_state = "isolated-fallback\n";
    #[cfg(windows)]
    let without_state = "";
    assert_completion(
        home.command()
            .env_remove("XDG_STATE_HOME")
            .env_remove("LOCALAPPDATA")
            .args(["__complete", "--", "ayran", "--codex", "-m", ""])
            .output()
            .unwrap(),
        without_state,
    );
    fs::remove_file(
        home.dir
            .path()
            .join("state/ayran/homes/codex/models_cache.json"),
    )
    .unwrap();
    assert_completion(home.complete(&["ayran", "--codex", "-m", ""]), "");
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        "[harnesses.codex]\nhome = 'shared'\n",
    );
    assert_completion(home.complete(&["ayran", "--codex", "-m", ""]), "override\n");
}

#[cfg(unix)]
#[test]
fn model_completion_does_not_run_a_harness() {
    use std::os::unix::fs::PermissionsExt;

    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        "[harnesses.codex]\nmodel = 'configured'\n",
    );
    for harness in ["claude", "codex", "copilot"] {
        let path = format!("bin/{harness}");
        write_fixture(&home, &path, "#!/bin/sh\n: > harness-was-run\n");
        fs::set_permissions(
            home.dir.path().join(path),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        assert_completion(
            home.command()
                .env("PATH", home.dir.path().join("bin"))
                .args([
                    "__complete",
                    "--",
                    "ayran",
                    "--harness",
                    harness,
                    "--model",
                    "",
                ])
                .output()
                .unwrap(),
            if harness == "codex" {
                "configured\n"
            } else {
                ""
            },
        );
    }
    assert!(!home.dir.path().join("harness-was-run").exists());
}

#[test]
fn profile_completion_merges_layers_and_omits_explicit_selections() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        r#"
[aliases.code]
harness = 'codex'
profiles = ['alpha']
[profiles.alpha]
[profiles.beta]
description = 'Old description'
plugins = ['undefined']
"#,
    );
    write_fixture(
        &home,
        "ayran.toml",
        r#"
[profiles.beta]
description = 'Project tools'
[profiles.gamma]
"#,
    );
    assert_completion(
        home.complete(&["ayran", "--profile", ""]),
        "alpha\nbeta\ngamma\n",
    );
    assert_completion(
        home.complete(&["ayran", "--alias", "code", "--profile", ""]),
        "beta\ngamma\n",
    );
    assert_completion(
        home.complete(&["ayran", "-p", "alpha", "--profile", "beta,g"]),
        "beta,gamma\n",
    );
    assert_completion(
        home.complete(&["ayran", "--codex", "--profile=alpha,b"]),
        "--profile=alpha,beta\n",
    );
    assert_completion(
        home.complete(&["ayran", "--profile", "alpha,beta,gamma", "--profile", ""]),
        "",
    );
    assert_completion(
        home.complete(&["ayran", "--alias", "unknown", "--profile", ""]),
        "",
    );
    assert_completion(
        home.command()
            .args([
                "__complete",
                "--descriptions",
                "--",
                "ayran",
                "--profile",
                "b",
            ])
            .output()
            .unwrap(),
        "beta\tProject tools\n",
    );
}

#[test]
fn no_profile_completion_follows_expansion_and_disable_precedence() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        r#"
[aliases.code]
harness = 'codex'
profiles = ['explicit']
disable = { profiles = ['alias-disabled'] }
[aliases.bare]
harness = 'codex'
profiles = ['explicit']
defaults = false
[profiles.far]
default = true
[profiles.explicit]
profiles = ['child', 'blocked']
[profiles.child]
[profiles.blocked]
profiles = ['hidden']
[profiles.hidden]
[profiles.optional]
"#,
    );
    write_fixture(
        &home,
        "ayran.toml",
        r#"
[disable]
defaults = true
profiles = ['blocked', 'explicit']
[profiles.near]
default = true
profiles = ['child', 'blocked', 'missing']
[profiles.alias-disabled]
default = true
"#,
    );
    assert_completion(
        home.complete(&["ayran", "--no-profile", ""]),
        "alias-disabled\nchild\nnear\n",
    );
    assert_completion(
        home.complete(&["ayran", "--alias", "code", "--no-profile", ""]),
        "child\nexplicit\nnear\n",
    );
    for words in [
        vec!["ayran", "--alias", "bare", "--no-profile", ""],
        vec![
            "ayran",
            "--alias",
            "code",
            "--no-defaults",
            "--no-profile",
            "",
        ],
    ] {
        assert_completion(home.complete(&words), "child\nexplicit\n");
    }
    assert_completion(
        home.complete(&[
            "ayran",
            "--no-defaults",
            "--profile",
            "optional",
            "--no-profile",
            "",
        ]),
        "optional\n",
    );
    assert_completion(
        home.complete(&[
            "ayran",
            "--no-defaults",
            "--profile",
            "explicit,blocked",
            "--no-profile",
            "",
        ]),
        "blocked\nchild\nexplicit\nhidden\n",
    );
    assert_completion(
        home.complete(&[
            "ayran",
            "--alias",
            "bare",
            "--no-profile",
            "explicit",
            "--no-profile",
            "",
        ]),
        "",
    );
    assert_completion(
        home.complete(&["ayran", "--alias", "code", "--no-profile=explicit,n"]),
        "--no-profile=explicit,near\n",
    );
    assert_completion(
        home.complete(&["ayran", "--no-profile", "near,"]),
        "near,alias-disabled\n",
    );
    assert_completion(
        home.complete(&["ayran", "--alias", "code", "--no-profile", "near,"]),
        "near,child\nnear,explicit\n",
    );
}

#[test]
fn plugin_completion_merges_layers_and_filters_for_the_selected_harness() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        r#"
default_harness = 'claude'
[plugins.common]
all = 'common@m'
description = 'Common tools'
[plugins.codex-only]
codex = 'code@m'
[plugins.absent]
all = 'absent@m'
codex = false
[plugins.replaced]
all = 'old@m'
"#,
    );
    write_fixture(
        &home,
        "ayran.toml",
        r#"
default_harness = 'codex'
[plugins.replaced]
claude = 'new@m'
"#,
    );
    assert_completion(
        home.complete(&["ayran", "--plugin", ""]),
        "codex-only\ncommon\n",
    );
    assert_completion(
        home.complete(&["ayran", "--claude", "--plugin", ""]),
        "absent\ncommon\nreplaced\n",
    );
    assert_completion(
        home.command()
            .args([
                "__complete",
                "--descriptions",
                "--",
                "ayran",
                "--plugin",
                "com",
            ])
            .output()
            .unwrap(),
        "common\tCommon tools\n",
    );
    fs::remove_file(home.dir.path().join("ayran.toml")).unwrap();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        "[plugins.unbound]\ndescription = 'No Binding yet'\n",
    );
    assert_completion(home.complete(&["ayran", "--plugin", ""]), "unbound\n");
}

#[test]
fn plugin_completion_omits_alias_and_cli_selections_including_comma_segments() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        r#"
[aliases.code]
harness = 'codex'
plugins = ['alpha']
[plugins.alpha]
all = 'a@m'
[plugins.beta]
all = 'b@m'
[plugins.gamma]
claude = 'g@m'
"#,
    );
    for words in [
        vec!["ayran", "--alias", "code", "--plugin", ""],
        vec!["ayran", "--codex", "--plugin", "alpha", "--plugin", ""],
    ] {
        assert_completion(home.complete(&words), "beta\n");
    }
    assert_completion(
        home.complete(&["ayran", "--codex", "--plugin", "alpha,b"]),
        "alpha,beta\n",
    );
    assert_completion(
        home.complete(&["ayran", "--codex", "--plugin=alpha,b"]),
        "--plugin=alpha,beta\n",
    );
    assert_completion(
        home.complete(&[
            "ayran",
            "--codex",
            "--plugin",
            "alpha,beta,",
            "--plugin",
            "",
        ]),
        "",
    );
    assert_completion(
        home.complete(&["ayran", "--alias", "unknown", "--plugin", ""]),
        "",
    );
}

#[test]
fn no_plugin_completion_offers_surviving_defaults_and_alias_selections() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        r#"
default_harness = 'codex'
[aliases.code]
harness = 'codex'
plugins = ['explicit']
disable = { plugins = ['alias-disabled'] }
[aliases.bare]
harness = 'codex'
plugins = ['explicit']
defaults = false
[plugins.far]
all = 'far@m'
default = true
[plugins.explicit]
all = 'explicit@m'
[plugins.optional]
all = 'optional@m'
"#,
    );
    write_fixture(
        &home,
        "ayran.toml",
        r#"
[disable]
defaults = true
plugins = ['disabled', 'explicit']
[plugins.near]
all = 'near@m'
default = true
description = 'Near Default'
[plugins.disabled]
all = 'disabled@m'
default = true
[plugins.alias-disabled]
all = 'alias@m'
default = true
[plugins.absent]
codex = false
default = true
[plugins.missing]
claude = 'missing@m'
default = true
"#,
    );
    assert_completion(
        home.complete(&["ayran", "--no-plugin", ""]),
        "alias-disabled\nnear\n",
    );
    assert_completion(
        home.complete(&["ayran", "--alias", "code", "--no-plugin", ""]),
        "explicit\nnear\n",
    );
    for words in [
        vec!["ayran", "--alias", "bare", "--no-plugin", ""],
        vec![
            "ayran",
            "--alias",
            "code",
            "--no-defaults",
            "--no-plugin",
            "",
        ],
    ] {
        assert_completion(home.complete(&words), "explicit\n");
    }
    assert_completion(
        home.complete(&[
            "ayran",
            "--alias",
            "code",
            "--no-plugin",
            "explicit",
            "--no-plugin",
            "",
        ]),
        "near\n",
    );
    assert_completion(
        home.complete(&["ayran", "--alias", "code", "--no-plugin=explicit,n"]),
        "--no-plugin=explicit,near\n",
    );
    assert_completion(
        home.complete(&[
            "ayran",
            "--no-defaults",
            "--plugin",
            "optional",
            "--no-plugin",
            "",
        ]),
        "",
    );
}

#[test]
fn codex_completion_omits_path_bindings_direct_and_inherited_from_all() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "ayran.toml",
        r#"
[plugins.direct]
codex = { path = 'direct' }
default = true
[plugins.inherited]
all = { path = 'inherited' }
default = true
[plugins.native]
all = { path = 'fallback' }
codex = 'native@m'
default = true
"#,
    );
    for flag in ["--plugin", "--no-plugin"] {
        assert_completion(home.complete(&["ayran", "--codex", flag, ""]), "native\n");
        assert_completion(
            home.complete(&["ayran", "--claude", flag, ""]),
            "inherited\nnative\n",
        );
    }
}

#[test]
fn skill_completion_merges_layers_and_filters_for_the_selected_harness() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        r#"
default_harness = 'claude'
[skills.common]
all = 'common'
description = 'Common tools'
[skills.codex-only]
codex = 'code'
[skills.absent]
all = 'absent'
codex = false
[skills.replaced]
all = 'old'
"#,
    );
    write_fixture(
        &home,
        "ayran.toml",
        r#"
default_harness = 'codex'
[skills.replaced]
claude = 'new'
"#,
    );
    assert_completion(
        home.complete(&["ayran", "--skill", ""]),
        "codex-only\ncommon\n",
    );
    assert_completion(
        home.complete(&["ayran", "--claude", "--skill", ""]),
        "absent\nayran\ncommon\nreplaced\n",
    );
    assert_completion(
        home.command()
            .args([
                "__complete",
                "--descriptions",
                "--",
                "ayran",
                "--skill",
                "com",
            ])
            .output()
            .unwrap(),
        "common\tCommon tools\n",
    );
    fs::remove_file(home.dir.path().join("ayran.toml")).unwrap();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        "[skills.unbound]\ndescription = 'No Binding yet'\n",
    );
    assert_completion(home.complete(&["ayran", "--skill", ""]), "ayran\nunbound\n");
}

#[test]
fn skill_completion_omits_alias_and_cli_selections_including_comma_segments() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        r#"
[aliases.code]
harness = 'codex'
skills = ['alpha']
[skills.alpha]
all = 'a'
[skills.beta]
all = 'b'
[skills.gamma]
claude = 'g'
"#,
    );
    for words in [
        vec!["ayran", "--alias", "code", "--skill", ""],
        vec!["ayran", "--codex", "--skill", "alpha", "--skill", ""],
    ] {
        assert_completion(home.complete(&words), "beta\n");
    }
    assert_completion(
        home.complete(&["ayran", "--codex", "--skill", "alpha,b"]),
        "alpha,beta\n",
    );
    assert_completion(
        home.complete(&["ayran", "--codex", "--skill=alpha,b"]),
        "--skill=alpha,beta\n",
    );
    assert_completion(
        home.complete(&["ayran", "--codex", "--skill", "alpha,beta,", "--skill", ""]),
        "",
    );
    assert_completion(
        home.complete(&["ayran", "--alias", "unknown", "--skill", ""]),
        "",
    );
}

#[test]
fn no_mcp_completion_follows_defaults_disables_and_recursive_profile_members() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        r#"
default_harness = 'codex'
[aliases.code]
harness = 'codex'
mcp = ['explicit']
profiles = ['team']
disable = { mcp = ['alias-disabled'] }
[aliases.bare]
harness = 'codex'
mcp = ['explicit']
defaults = false
[mcp.far]
all = 'far'
default = true
[mcp.explicit]
all = 'explicit'
[mcp.member]
all = 'member'
description = 'Profile member'
[mcp.cli]
all = 'cli'
[profiles.team]
profiles = ['child']
[profiles.child]
mcp = ['member', 'alias-disabled']
"#,
    );
    write_fixture(
        &home,
        "ayran.toml",
        r#"
[disable]
defaults = true
mcp = ['disabled', 'explicit']
[mcp.near]
all = 'near'
default = true
[mcp.disabled]
all = 'disabled'
default = true
[mcp.alias-disabled]
all = 'alias-disabled'
default = true
[mcp.absent]
codex = false
default = true
[mcp.missing]
claude = 'missing'
default = true
[mcp.path]
all = { command = 'npx' }
default = true
"#,
    );
    for (words, expected) in [
        (
            vec!["ayran", "--no-mcp", ""],
            "alias-disabled\nnear\npath\n",
        ),
        (
            vec!["ayran", "--alias", "code", "--no-mcp", ""],
            "explicit\nmember\nnear\npath\n",
        ),
        (
            vec!["ayran", "--alias", "bare", "--no-mcp", ""],
            "explicit\n",
        ),
        (
            vec!["ayran", "--alias", "code", "--no-defaults", "--no-mcp", ""],
            "explicit\nmember\n",
        ),
        (
            vec!["ayran", "--profile", "team", "--no-mcp", ""],
            "alias-disabled\nmember\nnear\npath\n",
        ),
        (
            vec![
                "ayran",
                "--profile",
                "team",
                "--no-profile",
                "child",
                "--no-mcp",
                "",
            ],
            "alias-disabled\nnear\npath\n",
        ),
        (
            vec![
                "ayran", "--alias", "code", "--no-mcp", "explicit", "--no-mcp", "",
            ],
            "member\nnear\npath\n",
        ),
        (
            vec!["ayran", "--alias", "code", "--no-mcp=explicit,m"],
            "--no-mcp=explicit,member\n",
        ),
        (
            vec!["ayran", "--no-defaults", "--mcp", "cli", "--no-mcp", ""],
            "cli\n",
        ),
    ] {
        assert_completion(home.complete(&words), expected);
    }
}

#[test]
fn mcp_completion_merges_layers_and_filters_for_the_selected_harness() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        r#"
default_harness = 'claude'
[mcp.common]
all = 'common'
description = 'Common tools'
[mcp.codex-only]
codex = 'code'
[mcp.absent]
all = 'absent'
codex = false
[mcp.replaced]
all = 'old'
"#,
    );
    write_fixture(
        &home,
        "ayran.toml",
        r#"
default_harness = 'codex'
[mcp.replaced]
claude = 'new'
"#,
    );
    assert_completion(
        home.complete(&["ayran", "--mcp", ""]),
        "codex-only\ncommon\n",
    );
    assert_completion(
        home.complete(&["ayran", "--claude", "--mcp", ""]),
        "absent\ncommon\nreplaced\n",
    );
    assert_completion(
        home.command()
            .args([
                "__complete",
                "--descriptions",
                "--",
                "ayran",
                "--mcp",
                "com",
            ])
            .output()
            .unwrap(),
        "common\tCommon tools\n",
    );
    fs::remove_file(home.dir.path().join("ayran.toml")).unwrap();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        "[mcp.unbound]\ndescription = 'No Binding yet'\n",
    );
    assert_completion(home.complete(&["ayran", "--mcp", ""]), "unbound\n");
}

#[test]
fn mcp_completion_omits_alias_and_cli_selections_including_comma_segments() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        r#"
[aliases.code]
harness = 'codex'
mcp = ['alpha']
[mcp.alpha]
all = 'a'
[mcp.beta]
all = 'b'
[mcp.gamma]
claude = 'g'
"#,
    );
    for words in [
        vec!["ayran", "--alias", "code", "--mcp", ""],
        vec!["ayran", "--codex", "--mcp", "alpha", "--mcp", ""],
    ] {
        assert_completion(home.complete(&words), "beta\n");
    }
    assert_completion(
        home.complete(&["ayran", "--codex", "--mcp", "alpha,b"]),
        "alpha,beta\n",
    );
    assert_completion(
        home.complete(&["ayran", "--codex", "--mcp=alpha,b"]),
        "--mcp=alpha,beta\n",
    );
    assert_completion(
        home.complete(&["ayran", "--codex", "--mcp", "alpha,beta,", "--mcp", ""]),
        "",
    );
    assert_completion(
        home.complete(&["ayran", "--alias", "unknown", "--mcp", ""]),
        "",
    );
}

#[test]
fn list_kind_completes_aliases() {
    let home = TestHome::new();
    assert_completion(home.complete(&["ayran", "list", "al"]), "aliases\n");
}

#[test]
fn mcp_completion_offers_claude_connector_bindings() {
    let home = TestHome::new();
    write_fixture(
        &home,
        "config/ayran/ayran.toml",
        "[mcp.linear]\nclaude = { connector = 'claude.ai Linear' }\n",
    );
    assert_completion(
        home.complete(&["ayran", "--claude", "--mcp", ""]),
        "linear\n",
    );
    assert_completion(home.complete(&["ayran", "--copilot", "--mcp", ""]), "");
}

#[test]
fn completion_offers_harness_args_suppression_on_launch_and_resume() {
    let home = TestHome::new();
    for words in [
        vec!["ayran", "--no-harness"],
        vec!["ayran", "resume", "--no-harness"],
    ] {
        assert_completion(home.complete(&words), "--no-harness-args\n");
    }
}

#[test]
fn native_marketplace_list_completes_commands_and_flags() {
    let home = TestHome::new();
    let output = home.complete(&["ayran", "native", "market"]);
    assert_completion(output, "marketplace\n");
    assert_completion(
        home.complete(&["ayran", "native", "marketplace", "li"]),
        "list\n",
    );
    let output = home.complete(&["ayran", "native", "marketplace", "list", "--ha"]);
    assert_completion(output, "--harness\n");
}

#[test]
fn native_skill_and_mcp_lists_complete_commands_and_flags() {
    let home = TestHome::new();
    for kind in ["skill", "mcp"] {
        assert_completion(home.complete(&["ayran", "native", kind, "li"]), "list\n");
        let output = home.complete(&["ayran", "native", kind, "list", "--ha"]);
        assert_completion(output, "--harness\n");
        assert_completion(
            home.complete(&["ayran", "native", kind, "list", "--harness", "co"]),
            "codex\ncopilot\n",
        );
    }
}
