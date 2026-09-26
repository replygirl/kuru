## Why

Kuru ships native executables for four targets once #106 removes Intel macOS, but Windows on Arm users have no
native release: an x64 `kuru.exe` runs only under emulation, and the x64 bootstrap
refuses an Arm host outright. Upstream Dolt publishes no windows/arm64 asset, so the
engine must be built from source; the companion change `dolt-source-build-inputs`
(PR6a) adds that pinned, verified build input as inert manifest data. This change
makes `aarch64-pc-windows-msvc` a delivered target end to end: catalog, installer,
updater, release, acceptance and documentation, with native `windows-11-arm` CI
proving memory, process cleanup, terminal, installation and update behavior before
support is documented.

The repository currently assumes x64 wherever it touches Windows: the release
catalog, the PowerShell bootstrap's machine check and PE machine constant, static
CRT flags, the MSVC `dumpbin` path, test fixtures that inject `AMD64`, the
published/staged verifiers' target constants, workflow literals and docs. Those
assumptions must be generalized once, keeping x64 green throughout, so that the
arm64 target is selected by the same authoritative catalog instead of a parallel
code path.

## What Changes

- Add `aarch64-pc-windows-msvc` (`kuru.exe`, ZIP) to the authoritative release
  target catalog, so the installer, updater, release assembly, staged and published
  verifiers select it through the existing `targets::CATALOG`/`host()` route.
- Make the Windows PowerShell bootstrap detect the native machine through its
  audited Add-Type bridge (`IsWow64Process2`) instead of `PROCESSOR_ARCHITECTURE`,
  select the target and PE machine constant per native machine, and parametrize
  every hard-coded x64 archive and support name.
- Generalize the source-build tasks and native acceptance scripts: static CRT
  flags for the arm64 target, native host tuple defaults, host-selected
  `verify-bundle-build.ps1`, and a `dumpbin` discovery that accepts the ARM64 MSVC
  host tools; add arm64 cross-check tasks in `kuru-platform`.
- Replace test fixtures that force `PROCESSOR_ARCHITECTURE=AMD64` or a fixed
  `x86_64-pc-windows-msvc` constant with values derived from the native machine of
  the test executable.
- Make the previous-release resolver target-scoped: when no published stable
  release carries the target's core archive, acceptance records
  "no predecessor for <target>" as visible evidence and passes; when any does, the
  greatest such release's updater must be exercised. Both branches are unit tested.
- Parametrize the staged and published Windows verifiers, their receipts and
  artifact names by target.
- Mirror every x64 Windows CI and Release job on `windows-11-arm` (platform
  primitives, coverage shards and collect, install/update and offline runtime,
  release build leg, staged acceptance, post-publication verification) with
  fail-closed aggregation, against the post-PR5 and post-PR4b workflow shapes.
  Workflow edits are deferred until the workflow owner's PR5 and PR4b merge
  notices; this change designs them and updates `release_workflow.rs` with them.
- Add `windows-arm64` platform entries to the tool lockfiles, using the x64 cospec
  asset under emulation first and a named, receipt-recorded
  `cospec_contract` exclusion only if mise cannot select or run it.
- Update `docs/install.md`, `docs/release.md`, `docs/development.md` and the
  native-windows and repository-delivery specs. The support row and any
  "supported" wording land only in the final commit after the five native
  checks (memory, process cleanup, terminal, installation, update) are green on
  `windows-11-arm`; staged and published arm64 acceptance are release-time
  gates observed on the first maintainer-authorized Release after merge.
- Require the ICU, LLVM runtime and mingw-w64 notices PR6a lists for built
  engine assets in the staged and published Windows verifiers.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `native-windows`: the native Windows application requirement, required native
  verification and staged release acceptance cover both `x86_64-pc-windows-msvc`
  and `aarch64-pc-windows-msvc`, with a Windows 11 floor for Arm.
- `repository-delivery`: the native Windows release artifact requirement covers
  every catalog Windows target; the published verifier and its receipt are
  target-parametrized; previous-release acceptance gains the generic target-scoped
  "no predecessor" rule.

## Impact

- `packages/kuru-delivery`: `src/targets.rs` (catalog entry), `src/published.rs`
  (target-scoped predecessor selection, from `previous-release-update-ci`),
  `src/published_windows.rs` (`WINDOWS_TARGET` becomes a parameter; receipt and
  artifact names carry the target), `src/mise_isolation.rs` (native-machine
  `PROCESSOR_ARCHITECTURE`), `src/coverage.rs` (host-keyed named test exclusion
  recorded in receipts, contingency only), `support/install.ps1`,
  `tests/release_workflow.rs`, `tests/support/mise_acceptance.rs` (target-scoped
  predecessor, catalog-derived asset ids, built-asset notices),
  `tests/support/previous_updater.rs`, `tests/bootstrap_windows.rs`,
  `tests/windows_update.rs`, `tests/windows_archive.rs`.
- `apps/kuru-tui`: `mise.toml` (native host default, arm64 crt-static),
  `support/install-source.ps1`, `support/verify-windows-imports.ps1`,
  `tests/embedded_runtime.rs`.
- `packages/kuru-memory`: `mise.toml` (arm64 crt-static clear/set),
  `support/verify-bundle-build.ps1`, `tests/bundle_build.rs`,
  `src/provision/native_tests.rs` (arm64 target). The manifest, both parsers and
  `bundle build` stay owned by `dolt-source-build-inputs`.
- `packages/kuru-platform`: `mise.toml` arm64 cross-check tasks,
  `tests/windows_commands.rs` fixture. No new unsafe API.
- `packages/kuru-connectors/src/tools.rs` fixture sites: reviewed, see design.
- `.github/workflows/ci.yml`, `native-tests.yml`, `release.yml`: designed here;
  edited only after the PR5 and PR4b merge notices (phase 3).
- Root `mise.toml` (cospec `windows-arm64` `asset_pattern`), `mise.lock`,
  `apps/kuru-docs/mise.lock`, `packages/kuru-delivery/mise.lock`:
  `windows-arm64` platform entries. `docs/development.md` lock-refresh commands.
- Docs: `docs/install.md`, `docs/release.md`, `docs/development.md`.
- No migration. The first arm64 release has no predecessor by design; existing
  x64 installations on Arm hardware keep updating x64, and bootstrap recovery
  accepts their x64 helper (migration is out of scope).
- Not BREAKING: no existing target, archive name, receipt field or workflow
  input is removed.

## Surfaces

- [x] interactive — PowerShell bootstrap machine detection and messages; `kuru update` target selection
- [x] deploy — `windows-11-arm` CI and Release jobs, artifact names, lockfile platforms
- [x] integration — GitHub release listing and `SHA256SUMS` cross-check in the predecessor rule; MSVC `dumpbin`/`vswhere`; mise github backend asset selection under emulation
- [ ] agent-behavior — no prompt, tool or model change
