# ayran

ayran launches a coding-agent CLI with only the capabilities a session needs.
It selects plugins, skills, MCP servers, and profiles for Codex, Claude Code,
and GitHub Copilot CLI.

See [Configure and run Ayran](example.md) for a complete example user
configuration, capability installation, shell aliases, and session commands.

This repository is a published mirror of a private development repository.

Pull requests aren't accepted. Issue reports are welcome through this
repository's GitHub Issues.

See [Installation](installation.md) for release installers, mise, source
builds, updates, uninstalling, and archive verification.

To verify the source, install the tool versions in `mise.toml`, then run:

```sh
just check
```


Choose Capabilities once in an Alias, then switch how the Session runs with a Preset in your user config:

```toml
[presets.sol]
harness = "codex"
model = "gpt-6-sol"
effort = "high"

[presets.luna]
harness = "codex"
model = "gpt-6-luna"
effort = "high"

[aliases.work]
preset = "sol"
profiles = ["coding"]  # a Profile defined in your config
```

After shell activation, `work` uses Sol and `work --luna` uses Luna with the same Capabilities. The canonical spelling is `ayran --alias work --preset luna`. Presets belong only in user config; changing their model also changes resumed Sessions that use them.
