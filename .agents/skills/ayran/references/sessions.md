# Sessions and resume

Every launch writes a Session record, `<id>.json`, to the Session records directory listed in the skill's Paths table.

- **What it stores:** the request **as typed**, meaning Capability flags, Disables, `-m`, `-e`, the Alias and the Preset. It also stores the Harness, home, working directory and times.
- **What it leaves out:** the resolved result and anything after `--`.
- **Pruning:** records are pruned after 30 days unused.

## Finding and resuming

```
ayran list sessions [--all] [--harness h] [--json]
ayran resume --last [--all] [--codex]     # most recent in this directory (--all: anywhere; a Harness flag filters)
ayran resume <id-or-prefix> [flags] [-- passthrough]
ayran resume <id> --fork                  # new Session continuing the conversation; original untouched
```

What resume does:

- It changes to the recorded directory and resolves the request again against **current** config. Config edits since the launch therefore apply.
- New flags merge into the record and persist.
  - `--skill x` adds a selection, and `--no-skill x` removes or disables it. The same applies to `--plugin`, `--mcp` and `-p`.
  - `-m` and `-e` replace the recorded values.
  - `--preset` replaces the recorded Preset, but must keep the same Harness.
- `--no-defaults` and `--no-harness-args` cannot be undone on resume. Start a new Session instead.
- If a Capability selected in the request has since vanished from config, resume fails. Drop it with `ayran resume <id> --no-skill x`.
- Copilot cannot fork at launch; use `/fork` inside the session.

## Codex linking

Codex cannot take a chosen session id, so ayran links its rollout later, on the first `resume` or `list sessions`.

- If several rollouts match (after `/new`, say), resume asks the user to pick one on a terminal.
- Without a terminal, resume fails with `codex-link-ambiguous`. Set the link by hand with `ayran resume <id> --native <uuid>`.
- `codex-link-not-found` usually means the Session exited before the first prompt.

Passthrough such as `-- --resume …` creates a native session that the ayran record does not track. Use `ayran resume` instead.
