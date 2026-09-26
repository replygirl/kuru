# Installation & updates

Kuru ships native executables for macOS, Linux and Windows, with the verified full-Dolt engine and its licenses included. Install with mise or your platform's bootstrap, then run `kuru`.

## Install with mise

With [mise](https://mise.jdx.dev/getting-started.html) installed and activated:

```sh
mise use -g github:replygirl/kuru
kuru --version
```

This installs the native executable and selects it globally. Omit `-g` for a project-local selection. To install and activate an exact release, replace `VERSION` in the examples below with a full `major.minor.patch` version from [releases](https://github.com/replygirl/kuru/releases) that includes your platform's archive:

```sh
mise use -g github:replygirl/kuru@VERSION
```

Mise applies a release-age cooldown when resolving latest; a full `major.minor.patch` pin also lets you select a newly published release. `mise install github:replygirl/kuru@VERSION` downloads the version without selecting it. Use `mise exec github:replygirl/kuru@VERSION -- kuru` to run that version explicitly.

Before publication, each release's exact Windows package is tested through native mise on Windows, including its bundled Dolt runtime and reopening a saved offline conversation from empty application and engine caches. Installation still needs no compiler or separate database.

Ordinary Kuru commands start or attach to a private per-project memory service from the installed executable. It is not registered as an operating-system service, requires no separately installed database daemon, and exits after a bounded idle grace once clients and accepted work have drained.

## Install with the shell bootstrap

On macOS or Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/replygirl/kuru/main/packages/kuru-delivery/support/install.sh | bash
export PATH="$HOME/.local/bin:$PATH"
kuru --version
```

The bootstrap installs the latest release into `~/.local/bin`. Add that directory to your shell configuration's `PATH` for future terminals. Installation uses Bash 3.2 or newer, curl, tar, gzip, and either `sha256sum` or `shasum`, plus standard system utilities; no Rust toolchain is required.

To choose a version and destination:

```sh
curl -fsSL https://raw.githubusercontent.com/replygirl/kuru/main/packages/kuru-delivery/support/install.sh -o /tmp/kuru-install.sh
bash /tmp/kuru-install.sh --version VERSION --install-dir "$HOME/.local/bin"
```

`KURU_INSTALL_DIR` also sets the destination; the CLI option takes precedence. The installer resolves latest once to a specific version, verifies the archive checksum, and extracts only the executable into staging before replacing it atomically. It does not run the candidate during installation. It rejects symlink or directory destinations. Failures and handled interruptions before replacement preserve the existing executable.

Try a conversation without credentials:

```sh
kuru --provider demo
```

Continue with [your first conversation](./first-conversation) or [authentication](./authentication) for live inference.

## Install with PowerShell

In stock Windows PowerShell 5.1:

```powershell
irm https://raw.githubusercontent.com/replygirl/kuru/main/packages/kuru-delivery/support/install.ps1 | iex
$env:PATH = "$env:LOCALAPPDATA\Programs\kuru\bin;$env:PATH"
kuru --version
```

The default destination is `$env:LOCALAPPDATA\Programs\kuru\bin`. Add it to your user `PATH` for future terminals. Installation uses stock PowerShell/.NET facilities and needs no separately installed compiler, Dolt server or MSVC redistributable. It loads PowerShell's own Management and Utility modules directly from `$PSHOME`, so a fresh Windows profile does not search every installed module before the first stage.

To select a release and destination, download the script first:

```powershell
irm https://raw.githubusercontent.com/replygirl/kuru/main/packages/kuru-delivery/support/install.ps1 -OutFile "$env:TEMP\kuru-install.ps1"
& "$env:TEMP\kuru-install.ps1" -Version VERSION -InstallDir C:\Tools\kuru\bin
```

Replace `VERSION` with a version from [releases](https://github.com/replygirl/kuru/releases). `-InstallDir` overrides `KURU_INSTALL_DIR`; use an ordinary directory on a local drive. `-ReleaseBase` overrides `KURU_RELEASE_BASE` and accepts an HTTPS version directory or a local directory containing the Windows ZIP and `SHA256SUMS`. A custom base requires `-Version`.

The bootstrap freezes latest to an explicit version, verifies the ZIP checksum and its exact three regular members, then publishes `kuru.exe` from private staging. It refuses reparse points, extra hardlinks and ambiguous names. It does not execute the downloaded candidate for validation. A private `.kuru-update` directory beside the executable coordinates installation and recovery.

## Supported platforms

| System                           | Architecture  | Target                      |
| -------------------------------- | ------------- | --------------------------- |
| macOS                            | Apple Silicon | `aarch64-apple-darwin`      |
| Linux                            | ARM64         | `aarch64-unknown-linux-gnu` |
| Linux                            | x86-64        | `x86_64-unknown-linux-gnu`  |
| Windows 10 version 1809 or newer | x86-64        | `x86_64-pc-windows-msvc`    |

Linux archives are built on Ubuntu 24.04 and need a compatible glibc. Linux support uses GNU targets; musl targets are not supported. The shell bootstrap's `--target` overrides host detection for supported tar targets. PowerShell selects Windows x86-64 and accepts `-Target x86_64-pc-windows-msvc` explicitly.

## Release archives and mirrors

Release archives use `kuru-VERSION-TARGET.tar.gz` for macOS/Linux and `kuru-VERSION-x86_64-pc-windows-msvc.zip` for Windows, alongside `SHA256SUMS`. Archives contain the executable, `LICENSE` and `README.md`. To use a mirror, pass its HTTPS version directory and an explicit version:

```sh
bash /tmp/kuru-install.sh --version VERSION \
  --release-base https://github.com/replygirl/kuru/releases/download/vVERSION
```

For offline installation, place the matching archive and `SHA256SUMS` in one local directory:

```sh
bash /tmp/kuru-install.sh --version VERSION --release-base /path/to/release-files
```

`KURU_RELEASE_BASE` sets the same option; the CLI overrides it. A custom base requires `--version` and is used directly. Remote downloads and redirects require HTTPS. Checksums are verified before extraction, and downloads and expanded output have size limits. Checksums detect corruption; choose a release source you trust.

## Build from source

Install mise, a C compiler for the legacy SQLite importer, and standard platform build tools, then:

```sh
git clone https://github.com/replygirl/kuru.git
cd kuru
bash scripts/install.sh --source
```

On Windows, install Visual Studio C++ Build Tools and the Windows SDK, then run this from the checkout:

```powershell
& .\scripts\install.ps1 -Source
```

The source installer prepares the pinned Rust toolchain and verified engine archive through package-owned mise tasks. It installs the complete executable into `~/.local/bin` on macOS/Linux or `$env:LOCALAPPDATA\Programs\kuru\bin` on Windows. Add that directory to your `PATH` if needed.

To choose another destination:

```sh
KURU_INSTALL_DIR="$HOME/.local/bin" bash scripts/install.sh --source
```

On Windows, use `& .\scripts\install.ps1 -Source -InstallDir C:\Tools\kuru\bin`. Release-selection options do not apply to source builds. The Windows task builds the explicit MSVC target with a static C runtime.

The source installer builds the locked release profile for the current host and replaces the executable atomically. It refuses a symlink or directory at the destination and uses the current checkout revision. From a configured maintainer checkout, `mise run install` performs the same source installation. Source installation prepares only its required tooling; it does not require the full maintainer setup.

### Prepare engine build inputs

Ordinary mise build tasks prepare the engine archive automatically. To import a pinned archive for an offline build:

```sh
mise run //packages/kuru-memory:bundle:prepare -- \
  --target x86_64-unknown-linux-gnu \
  --archive /absolute/path/to/dolt-linux-amd64.tar.gz --offline
```

Select your target and its matching archive from `packages/kuru-memory/support/dolt-assets.json`. Imported bytes receive the same size and checksum verification as downloads. `KURU_DOLT_BUNDLE_DIR` selects an absolute build-input cache for preparation and compilation; `KURU_DOLT_BUNDLE_OFFLINE=true` prevents preparation from downloading missing archives. The Rust toolchain and Cargo dependencies also need to be available before building offline.

Cargo embeds verified local input for its actual target and never downloads an engine itself. Prepare that input before a direct Cargo build. See the [developer build-input guide](https://github.com/replygirl/kuru/blob/main/docs/development.md#bundled-engine-build-inputs) for explicit target builds and mirrors. Installed Kuru extracts its bundled engine locally, so a first offline demo conversation needs no build cache or compiler.

## Updating {#update-deliberately}

For a mise-managed binary selected as latest:

```sh
mise upgrade github:replygirl/kuru
```

For an exact pin, select the new version with `mise use -g github:replygirl/kuru@VERSION`. Update mise-managed binaries through mise.

For a direct installation, rerun your platform's bootstrap to install latest, or select an explicit release with Kuru's native updater:

```sh
kuru update --version VERSION --release-base https://github.com/replygirl/kuru/releases/download/vVERSION
```

Replace `VERSION` in both places with the desired version. This command also works in PowerShell; a local release directory also works. The updater validates the archive in Rust and replaces the running executable. No compiler or interpreter is required. Updates are explicit; Kuru does not install background updates.

On Windows, a verified copy of the current running executable performs publication and records its result before success is reported. It waits for the original process to exit before deleting the displaced image. The trusted helper stays in a private cache for recovery. Close other old Kuru instances if cleanup remains pending, then rerun the normal PowerShell installer. It reconciles the receipt even if an interrupted update left `kuru.exe` absent, and refuses an unknown occupant at that path. `-Recover` performs recovery alone.

For a source installation, select the desired revision and install again, or run:

```sh
kuru update --source /path/to/kuru
```

## If the first launch fails

| Symptom                                               | Next step                                                                               |
| ----------------------------------------------------- | --------------------------------------------------------------------------------------- |
| `kuru` is not found                                   | Activate mise or add the installation directory to `PATH`.                              |
| A new release is missing from mise's latest selection | Select its full version with `github:replygirl/kuru@VERSION`.                           |
| ChatGPT is not authenticated                          | Run `kuru login`, or use `kuru login --device`; see [authentication](./authentication). |
| A release cannot be downloaded                        | Check the version and release directory against the available release assets.           |
| State directory is inside the workspace               | Set `--data-dir` to a private directory outside the project.                            |
