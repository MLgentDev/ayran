# Troubleshooting

Start with `ayran doctor` (all Harnesses) or `ayran doctor --claude`. For one specific launch, use `ayran … --dry-run`. The trace on stderr explains why each item is in or out and where each arg came from.

Diagnostics print as `ayran: <severity>[<code>]: <message>` with an optional `hint:` line. The hint is usually the fix.

- **Severity:** an **error** blocks the launch. A **warning** means the launch works but isn't exactly what was asked. A **note** is informational.
- **Reachable or latent:** a config gap is an error when some Session would load it without being asked: a Default, a Default Profile member, or an Alias selection. Otherwise it is a warning. To fix a gap on purpose, write `<harness> = false`.

## Exit codes

| Code | Meaning |
|---|---|
| the Harness's own | the launch reached the Harness |
| 2 | usage error. Common cause: a stray positional word; Harness args go after `--` |
| 3 | config or resolution error |
| 1 | `doctor` found errors, or the `config edit` editor failed |

## Common codes

| Code | Usual fix |
|---|---|
| `no-harness` | set `default_harness` or pass `--claude`, `--codex` or `--copilot` |
| `config-invalid` | fix the key, table or type the message names in that layer file |
| `user-level-only` / `private-layer-only` | move Aliases, Presets or `home` to the user layer; move `args` to the user or local layer |
| `missing-binding` | add `all = …` or `<harness> = …`, or `<harness> = false` |
| `unsupported-binding` | path Skill on Codex: `ayran install --skill x --codex`. Path Plugin on Codex, or a connector on Copilot: bind it differently |
| `native-not-found` | the native id isn't installed or discoverable. Check `ayran native plugin\|skill\|mcp list` |
| `skill-not-installed` / `skill-outdated` | `ayran install --skill x [--harness]` |
| `path-not-found` | a relative path resolves from the declaring file's directory |
| `skill-name-clash` | two selected Skills share a native name; select one |
| `untrusted-layer` | review the project layer's sources, then run `ayran trust <path>` |
| `harness-too-old` / `harness-not-found` | update or install the Harness CLI |
| `enumeration-failed` | ayran cannot read Harness state, so it refuses to launch; fix the file named in the message |
| `leak` | an item the Harness cannot hide (see below) |

## Leaks

A leak is a user-level item that stays visible because the Harness offers no way to hide it:

- **Copilot:** personal skills in `~/.copilot/skills` and `~/.agents/skills`, and the `skillDirectories` folders.
- **Claude:** account-synced plugins.
- **Codex:** remote plugins.
- **Claude, name clashes:** a personal skill sharing a name with a project, bundled or enterprise skill.

There are two remedies. Move the item out of the personal location and declare it as a path Skill, or use `home = "isolated"` for that Harness.

## Harness quirks

- Claude Code drops an Effort its model can't take to the highest supported level. Copilot reports it and doesn't apply it. Codex doesn't check.
- On Codex resume, model or Effort overrides may not apply (a known Codex issue). ayran prints `codex-resume-override`.
- On Claude, an MCP server switched off with `/mcp` stays off even when selected (`mcp-disabled`).
- On Copilot, a skill switched off with `copilot skill disable` stays off (`skill-disabled`). The hint gives the command that switches it back on.
- Generated per-Session files can always be deleted safely; their location is in the skill's Paths table.
- On Windows, ayran runs the Harness as a child process and passes its exit code through. On Unix it replaces itself with the Harness.
