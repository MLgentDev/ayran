# Installing and native state

Launch, resume, `list` and `doctor` never write to a Harness home. Only these commands change persistent Harness state: `install`, `native … enable|disable|update` and `trust`. Each takes `--dry-run`, except `trust`. Show the dry run to the user before running for real.

## Marketplaces and install

```toml
[marketplaces.acme]
all = { source = "github:acme/agent-plugins", ref = "v1.2" }
codex = { source = "github:acme/agent-plugins", name = "acme-agents" }   # per-Harness native name
claude = false

[plugins.dotnet]
all = "dotnet@acme"
```

- A `source` is `github:owner/repo`, a git URL, or `{ path = "…" }`. `ref` is a branch or tag; only Codex also accepts a commit SHA.
- `ayran install` with no names covers every declared Marketplace and every Plugin with a native Binding, on every installed Harness. Narrow it as needed:
  - `ayran install dotnet --codex` installs one Plugin on one Harness.
  - `ayran install --skill commit --claude` installs a git or path Skill snapshot. `ayran install --skill ayran --codex` installs the embedded manual as a built-in snapshot.
  - `ayran install --mcp files` installs an MCP definition.
- Explicit Skill installation replaces an unmodified owned snapshot when its source changes; modified or unowned copies conflict. Built-in snapshots use stable `builtin:<name>` sources, need no Trust, and become outdated only when embedded content changes. Claude and Codex Skill copies end Natively off; Copilot copies remain on with a leak note. Install never removes or replaces Plugins. An installed Plugin is left natively **off**, and ayran switches it on per Session when selected.
- Installation targets the home that launches use, including an Isolated home.

## Trust

A project layer (a shared `ayran.toml`) cannot install sources until the user trusts it. This applies to Marketplaces, MCP definitions, and local or git Skills.

- `ayran trust [path]` records approval of the nearest project layer's current source declarations.
- Changing a source, ref or definition invalidates the approval. The next install then reports `untrusted-layer`.
- `ayran trust --list` shows recorded approvals; `ayran trust --revoke [path]` removes one.
- User and local layers and built-in Skill sources need no Trust.

## Native commands

These change the Harness's own user-level config, outside any one Session:

```
ayran native plugin list|enable|disable|update <name>… [--claude|--codex|--copilot] [--id]
ayran native skill  list|enable|disable <name>…
ayran native mcp    list|enable|disable <name>…
ayran native marketplace list|update <name>…
```

- Names are logical names from config. Add `--id` together with a Harness flag to use native ids.
- **Natively off** means the Harness keeps the item out of every Session by itself. ayran can still switch a natively off Plugin on per Session.
- On Codex, a plugin update installs from the Marketplace snapshot on disk, so run `native marketplace update` first.

## Isolated homes

`[harnesses.<h>] home = "isolated"` in the user layer runs that Harness against an ayran-owned home (see the skill's Paths table). This removes leaks, but login, trust and history are separate from the normal home. The home starts empty: log in once inside it, then install into it with `ayran install`.
