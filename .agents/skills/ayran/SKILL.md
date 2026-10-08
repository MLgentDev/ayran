---
name: ayran
description: Explain, configure and troubleshoot ayran, the launcher for coding-agent CLIs.
disable-model-invocation: true
---

# ayran

ayran launches a coding-agent CLI (a **Harness**: `claude`, `codex` or `copilot`) with only the capabilities one **Session** needs. It is mise for coding agents: layered `ayran.toml` files declare what exists; each launch selects what loads.

You are probably running inside an ayran Session right now. Config changes apply to the next launch, never to this one.

## Turn this Skill on

- Claude Code: launch `ayran --claude --skill ayran`, then type `/ayran`.
- Copilot: launch `ayran --copilot --skill ayran`, then type `/ayran`.
- Codex: run `ayran install --skill ayran --codex`, then launch `ayran --codex --skill ayran` and type `$ayran`. Launch never installs; rerun install after a `skill-outdated` warning to refresh the manual.

The binary embeds this manual. No config is needed on Claude or Copilot, and the Skill is not a Default. To make it a Default, redefine `[skills.ayran]` with `all = { builtin = "ayran" }`, `default = true`, and `codex = false` until installed.

## Model

- **Capability**: something a Session can switch on. Four kinds:
  - **Plugin** (`[plugins.x]`), switched on as a whole.
  - **Skill** (`[skills.x]`), a single skill.
  - **MCP server** (`[mcp.x]`).
  - **Profile** (`[profiles.x]`), a named bundle of the other three and of nested Profiles.
- **Logical name**: config names a Capability once; a **Binding** per Harness says what it is natively (`all = …` for every Harness, `claude = …` to override, `codex = false` for deliberately absent). Inside the Session a skill keeps its native name.
- **Default**: `default = true` loads it in every Session. **Disable**: `[disable]` in config or `--no-skill x` etc. on the command line keeps it out.
- **Hiding**: user-level items (personal skills, installed plugins, user MCP servers) stay hidden unless selected. Project-level items in the repo are left alone. An item ayran cannot hide is a **leak**, reported as a warning.
- **Preset** (user layer only): how a Session runs, meaning Harness, model, Effort and Harness args. It carries no Capabilities.
- **Alias** (user layer only): a shell command (`work`) that fixes a Preset and/or Capabilities. It becomes a command after shell activation: `eval "$(ayran activate bash)"` (or `zsh`), or in PowerShell `ayran activate pwsh | Out-String | Invoke-Expression`.
- **Effort**: `low | medium | high | xhigh | max`, mapped to each Harness's native setting.

## Config layers

Nearer wins, from far to near:

1. Built-in layer: no file path; declares the `ayran` Skill with `all = { builtin = "ayran" }`, without `default`.
2. User layer: `$AYRAN_CONFIG`, otherwise the default path in [Paths](#paths).
3. Every directory from `/` down to the working directory: its `ayran.toml` (**project**, committed), then its `ayran.local.toml` (**local**, gitignored).

A same-named definition replaces the whole table. `[harnesses.<h>]` merges key by key, and `[disable]` lists add up across layers.

Placement rules:

- Aliases, Presets and `home` belong only in the user layer.
- Harness `args` belong only in the user or local layer, never in the project layer.

## Paths

| What | Linux / macOS | Windows |
|---|---|---|
| User layer | `$XDG_CONFIG_HOME/ayran/ayran.toml` (default `~/.config/ayran/ayran.toml`) | `%APPDATA%\ayran\ayran.toml` |
| Session records | `$XDG_STATE_HOME/ayran/sessions/` (default `~/.local/state/ayran/sessions/`) | `%LOCALAPPDATA%\ayran\sessions\` |
| Isolated homes | `$XDG_STATE_HOME/ayran/homes/<harness>/` | `%LOCALAPPDATA%\ayran\homes\<harness>\` |
| Generated per-Session files | `$XDG_CACHE_HOME/ayran/` (default `~/.cache/ayran/`) | `%LOCALAPPDATA%\ayran\` |

`$AYRAN_CONFIG` overrides the user layer on every OS, and `ayran config path -u` prints the one in effect. Project and local layers are the same file names on every OS. On Windows, Harness homes are under `%USERPROFILE%` (`.claude`, `.codex`, `.copilot`) unless `CLAUDE_CONFIG_DIR`, `CODEX_HOME` or `COPILOT_HOME` is set.

## Commands

`ayran --help` and `ayran <command> --help` are authoritative for flags. These are the entry points:

| Goal | Command |
|---|---|
| Launch | `ayran [--claude\|--codex\|--copilot] [-p profile] [--skill x,y] [--plugin x] [--mcp x] [-m model] [-e effort] [--preset p] [-- harness args or prompt]` |
| Preview a launch | the same command plus `--dry-run` (add `--json` for machine output); prints the command and a trace of why each item is in or out |
| See what is declared | `ayran list [plugins\|skills\|mcp\|profiles\|aliases\|presets\|marketplaces]` |
| See which files load | `ayran config list`; `ayran config path -u\|-p\|-l` |
| Find problems | `ayran doctor [--claude\|--codex\|--copilot]` |
| Resume | `ayran list sessions`, `ayran resume --last`, `ayran resume <id> [--fork]` |
| Install declared sources | `ayran install …`, `ayran trust` |
| Change persistent Harness state | `ayran native plugin\|skill\|mcp\|marketplace …` |

Flags go before `--`. Everything after `--` passes to the Harness unchanged and is never remembered.

## Working on a user's request

1. **Look before answering.** Run `ayran --version`, `ayran config list`, `ayran list` and, for a problem, `ayran doctor`. Answer from the user's real layers, not from the examples here.
2. **Preview with `--dry-run`.** A bare launch command starts an interactive Harness, so run launches only as `ayran … --dry-run`.
3. **Edit layer files directly.** Find the file with `ayran config path -u|-p|-l` and edit it with your file tools. `ayran config edit` opens an interactive editor and is meant for the human. Put each key in the layer its rules allow: personal choices in user or local, shared ones in project.
4. **Verify.** Done means `ayran doctor` shows no new errors and `ayran … --dry-run` shows the intended Harness, model and Capabilities.
5. **Ask before writing Harness state.** `install`, `trust` and `native … enable|disable|update` change the Harness's own user-level state. Show the `--dry-run` first and get approval.

## References

Read one when the request reaches its area:

- [config.md](references/config.md): full `ayran.toml` schema, Binding forms, resolution order, worked example.
- [install.md](references/install.md): Marketplaces, `install`, `trust`, `native` commands, Isolated homes.
- [sessions.md](references/sessions.md): Session records, `resume`, forks, Codex linking.
- [troubleshooting.md](references/troubleshooting.md): exit codes, common Diagnostics, leaks, Harness quirks.
