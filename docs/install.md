# Installation and updates

The executable is `kuru`; the Cargo package lives at `apps/kuru-tui`. Linux and macOS
are the initial release targets. Building from source requires Rust 1.98.1 and a
C compiler for SQLite. Installation and self-updates use native Rust code.

## Direct source installation

Clone the repository with an authorized GitHub account while it is private:

```sh
gh repo clone replygirl/kuru
cd kuru
```

From the checkout:

```sh
cargo install --path apps/kuru-tui --locked
```

Cargo installs under `$CARGO_HOME/bin`, normally `~/.cargo/bin`. For an arbitrary
binary directory, run:

```sh
KURU_INSTALL_DIR="$HOME/.local/bin" bash scripts/install.sh --source
```

The source installer builds the locked release profile, stages the executable
in the destination filesystem and renames it into place. It refuses a symlink
or directory at the `kuru` destination. It does not fetch source updates: select
the source revision you want before running it.

## Installation through mise

```sh
mise trust
mise install
mise run install
```

`mise install` uses committed tool pins and the lockfile. `install` runs the same
source installer. This workflow works before a hosted release exists on both
Apple Silicon and Intel macOS, as well as Linux. The separate `mise run setup`
command prepares maintainer checks and release-note tools; app installation
does not require it. See [development](development.md) for that toolchain.

## Release archives

No tagged release exists yet. After a release is published and the repository
is public, the versioned release installer can use:

```sh
bash scripts/install.sh \
  --release-base https://github.com/replygirl/kuru/releases/download/v0.1.0 \
  --version 0.1.0 \
  --install-dir "$HOME/.local/bin"
```

While the repository is private, use source installation or download an available
release with authenticated GitHub CLI and install from its local directory:

```sh
gh release download v0.1.0 --repo replygirl/kuru --dir /tmp/kuru-release
bash scripts/install.sh --release-base /tmp/kuru-release \
  --version 0.1.0 --install-dir "$HOME/.local/bin"
```

The checkout helper compiles the native installer using the pinned Rust
toolchain. An already installed Kuru updates itself without a compiler or
interpreter. These release commands require that version to exist; creating the repository
does not create a release. The base directory
must serve `SHA256SUMS` and `kuru-VERSION-TARGET.tar.gz`; local directories also
work for offline installation. Remote sources require HTTPS. The installer
checks the archive's SHA-256 hash, exact expected entries, regular-file types,
size bounds and executable permissions, then atomically replaces the binary.
Checksums detect corruption; trust still comes from the release source you choose.

Supported target triples are `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`aarch64-unknown-linux-gnu` and `x86_64-unknown-linux-gnu`. Linux archives are
built on Ubuntu 24.04 and require a compatible glibc; use source installation
on older Linux systems or musl distributions.

## Updating

For a source install, install again from the desired checkout revision, or use
`kuru update --source /path/to/kuru`. For a published binary release, use
`kuru update --version VERSION --release-base HTTPS_VERSION_DIRECTORY`.
The CLI uses the same native archive installer; its default destination is the
directory containing the running executable. You
can set `KURU_RELEASE_BASE` to a version directory for the release installer.
Updates require an explicit version and do not silently fetch or install
background updates. Mise-managed source installations use `mise run install`
after moving the checkout to the intended version.

## Releasing

Maintainers dispatch the Release workflow from main. It calculates the next
version from conventional commits, runs the gate, creates a signed version
commit with the scoped release app, builds four native targets, generates
Communiqué notes, and publishes a complete release before deploying its docs.
See [release operations](release.md) for credentials, retry rules and exact
permissions. No token needs to be placed in a local configuration file.
