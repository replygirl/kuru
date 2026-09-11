# Installation and updates

Kuru ships native executables for macOS and Linux on arm64 and x86-64, and Windows
on x86-64. Binary installation requires no separately installed compiler, Dolt
server or MSVC redistributable. Run it as `kuru` (`kuru.exe` on Windows).
ChatGPT sign-in and OpenAI model requests are native to Kuru; no Codex CLI,
Node or npm installation is needed for them.

## Install with mise

With [mise](https://mise.jdx.dev/getting-started.html) installed and activated:

```sh
mise use -g github:replygirl/kuru
kuru --version
```

`mise use -g` installs the release and selects it in your global configuration.
Omit `-g` to select it for the current project.

To install and activate an exact version, replace `VERSION` in the examples below
with a full `major.minor.patch` version from
[releases](https://github.com/replygirl/kuru/releases) that includes your platform's
archive:

```sh
mise use -g github:replygirl/kuru@VERSION
```

`mise install github:replygirl/kuru@VERSION` downloads that version without changing
the active selection; run it explicitly with
`mise exec github:replygirl/kuru@VERSION -- kuru`.

Mise's GitHub backend applies a default release-age cooldown to latest-version
resolution. A full `major.minor.patch` pin bypasses that cooldown when installing
a newly published release. See [available releases](https://github.com/replygirl/kuru/releases).

## Install with the shell bootstrap

On macOS or Linux:

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
bash /tmp/kuru-install.sh --version VERSION --install-dir "$HOME/.local/bin"
```

`KURU_INSTALL_DIR` also sets the destination; `--install-dir` takes precedence.
Use `--help` for all options. An existing symlink or directory at the `kuru`
destination is rejected. Before replacement, failed downloads, validation errors
and handled interruptions preserve the previous executable and remove staging files.

## Install with PowerShell

In stock Windows PowerShell 5.1:

```powershell
irm https://raw.githubusercontent.com/replygirl/kuru/main/packages/kuru-delivery/support/install.ps1 | iex
```

The default destination is `$env:LOCALAPPDATA\Programs\kuru\bin`. Add that directory to
your user `PATH`, then run `kuru --version` and `kuru --provider demo`. The script
uses stock Windows PowerShell and .NET Framework. Its small native API bridge
checks file identities, private staging and durable publication; no separate
compiler installation is needed.

To choose an exact version or use a local release directory:

```powershell
irm https://raw.githubusercontent.com/replygirl/kuru/main/packages/kuru-delivery/support/install.ps1 -OutFile "$env:TEMP\kuru-install.ps1"
& "$env:TEMP\kuru-install.ps1" -Version VERSION -ReleaseBase C:\Downloads\kuru-release -InstallDir "$env:LOCALAPPDATA\Programs\kuru\bin"
```

Replace `VERSION` with the release stored in that directory. Choose an ordinary
directory on a local drive for installation. `-InstallDir`
and `-ReleaseBase` override `KURU_INSTALL_DIR` and
`KURU_RELEASE_BASE`. A custom release directory requires `-Version`. The script
freezes latest to an explicit version, verifies the ZIP checksum and its exact
three regular members, and installs only `kuru.exe`. It never runs the downloaded
candidate to validate it. Reparse points, extra hardlinks, unsafe names and
unexpected installation objects are refused.

## Sign in after installation

For ChatGPT subscription access with the default `codex` provider:

```sh
kuru login
kuru auth
kuru models
kuru
```

Use `kuru login --no-browser` to open the printed URL yourself, or
`kuru login --device` for device authorization. Kuru keeps credentials in its
own private data directory and does not copy another application's auth store.
`kuru auth` reports redacted local status; `kuru logout` clears Kuru's ChatGPT
credentials.

For API-key access, supply `OPENAI_API_KEY` through your environment or secret
manager and run `kuru --provider responses --model MODEL_ID`. Provider selection
stays explicit. See [authentication configuration](configuration.md#authentication)
for storage overrides and migration from the removed `codex_command` setting.

## Supported platforms

| System | Architecture | Target |
| --- | --- | --- |
| macOS | Apple Silicon | `aarch64-apple-darwin` |
| macOS | Intel | `x86_64-apple-darwin` |
| Linux | ARM64 | `aarch64-unknown-linux-gnu` |
| Linux | x86-64 | `x86_64-unknown-linux-gnu` |
| Windows 10 version 1809 or newer | x86-64 | `x86_64-pc-windows-msvc` |

Linux archives are built on Ubuntu 24.04 and require a compatible glibc. The
supported Linux targets use GNU libc, including source builds with the bundled
engine. The shell bootstrap detects its macOS/Linux host; `--target` selects
another supported tar target. PowerShell uses the Windows x86-64 ZIP and accepts
`-Target x86_64-pc-windows-msvc` explicitly.

## Release archives

Release archives use `kuru-VERSION-TARGET.tar.gz` for macOS/Linux and
`kuru-VERSION-x86_64-pc-windows-msvc.zip` for Windows, alongside `SHA256SUMS`.
Each archive includes the executable,
`LICENSE` and `README.md`. To install from
an HTTPS mirror, pass its literal version directory and an explicit version:

```sh
bash /tmp/kuru-install.sh --version VERSION \
  --release-base https://github.com/replygirl/kuru/releases/download/vVERSION
```

For offline installation, download the matching archive and `SHA256SUMS` into
one directory and use that directory as `--release-base`:

```sh
bash /tmp/kuru-install.sh --version VERSION --release-base /path/to/release-files
```

`KURU_RELEASE_BASE` sets the same option; the CLI takes precedence. Custom bases
require `--version` and are used directly, without appending another version
directory. Remote sources and redirects must use HTTPS. Downloads and extraction
are bounded, and checksums are verified before extraction. Checksums detect
corruption; trust comes from the release source you choose.

From a checkout, `bash scripts/install.sh` forwards these binary-install options
to the shell bootstrap; `& .\scripts\install.ps1` forwards PowerShell options to
the Windows bootstrap.

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

On Windows, install the Visual Studio C++ Build Tools and Windows SDK, then use
the native PowerShell entrypoint from the checkout:

```powershell
& .\scripts\install.ps1 -Source
```

The owning mise task builds the explicit MSVC target with a static CRT. To set
the destination, pass `-InstallDir` or set `KURU_INSTALL_DIR`; release-selection
options are not source-build options.

The source installer prepares the pinned Rust toolchain and verified engine
archive through package-owned mise tasks. It installs into `~/.local/bin` on
macOS/Linux or `$env:LOCALAPPDATA\Programs\kuru\bin` on Windows.
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

For a direct installation, rerun the corresponding bootstrap to install latest, or choose an
explicit release through Kuru's native updater:

```sh
kuru update --version VERSION --release-base https://github.com/replygirl/kuru/releases/download/vVERSION
```

Replace `VERSION` in both places with the desired release. This command also
works in PowerShell. The updater requires an
explicit version and release directory, which can also be a local directory.
It validates the archive in Rust and defaults to replacing the running executable.
It requires no compiler or interpreter. Kuru does not install background updates.

On Windows, the current trusted executable performs replacement through a
verified helper copy. It records publication before reporting success and waits
for the old process to exit before cleanup. Publication has a two-minute
acknowledgment deadline to allow for verified copies of the bundled executable;
connection startup and individual protocol frames retain shorter deadlines.
Close other old Kuru instances if
cleanup remains pending. Rerunning the normal PowerShell installer reconciles
the private receipt, including an interrupted update where `kuru.exe` is absent.
It refuses an unknown replacement at that path. A verified helper cache entry is
retained for recovery and occupies approximately one executable per updated
current-version digest.

For a source installation, select the desired revision and install it again, or
run `kuru update --source /path/to/kuru`.

## Releasing

Maintainers dispatch the Release workflow from main. It calculates the next
version from conventional commits, runs the gate, creates or recovers the signed
version commit, builds five native targets, generates Communiqué notes, and
publishes the release before deploying its docs. See [release operations](release.md)
for credentials and recovery.
