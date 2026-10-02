//! Render shell dispatchers for user-level Aliases and completion hooks.

use std::collections::BTreeMap;

use crate::config::Alias;
use crate::diagnostic::Diagnostic;

pub enum Shell {
    Zsh,
    Bash,
    Pwsh,
}

// Union of bash and zsh keywords/builtins plus PowerShell language keywords.
// PowerShell command aliases are ordinary commands and may be shadowed.
const RESERVED: &str = "ayran alias autoload begin bg bind bindkey break builtin bye caller case catch cd chdir class clean clear-host \
command compadd comparguments compcall compctl compdescribe compfiles compgen compgroups \
compopt compquote compset comptags comptry compvalues complete continue coproc declare \
data define disable dirs disown do done dynamicparam echo echotc echoti elif else elseif emulate \
enable end enum esac eval exec exit export false fc fg fi filter finally float for foreach from \
function functions getln getopts hash help hidden history if in inlinescript integer jobs \
kill let limit local log logout mapfile mkdir nocorrect noglob oss parallel param pause popd print printf \
private process prompt pushd pushln pwd r read readarray readonly rehash repeat return sched \
select sequence set setopt shift shopt source static suspend switch test then throw time \
tabexpansion2 times trap true try ttyctl type typeset ulimit umask unalias unfunction unhash unlimit \
unset unsetopt until using var vared wait whence where which while workflow zcompile \
zformat zle zmodload zparseopts zregexparse zstyle";

pub fn render_activation(
    shell: Shell,
    aliases: &BTreeMap<String, Alias>,
) -> Result<String, Diagnostic> {
    if let Some(diagnostic) = aliases.keys().find_map(|name| check_alias_name(name)) {
        return Err(diagnostic);
    }

    let mut output = String::new();
    for name in aliases.keys() {
        match shell {
            Shell::Zsh | Shell::Bash => {
                output.push_str(&format!("{name}() {{ ayran --alias {name} \"$@\"; }}\n"));
            }
            Shell::Pwsh => {
                output.push_str(&format!(
                    "function {name} {{ ayran --alias {name} @args }}\n"
                ));
            }
        }
    }
    if matches!(shell, Shell::Bash) {
        output.push_str(
            r#"_ayran_complete() {
    COMPREPLY=()
    local candidate
    local -a words=("${COMP_WORDS[@]:0:COMP_CWORD+1}")
    if [[ ${words[0]} != ayran ]]; then
        words=(ayran --alias "${words[0]}" "${words[@]:1}")
    fi
    while IFS= read -r candidate; do
        COMPREPLY+=("$candidate")
    done < <(ayran __complete -- "${words[@]}" 2>/dev/null)
}
complete -F _ayran_complete ayran
"#,
        );
        for name in aliases.keys() {
            output.push_str(&format!("complete -F _ayran_complete {name}\n"));
        }
    }
    if matches!(shell, Shell::Zsh) {
        output.push_str(
            r#"if (( ! $+functions[compdef] )); then
    autoload -Uz compinit
    compinit -i -D
fi
_ayran_complete() {
    local candidate description
    local -a request_words=("${words[@]:0:$CURRENT}") candidates=()
    if [[ ${request_words[1]} != ayran ]]; then
        request_words=(ayran --alias "${request_words[1]}" "${request_words[@]:1}")
    fi
    while IFS=$'\t' read -r candidate description; do
        candidate=$(printf '%b' "$candidate")
        candidate=${candidate//\\/\\\\}
        candidate=${candidate//:/\\:}
        candidates+=("${candidate}:${description}")
    done < <(ayran __complete --descriptions -- "${request_words[@]}" 2>/dev/null)
    (( ${#candidates} )) || return 0
    _describe 'ayran' candidates
}
compdef _ayran_complete ayran
"#,
        );
        for name in aliases.keys() {
            output.push_str(&format!("compdef _ayran_complete {name}\n"));
        }
    }
    if matches!(shell, Shell::Pwsh) {
        output.push_str(
            r#"if (-not (Test-Path variable:global:__ayranCompletion)) {
    $global:__ayranCompletion = @{
        Original = (Get-Item function:TabExpansion2).ScriptBlock
        Handled = $false
        HasCandidates = $false
    }
}
$ayranCompleter = {
    param($wordToComplete, $commandAst, $cursorPosition)
    $global:__ayranCompletion.Handled = $true
    $global:__ayranCompletion.HasCandidates = $false
    try {
        $ErrorActionPreference = 'Stop'
        $PSNativeCommandUseErrorActionPreference = $false
        # Preserve an empty cursor word even if the user selects Legacy mode.
        $PSNativeCommandArgumentPassing = 'Standard'
        # Parse only the command text before the cursor; never evaluate it.
        $length = [Math]::Min($commandAst.Extent.Text.Length, $cursorPosition - $commandAst.Extent.StartOffset)
        $text = $commandAst.Extent.Text.Substring(0, $length)
        $tokens = $null
        $parseErrors = $null
        $ast = [System.Management.Automation.Language.Parser]::ParseInput($text, [ref]$tokens, [ref]$parseErrors)
        $command = $ast.Find({ param($node) $node -is [System.Management.Automation.Language.CommandAst] }, $false)
        $requestWords = @($command.CommandElements | ForEach-Object {
            if ($_ -is [System.Management.Automation.Language.StringConstantExpressionAst]) {
                $_.Value
            } else {
                $_.Extent.Text
            }
        })
        if ($cursorPosition -gt $commandAst.Extent.EndOffset -or $command.Extent.EndOffset -lt $text.Length) {
            $requestWords += ''
        }
        if ($requestWords[0] -ne 'ayran') {
            $requestWords = @('ayran', '--alias', $requestWords[0]) + @($requestWords | Select-Object -Skip 1)
        }
        ayran __complete --descriptions -- @requestWords 2>$null | ForEach-Object {
            $parts = $_ -split "`t", 2
            if ($parts.Count -eq 2) {
                $candidate = [regex]::Replace($parts[0], '\\(\\|t)', {
                    param($match)
                    if ($match.Groups[1].Value -eq 't') { "`t" } else { '\' }
                })
                $completionText = $candidate
                if ($candidate -match '[\s''"`$;|&(){}\[\]<>@#]') {
                    $completionText = "'" + $candidate.Replace("'", "''") + "'"
                }
                [System.Management.Automation.CompletionResult]::new($completionText, $candidate, 'ParameterValue', $parts[1])
                $global:__ayranCompletion.HasCandidates = $true
            }
        }
    } catch {
        # Completion failures must not interrupt the prompt.
    }
}
Register-ArgumentCompleter -Native -CommandName 'ayran' -ScriptBlock $ayranCompleter
function global:TabExpansion2 {
    [CmdletBinding(DefaultParameterSetName = 'ScriptInputSet')]
    param(
        [Parameter(ParameterSetName = 'ScriptInputSet', Mandatory = $true, Position = 0)]
        [AllowEmptyString()][string]$inputScript,
        [Parameter(ParameterSetName = 'ScriptInputSet', Position = 1)]
        [int]$cursorColumn = $inputScript.Length,
        [Parameter(ParameterSetName = 'AstInputSet', Mandatory = $true, Position = 0)]
        [System.Management.Automation.Language.Ast]$ast,
        [Parameter(ParameterSetName = 'AstInputSet', Mandatory = $true, Position = 1)]
        [System.Management.Automation.Language.Token[]]$tokens,
        [Parameter(ParameterSetName = 'AstInputSet', Mandatory = $true, Position = 2)]
        [System.Management.Automation.Language.IScriptPosition]$positionOfCursor,
        [Parameter(ParameterSetName = 'ScriptInputSet', Position = 2)]
        [Parameter(ParameterSetName = 'AstInputSet', Position = 3)]
        [hashtable]$options = $null
    )
    $global:__ayranCompletion.Handled = $false
    $result = & $global:__ayranCompletion.Original @PSBoundParameters
    # An empty registered completer otherwise falls back to filenames.
    # Preserve the original completion function for every other command.
    if ($global:__ayranCompletion.Handled -and -not $global:__ayranCompletion.HasCandidates) {
        $result = [System.Management.Automation.CommandCompletion]::new(
            [System.Collections.ObjectModel.Collection[System.Management.Automation.CompletionResult]]::new(),
            -1, $result.ReplacementIndex, $result.ReplacementLength)
    }
    $result
}
"#,
        );
        for name in aliases.keys() {
            output.push_str(&format!(
                "Register-ArgumentCompleter -CommandName '{name}' -ScriptBlock $ayranCompleter\n"
            ));
        }
    }
    Ok(output)
}

/// The reserved-name rule shared by activation and the config audit.
pub fn check_alias_name(name: &str) -> Option<Diagnostic> {
    RESERVED
        .split_whitespace()
        .any(|word| word.eq_ignore_ascii_case(name))
        .then(|| {
            Diagnostic::error(
                "alias-reserved-name",
                format!("Alias {name} is reserved by a supported shell"),
                Some("rename the Alias before activating it"),
            )
        })
}
