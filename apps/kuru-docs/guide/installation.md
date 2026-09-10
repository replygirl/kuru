# Installation & updates

Kuru ships native executables for macOS and Linux. Install with mise or the shell bootstrap, then run `kuru`.

## Install with mise

With [mise](https://mise.jdx.dev/getting-started.html) installed and activated:

```sh
mise use -g github:replygirl/kuru
kuru --version
```

This installs the native executable and selects it globally. Omit `-g` for a project-local selection. To install and activate an exact release:

```sh
mise use -g github:replygirl/kuru@0.1.0
```

Mise applies a release-age cooldown when resolving latest; a full `major.minor.patch` pin also lets you select a newly published release. `mise install github:replygirl/kuru@0.1.0` downloads the version without selecting it. Use `mise exec github:replygirl/kuru@0.1.0 -- kuru` to run that version explicitly.

## Install with the shell bootstrap

```sh
curl -fsSL https://raw.githubusercontent.com/replygirl/kuru/main/packages/kuru-delivery/support/install.sh | bash
export PATH="$HOME/.local/bin:$PATH"
kuru --version
```

The bootstrap installs the latest release into `~/.local/bin`. Add that directory to your shell configuration's `PATH` for future terminals. Installation uses Bash 3.2 or newer, curl, tar, gzip, and either `sha256sum` or `shasum`, plus standard system utilities; no Rust toolchain is required.

To choose a version and destination:

```sh
curl -fsSL https://raw.githubusercontent.com/replygirl/kuru/main/packages/kuru-delivery/support/install.sh -o /tmp/kuru-install.sh
bash /tmp/kuru-install.sh --version 0.1.0 --install-dir "$HOME/.local/bin"
```

`KURU_INSTALL_DIR` also sets the destination; the CLI option takes precedence. The installer resolves latest once to a specific version, verifies the archive checksum, and extracts only the executable into staging before replacing it atomically. It does not run the candidate during installation. It rejects symlink or directory destinations. Failures and handled interruptions before replacement preserve the existing executable.

Try a conversation without credentials:

```sh
kuru --provider demo
```

Continue with [your first conversation](./first-conversation) or [authentication](./authentication) for live inference.

## Supported platforms

| System | Architecture  | Target                      |
| ------ | ------------- | --------------------------- |
| macOS  | Apple Silicon | `aarch64-apple-darwin`      |
| macOS  | Intel         | `x86_64-apple-darwin`       |
| Linux  | ARM64         | `aarch64-unknown-linux-gnu` |
| Linux  | x86-64        | `x86_64-unknown-linux-gnu`  |

Linux archives are built on Ubuntu 24.04 and need a compatible glibc. Build from source on older Linux systems or musl distributions. `--target` overrides the bootstrap's host detection with one of these targets.

## Release archives and mirrors

Each [release](https://github.com/replygirl/kuru/releases) contains `SHA256SUMS` and a `kuru-VERSION-TARGET.tar.gz` archive for each platform. To use a mirror, pass its HTTPS version directory and an explicit version:

```sh
bash /tmp/kuru-install.sh --version 0.1.0 \
  --release-base https://github.com/replygirl/kuru/releases/download/v0.1.0
```

For offline installation, place the matching archive and `SHA256SUMS` in one local directory:

```sh
bash /tmp/kuru-install.sh --version 0.1.0 --release-base /path/to/release-files
```

`KURU_RELEASE_BASE` sets the same option; the CLI overrides it. A custom base requires `--version` and is used directly. Remote downloads and redirects require HTTPS. Checksums are verified before extraction, and downloads and expanded output have size limits. Checksums detect corruption; choose a release source you trust.

## Build from source

Install Rust 1.98.1 and a C compiler, then:

```sh
git clone https://github.com/replygirl/kuru.git
cd kuru
cargo install --path apps/kuru-tui --locked
```

Cargo installs into `$CARGO_HOME/bin`, normally `~/.cargo/bin`. Add that directory to your `PATH` if needed. The legacy SQLite importer is built with the application; live memory uses the verified native Dolt runtime provisioned on first use.

To choose another destination:

```sh
KURU_INSTALL_DIR="$HOME/.local/bin" bash scripts/install.sh --source
```

The source installer builds the locked release profile and replaces the executable atomically. It refuses a symlink or directory at the destination and uses the current checkout revision. From a configured maintainer checkout, `mise run install` performs the same source installation. Binary installation does not require the repository's maintainer tools.

## Updating {#update-deliberately}

For a mise-managed binary selected as latest:

```sh
mise upgrade github:replygirl/kuru
```

For an exact pin, select the new version with `mise use -g github:replygirl/kuru@VERSION`. Update mise-managed binaries through mise.

For a shell installation, rerun the bootstrap to install latest, or select an explicit release with Kuru's native updater:

```sh
kuru update --version 0.1.0 \
  --release-base https://github.com/replygirl/kuru/releases/download/v0.1.0
```

Replace `0.1.0` in both places with the desired version. A local release directory also works. The updater validates the archive in Rust and defaults to replacing the running executable. No compiler or interpreter is required. Updates are explicit; Kuru does not install background updates.

For a source installation, select the desired revision and install again, or run:

```sh
kuru update --source /path/to/kuru
```

## If the first launch fails

| Symptom                                               | Next step                                                                                          |
| ----------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `kuru` is not found                                   | Activate mise or add the installation directory to `PATH`.                                         |
| A new release is missing from mise's latest selection | Select its full version, such as `github:replygirl/kuru@0.1.0`.                                    |
| Codex cannot be started                               | Try `kuru --provider demo`, then install Codex as described in [authentication](./authentication). |
| A release cannot be downloaded                        | Check the version and release directory against the available release assets.                      |
| State directory is inside the workspace               | Set `--data-dir` to a private directory outside the project.                                       |
