# Installation and updates

The executable is `kuru`; the Cargo package lives at `apps/kuru-tui`. Linux and macOS
are the initial release targets. Building from source requires Rust 1.98.1 and a
C compiler for SQLite. Python 3.10 or newer is required for binary release
installation and updates; running chat does not invoke Python.

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
mise run setup
mise run install
```

`mise install` uses committed tool pins and the lockfile. `setup` installs the
pinned OpenSpec dependency used by cospec and the Rust components needed for
formatting, lint and coverage, then installs hk hooks. `install` runs the same
source installer. This workflow works before a hosted release exists.

## Release archives

No tagged release exists yet. After a release is published and the repository
is public, the versioned release installer can use:

```sh
python3 scripts/install_release.py \
  --release-base https://github.com/replygirl/kuru/releases/download/v0.1.0 \
  --version 0.1.0 \
  --install-dir "$HOME/.local/bin"
```

While the repository is private, use source installation or download an available
release with authenticated GitHub CLI and install from its local directory:

```sh
gh release download v0.1.0 --repo replygirl/kuru --dir /tmp/kuru-release
python3 scripts/install_release.py --release-base /tmp/kuru-release \
  --version 0.1.0 --install-dir "$HOME/.local/bin"
```

These release commands require that version to exist; creating the repository
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
The CLI embeds the same verified installer and calls local `python3`; its
default destination is the directory containing the running executable. You
can set `KURU_RELEASE_BASE` to a version directory for the release installer.
Updates require an explicit version and do not silently fetch or install
background updates. Mise-managed source installations use `mise run install`
after moving the checkout to the intended version.

## Releasing

Maintainers change `[workspace.package].version`, regenerate Cargo.lock, pass
`mise run check`, archive the cospec change, and publish a matching `vX.Y.Z` tag
when release publication is authorized. The release workflow rejects mismatched
tags, runs the full gate, builds all four native targets, creates archives and
checksums, verifies the checksums, and attaches them to the GitHub release.
No token needs to be placed in a local configuration file.
