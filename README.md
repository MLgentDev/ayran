# ayran

ayran launches a coding-agent CLI with only the capabilities a session needs.
It selects plugins, skills, MCP servers, and profiles for Codex, Claude Code,
and GitHub Copilot CLI.

This repository is a published mirror of a private development repository.

Pull requests aren't accepted. Issue reports are welcome through this
repository's GitHub Issues.

Install the latest stable release on Linux (x86_64 or aarch64):

```sh
curl -LsSf https://github.com/MLgentDev/ayran/releases/latest/download/ayran-cli-installer.sh | sh
```

On Windows (x86_64), use PowerShell:

```powershell
irm https://github.com/MLgentDev/ayran/releases/latest/download/ayran-cli-installer.ps1 | iex
```

Both installers use `~/.local/bin` (`%USERPROFILE%\\.local\\bin` on Windows)
and add it to your `PATH`. Windows builds are compiled, linted, and tested
in CI. Tests that require Unix shell fixtures run on Linux.

For installations made with these installers, update to the latest stable release:

```sh
ayran update
```

Use `ayran update --check` to check without installing. The command reports
the current version when up to date, and the old and new versions after an
update. Set `GITHUB_TOKEN` to use authenticated GitHub API requests with
higher rate limits. Updates require the cargo-dist install receipt for the
running executable. For source installs, reinstall from the latest source;
for manually unpacked archives, download a new archive or rerun an installer.

To verify a downloaded archive, download `sha256.sum` from the same release
and check its checksum, then verify its build attestation:

```sh
sha256sum --check --ignore-missing sha256.sum
gh attestation verify <file> -R MLgentDev/ayran
```

On Windows, compare `(Get-FileHash <file> -Algorithm SHA256).Hash` with the
entry in `sha256.sum`, then run the same `gh attestation verify` command.
The attestation identifies the public source commit and GitHub workflow
that built the archive.

To build and install from source with a Rust toolchain:

```sh
cargo install --path crates/ayran-cli --locked
```

To verify the source, install the tool versions in `mise.toml`, then run:

```sh
just check
```
