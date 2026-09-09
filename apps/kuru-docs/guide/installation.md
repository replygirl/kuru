# Installation & updates

Kuru runs on macOS and Linux. The executable is `kuru`. You can build from a checkout directly or use the repository's mise toolchain.

::: info Repository access
The repository is currently private. Source and release downloads require an authorized GitHub account. These public docs do not grant repository access; check the repository's available releases before choosing a version.
:::

## Build from source

Install Rust 1.98.1 and a C compiler, then clone with authenticated GitHub CLI:

```sh
gh repo clone replygirl/kuru
cd kuru
cargo install --path apps/kuru-tui --locked
```

Cargo installs into `$CARGO_HOME/bin`, normally `~/.cargo/bin`. Add that directory to your `PATH` if needed. SQLite is built with the application.

To choose another destination, use the source installer from the checkout:

```sh
KURU_INSTALL_DIR="$HOME/.local/bin" bash scripts/install.sh --source
```

The installer builds the locked release profile and replaces the executable atomically. It refuses a symlink or directory at the destination. It uses your current checkout; it does not fetch a newer revision.

## Install with mise

With [mise](https://mise.jdx.dev/getting-started.html) installed, run these commands inside the checkout:

```sh
mise trust
mise install
mise run setup
mise run install
```

The committed tool pins and lockfile select the toolchain. `setup` installs development dependencies, Rust verification components, and hooks. `install` builds and installs `kuru` into `~/.local/bin`; `KURU_INSTALL_DIR` changes that destination.

Confirm the installation, then try the offline provider:

```sh
kuru --version
kuru --provider demo
```

Continue with [your first conversation](./first-conversation) or [authentication](./authentication) for live inference.

## Install a release archive

Release installation and self-updates use native Rust code. The supported archive targets are:

| System | Architecture  | Target                      |
| ------ | ------------- | --------------------------- |
| macOS  | Apple Silicon | `aarch64-apple-darwin`      |
| macOS  | Intel         | `x86_64-apple-darwin`       |
| Linux  | ARM64         | `aarch64-unknown-linux-gnu` |
| Linux  | x86-64        | `x86_64-unknown-linux-gnu`  |

Linux archives are built on Ubuntu 24.04 and need a compatible glibc. Build from source on older Linux systems or musl distributions.

List published versions with `gh release list --repo replygirl/kuru`. Replace `VERSION` below with an available version such as a release's numeric `X.Y.Z`; the tag has a leading `v`.

From a checkout containing the installer:

```sh
gh release download vVERSION --repo replygirl/kuru --dir /tmp/kuru-release
bash scripts/install.sh \
  --release-base /tmp/kuru-release \
  --version VERSION \
  --install-dir "$HOME/.local/bin"
```

The checkout helper compiles the installer with Rust. An installed Kuru can update itself without a compiler or interpreter. The directory must contain `SHA256SUMS` and the matching `kuru-VERSION-TARGET.tar.gz`. The installer checks the checksum, archive entries, file types, size bounds, and executable permissions before replacing the binary. Download into a fresh directory when choosing a different release.

An HTTPS release directory also works when it is accessible without authentication. GitHub's private download URLs require the authenticated download step above. Checksums detect corruption; the release source must still be trusted.

## Update deliberately

For a source installation, select the desired revision and install it again:

```sh
kuru update --source /path/to/kuru
```

Or run `mise run install` from that checkout. For an accessible published release:

```sh
kuru update --version VERSION \
  --release-base https://github.com/replygirl/kuru/releases/download/vVERSION
```

For a private release, download it with `gh release download` first and supply its local directory as `--release-base`. Updates use the native installer and default to the directory containing the running executable. They require an explicit source or version; Kuru does not install background updates.

## If the first launch fails

| Symptom                                 | Next step                                                                                          |
| --------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `kuru` is not found                     | Add the installation directory to `PATH` or run the full executable path.                          |
| Codex cannot be started                 | Try `kuru --provider demo`, then install Codex as described in [authentication](./authentication). |
| A release cannot be downloaded          | Confirm the version exists and your GitHub account can access the repository.                      |
| State directory is inside the workspace | Set `--data-dir` to a private directory outside the project.                                       |
