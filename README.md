# ayran

ayran launches a coding-agent CLI with only the capabilities a session needs.
It selects plugins, skills, MCP servers, and profiles for Codex, Claude Code,
and GitHub Copilot CLI.

This repository is a published mirror of a private development repository.
Each snapshot commit has a `Source:` trailer naming the development commit
it was published from. All changes originate in that development repository;
direct edits here are overwritten by the next snapshot.

Pull requests aren't accepted. Issue reports are welcome through this
repository's GitHub Issues.

To verify the source, install the tool versions in `mise.toml`, then run:

```sh
just check
```

The justfile also provides `build`, `fmt-check`, `lint`, and `test` separately.
