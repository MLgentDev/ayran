# ayran.toml reference

Unknown tables, unknown keys and wrong types are errors (`config-invalid`). Relative paths resolve against the directory of the file that declares them; `~` expands, `$VARS` and `%VARS%` do not.

## Top level and Harness settings

```toml
default_harness = "claude"        # claude | codex | copilot

[harnesses.claude]
model = "sonnet"                  # passed as-is, never checked
effort = "medium"                 # low | medium | high | xhigh | max
home = "shared"                   # shared | isolated; user layer only
args = ["--some-flag"]            # Harness args; user or local layer only
```

`model` and `effort` exist only per Harness. There is no global model.

## Plugins and Skills

Keys: `all`, `claude`, `codex`, `copilot`, `default`, `description`. A per-Harness key overrides `all`.

```toml
[skills.tdd]
all = "tdd"                                   # native: a skill the Harness already discovers
default = true

[skills.lint]
all = { path = "tools/lint-skill" }           # path: directory containing SKILL.md
codex = false                                 # deliberately absent on Codex

[skills.commit]
all = { source = "github:owner/repo", ref = "v1.0", subdir = "skills/commit" }   # git; needs `ayran install --skill commit`

[plugins.review]
all = "review@acme"                           # native: name@marketplace
copilot = { path = "~/dev/review-plugin" }    # path: plugin root
```

Binding values:

| Value | Meaning |
|---|---|
| `"native-id"` | something the Harness has installed or discovers. A Skill name cannot contain `:` (no `plugin:skill`) |
| `{ path = "…" }` | a local directory. Codex cannot load a path Skill until it is installed with `ayran install --skill x --codex`, and it never loads a path Plugin |
| `{ builtin = "ayran" }` | the Skill embedded in the binary, valid only for Skills. No other keys; any logical name. Claude and Copilot load it per Session; Codex requires an installed snapshot |
| `{ source, ref?, subdir? }` | Skill from git: `github:o/r`, `https://`, `ssh://`, `git@`. Installed explicitly |
| `false` | deliberately absent. Explicit selection gives an error; via a Default or Profile it is skipped with a note |
| no key and no `all` | missing. Selecting it is always an error (`missing-binding`) |

## MCP servers

```toml
[mcp.linear]
all = { url = "https://mcp.linear.app/mcp", bearer_token_env = "LINEAR_TOKEN" }

[mcp.files]
all = { command = "./bin/files-mcp", args = ["--root", "."], env_vars = ["FILES_TOKEN"] }

[mcp.pg]
claude = "postgres"                           # native: already configured in the Harness
all = false

[mcp.notion]
claude = { connector = "claude.ai Notion" }   # account connector, exact Harness id
codex = { connector = "connector_…" }
```

- A **definition** is added for one Session under the logical name.
  - stdio uses `command`, `args`, `env` (literal values) and `env_vars` (names to pass through).
  - HTTP uses `url` (streamable HTTP only), `headers`, `env_headers = { "Header" = "VAR" }` and `bearer_token_env`.
- Secrets are referenced only by environment variable name. `${` in any literal is an error.
- A connector Binding works on Claude and Codex only, and never through `all`.
- MCP names must match `[A-Za-z0-9_-]+`.

## Profiles

```toml
[profiles.review]
plugins = ["review"]
skills = ["tdd"]
mcp = ["linear"]
profiles = ["base"]        # nesting; cycles are errors
default = true
description = "Code review kit"
```

## Presets and Aliases (user layer only)

```toml
[presets.opus]
harness = "claude"         # required
model = "opus"
effort = "high"
args = ["--x"]
harness_args = true        # false drops [harnesses.claude].args

[aliases.cr]
preset = "opus"            # or harness/model/effort inline; inline fields override the Preset
profiles = ["team"]
skills = ["tdd"]
defaults = false           # like --no-defaults
disable = { skills = ["x"] }
description = "Claude review"
```

- `--preset opus` and the shorthand `--opus` replace the Alias's whole run block (Harness, model, Effort, args). The Alias's Capabilities still apply.
- Alias names match `[a-zA-Z_][a-zA-Z0-9_-]*` and cannot be a shell builtin, a shell keyword or `ayran`.
- After adding or removing an Alias, re-run `eval "$(ayran activate <shell>)"`.

## Disables

```toml
[disable]
skills = ["tdd"]
plugins = []
mcp = []
profiles = []              # never expand these
defaults = true            # drop Defaults declared in farther layers
```

## Resolution order

- **Harness:** Harness flag, then command-line Preset, then Alias `harness`, then Alias Preset, then nearest `default_harness`. If none is set, that is an error.
- **Model and Effort:** flag, then command-line Preset, then Alias inline field, then Alias Preset, then nearest `[harnesses.<h>]`. If none is set, ayran passes nothing and the Harness default applies.
- **Capabilities:**
  1. Start with explicit selections (flags and the Alias).
  2. Add Defaults.
  3. Expand Profiles.
  4. Apply Disables. A CLI Disable beats everything. An explicit selection beats a config Disable. A config Disable removes items that came via Defaults or Profiles.
  5. Members of an explicitly selected Profile count as "via Profile", not explicit.
- **argv:** ayran's generated flags, then `[harnesses.<h>].args`, then the run block's `args`, then `--` passthrough. Later arguments win where the Harness allows it. `--no-harness-args` drops both kinds of configured args.

## Worked example

User layer: `default_harness = "claude"`, `[harnesses.claude] model = "sonnet"`, `effort = "medium"`. Default Skill `tdd`. Alias `cr` with `preset = "opus"` and `profiles = ["team"]`.

Repo `ayran.toml`: Default Profile `team` = `review` and `lint`, plus Profile `base`. Profile `base` contains `tdd`. `[disable] skills = ["tdd"]`.

Repo `ayran.local.toml`: `[harnesses.claude] effort = "low"`.

| Command | Result |
|---|---|
| `ayran` | claude / sonnet / low; review, lint. `tdd` came via a Default, so the config Disable removes it |
| `cr --skill tdd` | claude / opus / high; review, lint, tdd. The explicit selection beats the config Disable |
