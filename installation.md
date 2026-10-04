# Install Ayran

## Release installers

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

## Update a script installation

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

## Uninstall a script installation

To uninstall a default script installation on Linux:

```sh
rm -f "$HOME/.local/bin/ayran"
```

For optional installer cleanup, remove lines referencing `ayran-cli/env` from
your shell startup files, then delete `~/.config/ayran-cli`. Keep
`~/.config/ayran/ayran.toml` to preserve your settings. Open a new terminal afterward.

## Install with mise

Install Ayran with [mise's GitHub backend](https://mise.jdx.dev/dev-tools/backends/github.html)
on the supported Linux and Windows platforms:

```sh
mise use -g github:MLgentDev/ayran@latest
mise exec -- ayran --version
```

Omit `-g` to record the tool in the current project's `mise.toml` instead.
With mise activated in your shell, run `ayran` normally. Manage updates through
mise:

```sh
mise upgrade github:MLgentDev/ayran
```

## Verify downloaded archives

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

## Install from source

To build and install from source with a Rust toolchain:

```sh
cargo install --path crates/ayran-cli --locked
```

