# Configure and run Ayran

This example uses the user configuration in `~/.config/ayran/ayran.toml`.
Ayran launches Codex, Claude Code, or GitHub Copilot CLI with selected plugins,
skills, and profiles. Install and sign in to each harness you want to use first;
Ayran does not replace its CLI or account setup.

## 1. Install Ayran

Follow the [installation guide](installation.md) to install Ayran using a
release installer, mise, or a source build. Then verify:

```sh
ayran --version
```

During development, you can use `cargo run -p ayran-cli -- --help` from this repository
instead of an installed binary.

## 2. Set up the user configuration

Find or edit the user layer with:

```sh
ayran config path --user
ayran config edit --user
```

On Linux and macOS, the default location is `~/.config/ayran/ayran.toml`;
`XDG_CONFIG_HOME` changes its base directory. On Windows, it is
`%APPDATA%\ayran\ayran.toml`. Set `AYRAN_CONFIG` to use a different file.
Paste the following TOML into that user file, adjusting models and capabilities
to your own accounts and installed harnesses. The model identifiers below are
copied from the existing user configuration.

```toml
default_harness = "codex"

[harnesses.codex]
model = "gpt-6.1-sol"
effort = "medium"

[harnesses.claude]
model = "claude-opus-5-5"
effort = "medium"

[harnesses.copilot]
args = ["--no-experimental", "--allow-all"]

[marketplaces.claude-plugins-official]
claude = { source = "github:anthropics/claude-plugins-official" }
codex = { source = "github:anthropics/claude-plugins-official" }

[marketplaces.mattpocock]
copilot = { source = "github:mattpocock/skills", ref = "v1.2.3" }

[marketplaces.pstack-claude]
all = { source = "github:michael-denyer/pstack-claude" }

[marketplaces.dotnet-agent-skills]
all = { source = "github:dotnet/skills" }

[plugins.matt]
all = "mattpocock-skills@claude-plugins-official"
copilot = "mattpocock-skills@mattpocock"

[plugins.pstack]
all = "pstack@pstack-claude"

[plugins.dotnet]
all = "dotnet@dotnet-agent-skills"

[skills.suggest-commit-message]
all = { source = "github:mlgentdev/skills", subdir = "skills/suggest-commit-message" }

[profiles.matt]
plugins = ["matt"]
skills = ["suggest-commit-message"]
description = "Matt Pocock skills"

[profiles.pstack]
plugins = ["pstack"]
skills = ["suggest-commit-message"]

[profiles.dotnet]
plugins = ["dotnet"]

[aliases.sol]
harness = "codex"
model = "gpt-6.1-sol"
effort = "medium"

[aliases.solh]
harness = "codex"
model = "gpt-6.1-sol"
effort = "high"

[aliases.soll]
harness = "codex"
model = "gpt-6.1-sol"
effort = "low"

[aliases.luna]
harness = "codex"
model = "gpt-6-luna"
effort = "high"

[aliases.lunax]
harness = "codex"
model = "gpt-6-luna"
effort = "xhigh"

[aliases.opus]
harness = "claude"
model = "claude-opus-5-5"
effort = "medium"

[aliases.matt]
harness = "codex"
model = "gpt-6.1-sol"
effort = "medium"
profiles = ["matt"]

[aliases.matt-opus]
harness = "claude"
model = "claude-opus-5-5"
effort = "medium"
profiles = ["matt"]

[aliases.pstack]
harness = "codex"
model = "gpt-6.1-sol"
effort = "medium"
profiles = ["pstack"]

[aliases.pstack-opus]
harness = "claude"
model = "claude-opus-5-5"
effort = "medium"
profiles = ["pstack"]
```

### What the configuration means

- `default_harness = "codex"` makes `ayran` launch Codex unless you choose another harness.
- `[harnesses.codex]` and `[harnesses.claude]` set the default model and effort for each harness. Command-line `--model` and `--effort` override these settings.
- `[harnesses.copilot].args` passes `--no-experimental` and `--allow-all` to every Copilot session. `--allow-all` grants Copilot broad permissions; remove it if you prefer permission prompts, or use `--no-harness-args` for a session.
- `[marketplaces.*]` declares plugin sources. The Copilot `mattpocock` source is pinned to `v1.2.3`; the other sources have no explicit ref.
- `[plugins.*]` binds a logical name to a native plugin. `all` supplies the binding for every harness; a harness-specific entry overrides it. Here, `matt` uses `claude-plugins-official` for Codex and Claude, and `mattpocock` for Copilot.
- `[skills.suggest-commit-message]` declares a skill from the `skills/suggest-commit-message` folder of a Git repository. Install it for each harness before selecting it.
- `[profiles.*]` bundles capabilities. `matt` and `pstack` each include their plugin and the commit-message skill; `dotnet` includes only its plugin. Profiles carry no model or effort.
- `[aliases.*]` defines shell commands with a fixed harness, model, effort, and optional profile. Shell activation makes these commands available.

This file declares capabilities without making any of them defaults. Select
profiles, plugins, or skills explicitly, or use an alias that selects a profile.

## 3. Install the declared capabilities

Declaring a source does not install its plugin or skill. Preview installation
for the harness you want to use:

```sh
ayran install matt pstack dotnet --codex --dry-run
ayran install --skill suggest-commit-message --codex --dry-run
```

Then install:

```sh
ayran install matt pstack dotnet --codex
ayran install --skill suggest-commit-message --codex
```

Install only the plugins you need by removing names from the first command.
Repeat with `--claude` or `--copilot` for another harness. Installation changes
the selected harness's user-level installation/configuration; Ayran then selects
capabilities per session. Diagnostics explain unsupported bindings or missing
prerequisites.

Inspect the resulting configuration and harness state:

```sh
ayran config list
ayran list plugins
ayran list skills
ayran list profiles
ayran doctor --codex
```

## 4. Activate shell aliases

For Bash, add this after the PATH setting in `~/.bashrc` and evaluate it in
your current shell:

```sh
eval "$(ayran activate bash)"
```

For Zsh, use this in `~/.zshrc`:

```sh
eval "$(ayran activate zsh)"
```

For PowerShell, add this to your PowerShell profile:

```powershell
ayran activate pwsh | Out-String | Invoke-Expression
```

Activation installs shell dispatchers and completion hooks. Re-evaluate it or
open a new shell after adding or removing aliases.

| Command | Harness | Model | Effort | Profile |
| --- | --- | --- | --- | --- |
| `sol` | Codex | `gpt-6.1-sol` | medium | — |
| `solh` | Codex | `gpt-6.1-sol` | high | — |
| `soll` | Codex | `gpt-6.1-sol` | low | — |
| `luna` | Codex | `gpt-6-luna` | high | — |
| `lunax` | Codex | `gpt-6-luna` | xhigh | — |
| `opus` | Claude | `claude-opus-5-5` | medium | — |
| `matt` | Codex | `gpt-6.1-sol` | medium | matt |
| `matt-opus` | Claude | `claude-opus-5-5` | medium | matt |
| `pstack` | Codex | `gpt-6.1-sol` | medium | pstack |
| `pstack-opus` | Claude | `claude-opus-5-5` | medium | pstack |

## 5. Launch a session

Change to the project directory first. Ayran resolves configuration from that
working directory.

```sh
ayran                              # Default Codex session
ayran --claude                     # Claude with its configured model and effort
ayran --copilot --no-harness-args   # Copilot without configured arguments
ayran -p matt                      # Codex with the matt profile
ayran --claude -p pstack            # Claude with the pstack profile
ayran -p dotnet                    # Codex with the dotnet profile
ayran --plugin matt                # Only the matt plugin, without its profile's skill
ayran --skill suggest-commit-message
ayran --model gpt-6.1-sol --effort high
```

With shell activation, use the configured shortcuts:

```sh
solh
matt
matt-opus
pstack-opus
matt --effort high
```

Preview a launch command without starting the harness:

```sh
ayran -p matt --dry-run
matt --dry-run
```

Pass a prompt or native harness arguments after `--`:

```sh
ayran -p matt -- "Explain this repository"
solh -- "Review the current changes"
```

Configured harness arguments precede arguments passed after `--`. Passthrough
arguments are not remembered for resumed sessions.

List and resume sessions:

```sh
ayran list sessions
ayran resume --last
ayran resume SESSION_ID
ayran resume SESSION_ID --fork
```

Replace `SESSION_ID` with an Ayran session ID from the list. `--last` uses the
current directory; `--last --all` searches across directories. Resume resolves
the saved request again using current configuration; `--fork` continues the
conversation in a new session.

## 6. Add project and personal overrides

The user layer applies everywhere. Ayran also reads `ayran.toml` and
`ayran.local.toml` in directories from the filesystem root to your working
directory. Nearer layers win; a local layer wins over a project layer in the
same directory.

For a shared project preference, put this in the project's `ayran.toml`:

```toml
[harnesses.codex]
effort = "high"
```

For a personal project preference, put this in `ayran.local.toml` and add that
filename to `.gitignore`:

```toml
[harnesses.codex]
effort = "low"
```

Use `ayran config edit --project` or `ayran config edit --local` to edit those
layers. Harness settings merge key by key, so these examples retain the user
layer's model. Same-named capability definitions and profiles replace the
entire earlier table. Aliases and isolated-home settings belong only in the
user layer; harness `args` may appear in user or local layers, never shared
project layers.

When a project layer declares install sources, review those sources and use
`ayran trust` before installing from them. The user-layer sources in the main
example do not require project trust.

If a launch fails, run `ayran config list` to see the loaded files and
`ayran doctor --codex` (or `--claude` / `--copilot`) to inspect diagnostics.
