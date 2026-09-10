# Installation and updates

Kuru ships native executables for macOS and Linux on arm64 and x86-64. Binary
installation requires no Rust toolchain. The executable is `kuru`.

## Install with mise

With [mise](https://mise.jdx.dev/getting-started.html) installed and activated:

```sh
mise use -g github:replygirl/kuru
kuru --version
```

`mise use -g` installs the release and selects it in your global configuration.
Omit `-g` to select it for the current project. `mise install
github:replygirl/kuru@0.1.0` downloads an exact version without changing the active
selection; run it explicitly with `mise exec github:replygirl/kuru@0.1.0 -- kuru`.

To install and activate an exact version:

```sh
mise use -g github:replygirl/kuru@0.1.0
```

Mise's GitHub backend applies a default release-age cooldown to latest-version
resolution. A full `major.minor.patch` pin bypasses that cooldown when installing
a newly published release. See [available releases](https://github.com/replygirl/kuru/releases).

## Install with the shell bootstrap

```sh
curl -fsSL https://raw.githubusercontent.com/replygirl/kuru/main/packages/kuru-delivery/support/install.sh | bash
```

The default destination is `~/.local/bin`. Add it to your shell's `PATH`, then try
the offline provider:

```sh
export PATH="$HOME/.local/bin:$PATH"
kuru --version
kuru --provider demo
```

The bootstrap requires Bash 3.2 or newer, curl, tar, gzip, and either `sha256sum`
or `shasum`, alongside standard macOS/Linux command-line utilities. It resolves
latest once, then downloads the selected version's immutable archive and verifies
its SHA-256 digest. It extracts only the executable into staging beside the
destination and replaces it atomically after validation. It does not run the
downloaded executable during installation.

To choose a version and destination, download the script and pass options:

```sh
curl -fsSL https://raw.githubusercontent.com/replygirl/kuru/main/packages/kuru-delivery/support/install.sh -o /tmp/kuru-install.sh
bash /tmp/kuru-install.sh --version 0.1.0 --install-dir "$HOME/.local/bin"
```

`KURU_INSTALL_DIR` also sets the destination; `--install-dir` takes precedence.
Use `--help` for all options. An existing symlink or directory at the `kuru`
destination is rejected. Before replacement, failed downloads, validation errors
and handled interruptions preserve the previous executable and remove staging files.

## Supported platforms

| System | Architecture | Target |
| --- | --- | --- |
| macOS | Apple Silicon | `aarch64-apple-darwin` |
| macOS | Intel | `x86_64-apple-darwin` |
| Linux | ARM64 | `aarch64-unknown-linux-gnu` |
| Linux | x86-64 | `x86_64-unknown-linux-gnu` |

Linux archives are built on Ubuntu 24.04 and require a compatible glibc. The
supported Linux targets use GNU libc, including source builds with the bundled
engine. The bootstrap detects the host; `--target` explicitly selects one of
the supported archive targets.

## Release archives

Each release contains `SHA256SUMS` and `kuru-VERSION-TARGET.tar.gz`. To install from
an HTTPS mirror, pass its literal version directory and an explicit version:

```sh
bash /tmp/kuru-install.sh --version 0.1.0 \
  --release-base https://github.com/replygirl/kuru/releases/download/v0.1.0
```

For offline installation, download the matching archive and `SHA256SUMS` into
one directory and use that directory as `--release-base`:

```sh
bash /tmp/kuru-install.sh --version 0.1.0 --release-base /path/to/release-files
```

`KURU_RELEASE_BASE` sets the same option; the CLI takes precedence. Custom bases
require `--version` and are used directly, without appending another version
directory. Remote sources and redirects must use HTTPS. Downloads and extraction
are bounded, and checksums are verified before extraction. Checksums detect
corruption; trust comes from the release source you choose.

From a checkout, `bash scripts/install.sh` forwards these binary-install options
to the same bootstrap.

The executable includes its verified full-Dolt engine and upstream licenses.
First memory use extracts them locally, so an offline installation also supports
a first offline demo conversation. See [memory storage](memory.md).

## Build from source

Building requires mise, a C compiler for the legacy SQLite importer, and standard
platform build tools:

```sh
git clone https://github.com/replygirl/kuru.git
cd kuru
bash scripts/install.sh --source
```

The source installer prepares the pinned Rust toolchain and verified engine
archive through package-owned mise tasks, then installs into `~/.local/bin`.
It builds for the current host and rejects a foreign `CARGO_BUILD_TARGET` before
installing an executable; use the build task for cross-compilation.
For another binary directory:

```sh
KURU_INSTALL_DIR="$HOME/.local/bin" bash scripts/install.sh --source
```

The source installer builds the locked release profile and atomically replaces
the executable. It refuses a symlink or directory at the destination and uses
your selected checkout revision. Maintainers with the repository toolchain can
run `mise run install`; see [development](development.md) for setup and platform
requirements. Source installation does not require the full maintainer toolchain;
it omits mise's repository-setup postinstall hook while preparing Rust. It does
not alter Git commit or push hooks.

For direct Cargo builds or offline source preparation, see the
[bundled engine build inputs](development.md#bundled-engine-build-inputs) guide.

## Updating

For a mise-managed binary selected as latest:

```sh
mise upgrade github:replygirl/kuru
```

For an exact pin, select the new version with `mise use -g
github:replygirl/kuru@VERSION`. Use mise to update its managed binaries.

For a shell installation, rerun the bootstrap to install latest, or choose an
explicit release through Kuru's native updater:

```sh
kuru update --version 0.1.0 \
  --release-base https://github.com/replygirl/kuru/releases/download/v0.1.0
```

Replace `0.1.0` in both places with the desired release. The updater requires an
explicit version and release directory, which can also be a local directory.
It validates the archive in Rust and defaults to replacing the running executable.
It requires no compiler or interpreter. Kuru does not install background updates.

For a source installation, select the desired revision and install it again, or
run `kuru update --source /path/to/kuru`.

## Releasing

Maintainers dispatch the Release workflow from main. It calculates the next
version from conventional commits, runs the gate, creates or recovers the signed
version commit, builds four native targets, generates Communiqué notes, and
publishes the release before deploying its docs. See [release operations](release.md)
for credentials and recovery.
