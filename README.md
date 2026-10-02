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

Install the latest stable release on Linux (x86_64 or aarch64):

```sh
curl -LsSf https://github.com/MLgentDev/ayran/releases/latest/download/ayran-cli-installer.sh | sh
```

On Windows (x86_64), use PowerShell:

```powershell
irm https://github.com/MLgentDev/ayran/releases/latest/download/ayran-cli-installer.ps1 | iex
```

Both installers use `~/.local/bin` (`%USERPROFILE%\\.local\\bin` on Windows)
and add it to your `PATH`. **The Windows build is experimental**: it is
compiled and linted in CI; the full test suite does not run on Windows yet.
CI runs the tests that do not require Unix shell fixtures.

Release candidates are prereleases and are not available through
`releases/latest`. For a candidate, replace `latest/download` in the installer
URL with `download/v0.1.0-rc.1`.

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

The justfile also provides `build`, `fmt-check`, `lint`, and `test` separately.
