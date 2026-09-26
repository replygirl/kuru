## Context

Current state, `main` at `501ab92d` unless noted:

- `packages/kuru-delivery/src/targets.rs` holds the five-entry `CATALOG` on
  `501ab92d`; #106 (`0ed39198`) reduces it to `[Target; 4]` by removing
  `x86_64-apple-darwin` and adds asserts that `macos`/`x86_64` is rejected. Every
  delivery path selects by `targets::host()` (`archive::host_target`), which maps
  `std::env::consts::{OS, ARCH}`, so an executable always selects the target it
  was compiled for. `release::archive_assets` (`release.rs:669`) requires one core
  archive and one shell-support envelope per catalog entry, and
  `published_windows::expected_assets` derives the published inventory from the
  same catalog. The catalog is therefore release-inventory load-bearing.
- The PowerShell bootstrap (`packages/kuru-delivery/support/install.ps1`) gates on
  `$env:PROCESSOR_ARCHITECTURE -ne 'AMD64'` (`:626`), defaults and allows only
  x64 (`:627-628`), requires PE machine `0x8664` (`:515`) and hard-codes the x64
  archive, support and helper names (`:641,693,711,723` plus the
  `PublishSupport(...,'x86_64-pc-windows-msvc',...)` call). Under an x64-emulated
  shell on Arm, `PROCESSOR_ARCHITECTURE` is `AMD64`, so the gate would install the
  wrong target. The bridge already P/Invokes kernel32 through `Add-Type` with
  `IntPtr`/`UIntPtr` layouts (`:67-99`).
- Source-build and acceptance scripts pin x64: `apps/kuru-tui/mise.toml:16,31`
  (`host` sentinel → x64), `:23` and `packages/kuru-memory/mise.toml:11,48`
  (crt-static via `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS` only),
  `apps/kuru-tui/support/install-source.ps1:23-24`,
  `packages/kuru-memory/support/verify-bundle-build.ps1:8-10`,
  `apps/kuru-tui/support/verify-windows-imports.ps1:14` (`VC.Tools.x86.x64`,
  `Hostx64/x64/dumpbin.exe`), `packages/kuru-platform/mise.toml:16,20,24`.
- Fixtures inject `PROCESSOR_ARCHITECTURE=AMD64` (`mise_isolation.rs:64`,
  `bootstrap_windows.rs:90`, `windows_update.rs:326,1039`,
  `windows_commands.rs:184`, `embedded_runtime.rs:1129,1198,1250`) or a fixed
  target constant (`mise_acceptance.rs:35`, `windows_archive.rs:15`,
  `bootstrap_windows.rs:30`, `published_windows.rs:20`).
- The staged mise fixture (`tests/support/mise_acceptance.rs:95-99`) numbers
  asset API ids as `assets/{index+1}` in `CATALOG` order and hard-codes
  `assets/5` for the x64 ZIP and `assets/6` for `SHA256SUMS` (`:99,153,170,843,
  854,864`). On the five-entry catalog an appended arm64 entry would take
  `assets/6` and collide with `SHA256SUMS`; after #106 x64 is `assets/4` and the
  hard-coded `assets/5` x64 assertions no longer name the x64 ZIP, so #106 as
  it stands probably breaks the `api_fallback` scenario (not run).
- On `origin/test/previous-release-update-ci` (#108) the previous-release
  resolver is `published::previous_release(candidate, target, token)`:
  `select_previous` picks the greatest older stable `vX.Y.Z` release ignoring the
  target, then `listed_digest` errors when that release lacks the target's
  archive. A first arm64 release would fail there. `tests/release_workflow.rs`
  forbids any `if:` in every checked release job except `build`, so this branch
  cannot be a workflow conditional.
- CI: seven x64 Windows job definitions (`ci.yml native-platform`;
  `native-tests.yml windows-coverage` ×4, 5 after #107, `windows-coverage-report`,
  `windows-install`; `release.yml build` Windows leg, `verify-staged-windows`,
  `verify-published-windows`). `native-tests.yml` keys its `coverage` `if` and
  `native-gate` on the literal `windows-2025`; artifact names carry no OS.
  `release_workflow.rs` asserts the literal workflow text. Release-time
  `native-tests` calls pass no `os`, so release validation runs Ubuntu only.
  At `501ab92d` no CI job runs #108's `test:previous-release-update`; the
  workflow owner's confirmed PR5 shape (next bullet) adds it to ordinary CI on
  every OS, including Windows.
- Confirmed post-PR5 and post-PR4b workflow shapes (workflow owner, 2026-09-26;
  exact text lands with the PR5 and PR4b merge notices). `release.yml`: the
  pre-bump `native-tests` call (`source-tests`) and the post-bump coverage rerun
  (`verify-tests`) are removed; a pre-bump `tests` job runs the ordinary
  `mise run test` on `ubuntu-latest`; post-bump `verify` (`quality.yml`) stays;
  `verify-staged` is a three-OS matrix (`windows-latest` keeps the mise-route
  `verify:staged-windows`; `ubuntu-latest` and `macos-latest` run an interim leg:
  `SHA256SUMS` verify, extract, `test:embedded-runtime` on the extracted binary,
  `test:previous-release-update` against it); `deploy-docs` and `publish` need
  `verify-staged`; `verify-published-windows` is unchanged. `native-tests.yml`:
  the Windows label is `windows-latest` (not `windows-2025`); PR4b removes the
  Unix monolithic coverage job, so every OS runs the five shards through a Rust
  `coverage:shard` task and one uniform collect job per OS; PR5 adds
  `test:previous-release-update` to a per-OS install/update job on all three
  OSes for `pull_request`, `merge_group` and main push (`GITHUB_TOKEN` from CI,
  network on, `KURU_UPDATE_CANDIDATE_BINARY` per #108's documented paths); and
  `native-gate` is rewritten around shard, collect and the per-OS install job.
- Tooling on `windows-11-arm` (design note §7; runner image README
  `actions/runner-images` `images/windows/Windows11-Arm64-Readme.md` at
  `d50c3090b1`, 2026-09-25, which supersedes the `actions/partner-runner-images`
  README last updated 2026-01-16 at image `20260105.41.1`): Rust, mise,
  mise-action, mr-boxington, hk, taplo, actionlint, cargo-llvm-cov, node and
  communique have native arm64 assets; cocogitto and shellcheck are x64-only and
  run emulated; `github:aligned-team/cospec` 0.7.1 has no arm64 asset (blocker
  B2). The `windows-11-arm` label is mid-migration (actions/runner-images#14602,
  rolling out 2026-09-21 to 2026-09-30) from image `20260920.174.1` (Visual
  Studio Enterprise 2022 17.14.37710.0) to image `20260920.164.1` (Visual Studio
  Enterprise 2026 18.10.12210.168 under `Microsoft Visual Studio\18\Enterprise`;
  pre-test label `windows-11-vs2026-arm`). Both images ship Rust 1.98.1, rustup
  1.29.1, PowerShell 7.6.6 and zstd 1.5.7, so `rust-cache` no longer falls back
  to gzip; public-repo Arm runners have 4 vCPU. Instrumented coverage on
  `aarch64-pc-windows-msvc` has an open upstream defect, rust-lang/rust#150123
  (D7; Open Question 7, decided by lead ruling 1 on 2026-09-26). Whether stock
  Windows PowerShell 5.1 is native ARM64 on the image is not itemized. Windows 11
  is the Arm floor because .NET Framework 4.8.1's native Arm64 support is
  Windows 11+ (confirmed by lead ruling 4 on 2026-09-26; x64 keeps Windows 10
  1809).
- Constraints: AGENTS.md forbids compile-only support claims, host fallbacks and
  unbundled builds; platform mechanics live in `kuru-platform`; `linux-x64` is the
  only engine build host (lead decision 4); PR6a owns the manifest, both parsers,
  `bundle build`, the Linux determinism job and the arm64 asset data; another
  session owns `.github/workflows` until its PR5 merge notice; #106 removes
  the Intel macOS target from the catalog, `install.sh`, `dolt-assets.json` and
  `bundle_build.rs`, leaving only the `mise.lock` `macos-x64` entries.

Lead decisions, quoted exactly from the design note's final section, with where
each is applied:

1. "**cospec on arm64**: first try the x64 cospec asset under Windows 11 Arm x64
   emulation (as cocogitto and shellcheck already run); pin the asset pattern per
   platform for the arm64 entry via mise's github backend. Only if mise genuinely
   cannot select or run it: drop cospec from the arm64 job installs and exclude
   `cospec_contract.rs` by name in the recorded test list, with the reason
   written down. Do not depend on an upstream arm64 cospec build (raised
   separately with the maintainer)." Applied in D7 (per-platform
   `asset_pattern` in root `mise.toml`, lock entry, contingency) and tasks
   2.9, 3.2, 3.3.
2. "**Previous-release updater check**: no release-keyed exemption. Make the rule
   generic and target-scoped in the previous-release resolver: when NO published
   release carries an asset for this target, record "no predecessor for
   <target>" as visible evidence and pass; if any predecessor exists for the
   target it must be exercised. Unit-test both branches." Applied in D5 and
   tasks 2.6, 2.7.
3. "**Notices**: ship ICU, LLVM runtime and mingw-w64 notices alongside Dolt's
   LICENSES for the built asset; the manifest/verifier requires them for built
   assets, not for upstream ones." Shipping them and the manifest requirement
   are PR6a's; requiring them in the staged and published verifiers this change
   owns is D6 and task 3.6.
4. "**Ownership/split**: PR6a (manifest schema v2, bundle build command and
   tasks, kuru-memory mise pins and lock, Linux build-twice determinism job,
   arm64 manifest entry as data) is owned by this session and must be
   self-contained and merged before PR6b starts. PR6b (target, windows-11-arm
   jobs, installer/updater/release/docs, the ~30 x64 assumptions) goes to
   another assistant with this note as the brief; this session reviews its
   workflow wiring. Linux-to-Linux determinism must be verified in CI before the
   arm64 entry is trusted; linux-x64 stays the only build host and the docs say
   so." Applied in D8 (PR6a owner reviews the wiring before task 3.4), D9
   (build-host sentence) and D10. This change departs from "merged before PR6b
   starts": phase 2 runs before PR6a merges, following the computed brief, and
   PR6a is a hard gate only for phase 3 and archive (blocking-changes Phase
   Gates). Open Question 3 asks the lead to confirm.
5. "**Sequence**: PR4a, PR5, PR4b before PR6a unless PR6a is already cheap to
   finish; CI speed and parity are the priority." Applied in D10 and the
   blocking-changes Phase Gates (PR5 notice gates phase 3).

## Goals / Non-Goals

**Goals:**

- One catalog entry drives every arm64 delivery path; no parallel arm64 code.
- Every generalization is built and tested on x64 and Unix before any arm64 job
  exists, and x64 stays green at every commit (see Decision 10).
- Native machine detection that cannot be fooled by an emulated shell.
- The "no predecessor" rule lives in Rust, is generic across targets, and both
  branches are proven without network access.
- Workflow wiring is fully specified here so the phase-3 edits are mechanical.
- Support is documented only after the five native checks pass on
  `windows-11-arm`.

**Non-Goals:**

- Building or hosting the arm64 Dolt engine (PR6a), manifest schema v2, shipping
  the ICU/LLVM/mingw-w64 notices or declaring them in the manifest (PR6a), or an
  upstream arm64 cospec build. Requiring those notices in the staged and
  published verifiers is in scope (D6).
- Migrating an emulated x64 installation on Arm hardware to arm64 (Open
  Questions).
- Removing `x86_64-apple-darwin` from the catalog (done by #106) or its
  remaining `mise.lock` `macos-x64` entries.
- Adding an arm64-only coverage shard, changing `KURU_COVERAGE_JOB_MINUTES`, or
  tuning runner time budgets before a measured first run.
- Windows 10 on Arm.

## Decisions

### 1. Target catalog entry and host selection

Add `Target { triple: "aarch64-pc-windows-msvc", os: "windows", arch: "aarch64",
executable: "kuru.exe", format: ArchiveFormat::Zip }` to `CATALOG` after the x64
Windows entry, changing #106's `CATALOG: [Target; 4]` to `[Target; 5]`, and
flip the test at `targets.rs:115` to assert `for_platform("windows", "aarch64")`
selects it with `kuru.exe`/`zip`, keeping #106's asserts that `macos`/`x86_64`
is rejected.

`host()` keeps mapping `std::env::consts::ARCH`. That constant is the compiled
target, which is the true native architecture for a native executable and is the
correct answer for self-update: an x64 `kuru.exe` running emulated on Arm keeps
updating x64. Rejected: querying `IsWow64Process2` from Rust so an emulated x64
binary could pick arm64. That would make the updater replace an executable with
one of a different machine type mid-flight and is the migration question, not
target selection. Confirmed by lead ruling 3 (kuru-implement-phase2-sep26a, 2026-09-26): an x64 `kuru.exe` under
emulation keeps self-updating to x64, and migration to arm64 is a separate
follow-on change (Open Question 1).

Because `release::archive_assets` and `expected_assets` iterate the catalog, the
entry, the test flip, the `release.yml` legs and the `release_workflow.rs`
expectations are one atomic phase-3 commit (Decision 10).

### 2. Installer native machine detection

In `install.ps1`, extend the existing `Kuru.Bootstrap.Native` bridge with
`[DllImport("kernel32.dll", SetLastError=true)] static extern bool
IsWow64Process2(IntPtr process, out ushort processMachine, out ushort
nativeMachine);` and a `public static ushort NativeMachine()` that calls it on
`GetCurrentProcess()` and throws on failure. `IsWow64Process2` exists since
Windows 10 1511, below the x64 floor of 1809, so a direct import is acceptable;
no `GetProcAddress` probing. Replace the `:626` gate with:

- `Win32NT` and `Is64BitProcess` as today;
- `nativeMachine` `0x8664` → `x86_64-pc-windows-msvc`; `0xAA64` →
  `aarch64-pc-windows-msvc`; anything else → throw naming the machine value.
- `PROCESSOR_ARCHITECTURE` and `PROCESSOR_ARCHITEW6432` are no longer consulted;
  both report the emulated view under WOW64/ARM64EC, which is the pitfall.
- `-Target` defaults to the native target. An explicit `-Target` that differs from
  the native machine throws before any download with a message naming both the
  requested and the native target ("target <requested> does not match the
  native machine target <native>"); a 32-bit
  process still fails the `Is64BitProcess` check. Rejected: allowing an explicit
  x64 install on Arm, which would silently ship the emulated executable users
  are trying to leave. Confirmed by lead ruling 2 (kuru-implement-phase2-sep26a, 2026-09-26): "Reject an explicit
  -Target that mismatches the native machine, with a message that names the
  native target." (Open Question 4)
- `Pe()` takes the expected machine (`0x8664` or `0xAA64`) instead of the
  constant; the error text names the expected architecture.
- The manifest pattern, support name, new helper cache name and the
  `PublishSupport` call use `$Target`.
- Recovery keeps the helper's own target, separate from the target being
  installed. `update.rs:837` names the recorded helper
  `{host_target()}-{sha}.exe` from the installed binary's compiled target, so an
  emulated x64 `kuru.exe` on Arm (which D1 keeps on x64) records an x64 helper.
  The receipt helper check (`install.ps1:644`) therefore derives the expected
  helper target from the recorded helper image's PE machine (`0x8664` or
  `0xAA64`), accepts either Windows catalog target, and requires the name to be
  `"<that target>-$sha.exe"`; it never compares it with the native `$Target`.
  Rejected: checking `"$Target-$sha.exe"`, which would leave an interrupted
  emulated-x64 update unrecoverable and unreinstallable on Arm.
- `tests/fixtures/install-v0.4.2.ps1` stays x64-only: it is the frozen historical
  bootstrap the update fixtures replay.
- Bootstrap tests (`bootstrap_windows.rs`) gain a case that runs the bootstrap
  with `PROCESSOR_ARCHITECTURE=AMD64` injected on every host and asserts the
  selected target equals the test executable's native target, proving the
  environment variable is ignored, and a recovery case with an x64-named helper
  receipt and x64 helper image that must recover on either runner. Injecting the
  variable into a native shell does not run `IsWow64Process2` from an emulated
  process. Step zero looks for an x64 launch path on `windows-11-arm` (x64
  `pwsh` or an equivalent x64 PowerShell host); if one exists, task 3.7 adds a
  real emulated-process bootstrap case, and otherwise the emulated-shell claim
  stays limited to what the tests prove: native detection through
  `IsWow64Process2` with the environment ignored.

Rust-side detection needs no change: the updater and `install-local` select by
the compiled target (Decision 1).

### 3. Static CRT, dumpbin discovery and fixtures forcing AMD64

- crt-static: add `CARGO_TARGET_AARCH64_PC_WINDOWS_MSVC_RUSTFLAGS =
  "-C target-feature=+crt-static"` beside every x64 setting
  (`apps/kuru-tui/mise.toml` `build:release`,
  `packages/kuru-memory/mise.toml` `bundle:verify-native-build`) and clear it
  beside the x64 clear in `bundle:prepare` (`packages/kuru-memory/mise.toml:11`).
  The spec's static-CRT requirement applies per target; the host build-script
  settings stay untouched because the variable is target-scoped.
- Native host default: `apps/kuru-tui/mise.toml` `run_windows` replaces the
  literal `x86_64-pc-windows-msvc` with the host tuple. mise templates cannot run
  `rustc`, so the task passes `--target host` through and the two PowerShell
  callers (`install-source.ps1`, the release `build` step) already resolve the
  triple explicitly; the `run_windows` default becomes a small
  `%KURU_HOST_TARGET%` resolved by a preceding `rustc --print host-tuple` line in
  the same `cmd` script. Rejected: keeping x64 as the default and requiring
  `--target` on Arm, which reintroduces a silent x64 assumption.
- `install-source.ps1:23-24` and `verify-bundle-build.ps1:8-10`: accept
  `x86_64-pc-windows-msvc` or `aarch64-pc-windows-msvc` from
  `rustc --print host-tuple`, select the manifest asset by that tuple, and keep
  rejecting anything else.
- `verify-windows-imports.ps1`: query `vswhere` with `-requires` either
  `Microsoft.VisualStudio.Component.VC.Tools.x86.x64` or
  `...VC.Tools.ARM64` (two queries, union), and find `dumpbin.exe` under
  `bin/Hostx64/x64` or `bin/Hostarm64/arm64` in that order of preference for
  the native host. `dumpbin /IMPORTS` reads any PE machine, so either host tool
  is acceptable for the check; the step-zero probe confirms which components the
  image ships before the arm64 path is trusted.
- `packages/kuru-platform/mise.toml`: add `setup:windows-arm64`,
  `typecheck:windows-arm64` and `lint:windows-arm64` mirroring the x64
  cross-check tasks (`rustup target add aarch64-pc-windows-msvc`).
- Fixtures: add `pub fn native_processor_architecture() -> &'static str` beside
  the production injection in `packages/kuru-delivery/src/mise_isolation.rs`,
  mapping `x86_64` → `AMD64` and `aarch64` → `ARM64` through
  `#[cfg(target_arch = ...)]` arms with a `compile_error!` arm for anything
  else, and use it at `mise_isolation.rs:64`,
  `bootstrap_windows.rs:90`, `windows_update.rs:326,1039`,
  `embedded_runtime.rs:1129,1198,1250`. `kuru-platform/tests/windows_commands.rs:184`
  cannot depend on `kuru-delivery`, so it uses the same two-line `consts::ARCH`
  match inline. Rejected: a new `kuru-platform` API calling `IsWow64Process2`; a
  test executable is native, so the compile-time arch is already the native
  machine and no unsafe surface is needed.
- Fixed target constants: `mise_acceptance.rs:35`, `windows_archive.rs:15` and
  `bootstrap_windows.rs:30` become `archive::host_target()` calls (or a
  `LazyLock`), asserted at startup to be a Windows catalog target.

### 4. Catalogue and gap disposition

Every row of the verified §8 catalogue and every confirmed gap:

| Id | Site | Disposition |
|---|---|---|
| A1 | `targets.rs:29-65` | add the entry (D1, phase 3) |
| A2 | `targets.rs:115` | flip the assertion (D1, phase 3) |
| A3 | `update.rs:761,837,1224` | no code change; selects by catalog (D1) |
| A4 | `archive.rs:65-66,256,275` | no code change; selects by catalog (D1) |
| A5 | `consts::ARCH` behavior | out of scope: migration is a follow-on change (Open Question 1, lead ruling 3); emulated x64 keeps x64 |
| A6 | `install.ps1:626` | `IsWow64Process2` native machine (D2) |
| A7 | 32-bit process | still rejected by `Is64BitProcess`; documented (D2) |
| A8 | `install.ps1:627-628` | default native target; explicit mismatch rejected (D2) |
| A9 | `install.ps1:515` | per-target PE machine (D2) |
| A10 | `install.ps1:641,693,711,723` + `PublishSupport` call | parametrize by `$Target` (D2) |
| A11 | `install.ps1:67-99` | no layout change; add one import (D2) |
| A12 | `tests/fixtures/install-v0.4.2.ps1` | out of scope: frozen historical fixture |
| A13 | `install-source.ps1:23-24` | accept both Windows tuples (D3) |
| A14 | `apps/kuru-tui/mise.toml:16,31` | native host tuple (D3) |
| A15 | `apps/kuru-tui/mise.toml:23` | add the arm64 variable (D3) |
| A16 | `packages/kuru-memory/mise.toml:11,48` | add the arm64 clear and set (D3) |
| A17 | `verify-bundle-build.ps1:8-10` | select by host tuple (D3) |
| A18 | `packages/kuru-memory/mise.toml:37` `bundle:test-fixtures` | out of scope, deliberately: it decodes the real upstream x64 ZIP as a host-independent decoder fixture on every host; recorded in `docs/development.md` |
| A19 | `verify-windows-imports.ps1:14` | either host dumpbin (D3) |
| A20 | `packages/kuru-platform/mise.toml:16,20,24` | add arm64 tasks (D3) |
| A21 | `mise_isolation.rs:64` | `native_processor_architecture()` (D3) |
| A22 | `bootstrap_windows.rs:90` | same helper (D3) |
| A23 | `windows_update.rs:326,1039` | same helper (D3) |
| A24 | `windows_commands.rs:184` | inline `consts::ARCH` match (D3) |
| A25 | `tools.rs:5456-5457` | out of scope: synthetic `C:\fixture\...` environment asserting passthrough of fixed values; nothing selects an asset from it; re-verify at implementation and record |
| A26 | `mise_acceptance.rs:35` | `host_target()` (D3) |
| A27 | `windows_archive.rs:15` | `host_target()` (D3) |
| A28 | `bootstrap_windows.rs:30` | `host_target()` (D3) |
| A29 | `kuru-memory/tests/bundle_build.rs:26-32` | add the arm64 target (the fifth after #106) once PR6a's data exists (phase 3, task 3.1) |
| A30 | `provision/native_tests.rs:225` | add the arm64 target, same timing as A29 |
| A31 | `published_windows.rs:20,365,380` | `WINDOWS_TARGET` becomes an `Options.target` validated against `host_target()` (D6) |
| A32 | `mise_acceptance.rs:917-921` | target-scoped predecessor rule (D5) |
| A33 | `openspec/specs/native-windows/spec.md:15,200` | delta in this change |
| A34 | `openspec/specs/repository-delivery/spec.md:127` | delta in this change |
| A35 | `docs/install.md:135-150,154` | phase 3, final commit (D9) |
| A36 | `docs/release.md:151,180` | phase 3 (D9) |
| A37 | `docs/development.md:217-320` (bundled-engine build inputs, offline import) | arm64 offline import example using PR6a's built archive and the `bundle:test-fixtures` note (A18); phase 3 (D9) |
| A38 | `docs/development.md:439-453` (lock refresh commands) | add `windows-arm64` to the `mise lock --platform` lists and describe the cospec `asset_pattern` pin (D7, task 2.9) |
| A39 | `tests/support/mise_acceptance.rs:95-99,153,170,843,854,864` | derive each asset API id from `CATALOG.iter().position()`, give `SHA256SUMS` an id past the catalog, make the request assertions use those ids and assert the ids are unique (task 2.10, phase 2) |
| G1 | `embedded_runtime.rs:1129,1198,1250` | same helper as A21 (D3) |
| G2 | `windows_cli.rs:34-35,82,163,195-196` | out of scope: passes the real machine environment through and asserts it is non-empty; correct on any host |
| G3 | `tools.rs:116-117,5966-5967` | `:116-117` is the passthrough allowlist, benign; `:5966-5967` is the same synthetic fixture class as A25, same disposition |
| G4 | tool locks with `windows-x64` entries | add `windows-arm64` entries (D7) |
| G5 | #106 removes the Intel catalog entry (`0ed39198`) | rebase onto #106 (D10); phase-3 catalog edit is `[Target; 4]` → `[Target; 5]` keeping #106's asserts; #106's untouched `mise.lock` `macos-x64` entries are out of scope |
| G6 | resolver moved to `published.rs` (#108) | this change targets `published.rs` (D5) |
| — | `openspec/specs/embedded-runtime/spec.md:26-38` | out of scope: PR6a's built-source provenance delta |

### 5. Previous-release rule in `published.rs`

Split the target-blind `select_previous` into pure, target-scoped selection over
an injected manifest provider plus the async fetch that feeds it:

```text
pub enum Predecessor {
    Release(PreviousRelease),
    None { target: &'static str, candidate: String, inspected: Vec<String>, horizon: String },
}
fn select_previous(
    releases: &[PublishedRelease], candidate: Version, target: &str,
    manifest: &mut dyn FnMut(&PublishedRelease) -> Result<Vec<u8>>,
) -> Result<Selection>   // Selection::Release(version) | Selection::None { inspected }
```

`previous_release` passes a provider that fetches the release's `SHA256SUMS`
from its immutable version path (64 KiB bound) and authenticates it against its
listed digest with #108's `listed_digest(..., "SHA256SUMS")` plus
`verify_manifest` before returning the bytes. Unit tests pass an in-memory
provider, so no test needs the network.

Rules, in order:

1. Keep the existing fail-closed checks: every stable release tag must be
   canonical `vX.Y.Z`; a stable release newer than the candidate is an error; the
   candidate itself is skipped.
2. Walk the remaining stable releases in descending version order. The first
   whose asset listing names `archive::archive_name(version, target)` is the
   predecessor; `previous_release` then downloads and authenticates it exactly as
   today and it must be exercised.
3. A release whose listing lacks the name is not yet "no predecessor": obtain its
   authenticated `SHA256SUMS` from the provider and require, through #108's
   `publishes_asset(listed, manifest, name)`, that listing and manifest agree
   the archive is absent. Disagreement in either direction is an error
   ("previous release vX lists/omits <name> inconsistently"). Rejected:
   trusting the listing alone, and adding a second agreement helper.
4. If no stable older release carries the archive, return
   `Predecessor::None { inspected }` with every inspected version. If there is no
   stable older release at all, keep today's error: the first release of the
   project is a different situation from the first release for a target.
5. Horizon: the release listing request is unpaginated and GitHub returns at
   most 30 releases per page. The `None` branch follows the `Link: rel="next"`
   pages (each bounded by the existing 4 MiB metadata limit) until exhausted, so
   "no predecessor" covers every published release, as lead decision 2 requires
   ("NO published release"). If pagination cannot be completed, the `None`
   branch fails rather than passing on a partial listing. The evidence line names
   the horizon (`across N releases, all pages`).

`previous_release` returns `Result<Predecessor>`. Callers:

- `tests/support/previous_updater.rs::previous_updater_accepts_candidate` takes
  `&Predecessor`; on `None` it prints exactly
  `no predecessor for <target>: inspected v0.9.0, v0.8.0 ... across N releases, all pages`
  and returns `Ok` without running any updater. On `Release` it runs unchanged.
- `mise_acceptance.rs::run_staged` and `tests/previous_release_update.rs` pass
  the host target and print the same line, so the Windows `verify-staged` leg's log
  (`verify-staged-windows` before PR5), PR5's per-OS install/update job log
  and the CI acceptance log carry the evidence. No new receipt file: the staged
  check has none today and the published verifier does not run the previous
  updater. The evidence surface is confirmed by lead ruling 6 (kuru-implement-phase2-sep26a, 2026-09-26)
  (Open Question 2): a job-log line plus the two unit-tested branches
  (`Release` and `None`), with no receipt field; "no stable older release at
  all" stays a distinct error (rule 4).
- The rule cannot be a workflow `if:` (`release_workflow.rs` forbids it in
  `verify-staged-windows`; the post-PR5 `verify-staged` assertions are pending
  the PR5 notice), which is why it lives here.

Unit tests (pure, fixture listings and in-memory manifest provider, no network):

- `predecessor_is_the_greatest_older_release_carrying_the_target` — Windows x64
  candidate with v0.10.0 (asset present), v0.9.0 (present): selects v0.10.0.
- `predecessor_skips_releases_without_the_target_when_a_later_one_exists` —
  v0.10.0 lacks arm64 (listing and manifest agree), v0.9.0 has it: selects
  v0.9.0 and reports v0.10.0 inspected.
- `no_predecessor_when_no_stable_release_carries_the_target` — arm64 candidate
  over a listing of x64-only releases: `Selection::None` naming every inspected
  version; draft and prerelease entries are not inspected.
- `no_predecessor_requires_the_manifest_to_agree` — the provider's manifest
  names the archive the listing omitted, and the listing names it but the
  manifest does not: both fail.
- `no_predecessor_follows_every_listing_page` — a two-page listing where only
  the second page carries the target selects it; an incomplete page chain fails.
- Existing tests keep passing: newer-than-candidate error, malformed tags, the
  project-wide "no stable release precedes" error, synthetic next-patch selection,
  and #108's manifest authentication tests.

All 16 published releases (v0.1.0 through v0.9.0) carry `SHA256SUMS`, so the
cross-check is always possible.

`docs/release.md` gains the sentence: the check inspects every published stable
release for the target's archive; on a target's first release it records
"no predecessor for <target>" with the inspected versions and passes, and that
line is the acceptance evidence to look for in the job log.

### 6. Staged and published verifiers parametrized by target

- `published_windows::Options` gains `target: String`; `run` validates it is a
  Windows catalog target equal to `archive::host_target()` and uses it for the
  archive name, the isolated mise install and the PE machine check on the
  installed executable (`0x8664`/`0xAA64`). The receipt gains `target` and keeps
  `runner_arch`. The task/script take `KURU_PUBLISHED_TARGET` (default host
  target) and the receipt path
  `published-windows-<target>-receipt.json`; the artifact name becomes
  `published-windows-<target>-<version>-<attempt>`. In phase 2 the verifier
  keeps writing to the path in `KURU_PUBLISHED_WINDOWS_RECEIPT` that
  `release.yml verify-published-windows` sets; the target-qualified receipt
  and artifact names are workflow values that land in phase 3 (task 3.4).
- Built-asset notices (lead decision 3): once PR6a's v2 schema exists, the
  staged (`mise_acceptance.rs:653`) and published
  (`published_windows.rs:1098-1111`) verifiers read the target's manifest
  source kind; for a built asset they require every ICU, LLVM runtime and
  mingw-w64 notice the manifest lists, each digest-equal, alongside the
  `LICENSES` check they already make, and the arm64 published receipt records
  the verified notice names and digests. Upstream assets keep today's
  `LICENSES`-only check (task 3.6).
- `apps/kuru-tui:verify:staged-windows` keeps its single test; `mise_acceptance`
  derives `TARGET` from the host (D3) and the workflow passes
  `KURU_STAGED_WINDOWS_ARCHIVE` with `${{ matrix.target }}` (D8).

### 7. Tooling on `windows-11-arm` and the cospec contingency

| Tool | On arm64 | Action |
|---|---|---|
| rust 1.98.1, llvm-tools-preview | native | none |
| cargo-llvm-cov 0.9.1 | built from source for arm64 through the `cargo:` backend; installation works, but instrumented collection has an open upstream defect: rust-lang/rust#150123 (open, label `O-aarch64-pc-windows-msvc`, filed 2025-12-18 against nightly 1.94.0 `f794a0873`, last updated 2025-12-19, no comments) reports `llvm-profdata merge` failing with "malformed instrumentation profile data: symbol name is empty"; not yet checked on the pinned 1.98.1 | step zero runs an instrumented smoke test including the issue's `llvm-profdata merge -sparse` step and records its stderr as the pinned toolchain's state; per lead ruling 1 (Open Question 7, decided 2026-09-26) arm64 runs the same shard suites uninstrumented as separately named behavioral evidence, never counted toward the 90% gate, and instrumented arm64 shards are held until a pinned toolchain carries the upstream fix |
| mise 2026.9.4, mise-action v4.3.0 | native | none |
| mr-boxington 1.17.0 | native asset; CI sets `KURU_MBX=0` | lock entry for maintainers |
| hk 1.58.1 | native; CI sets `MISE_NO_HOOKS=1` | lock entry |
| taplo, actionlint, node/npm, communique | native | lock entries |
| cocogitto 7.0.0 | x64 asset, `windows_arm_emulation: true` in the registry | lock entry to the x64 asset; smoke-test in step zero |
| shellcheck 0.11.0 | x64 only; not installed on Windows jobs | lock entry to the x64 asset for `MISE_LOCKED=1` completeness |
| cospec 0.7.1 | no arm64 asset (B2); release assets are `linux-{x64,arm64}[-musl]`, `macos-{x64,arm64}`, `windows-x64.zip`, `SHA256SUMS` | lead decision 1: root `mise.toml` moves the tool to table form with `[tools."github:aligned-team/cospec".platforms] windows-arm64 = { asset_pattern = "cospec-*-windows-x64.zip" }`, and the lock entry is generated from it; step zero runs `cospec --version` under emulation and records it |

Lockfiles: root `mise.lock` (hk, shellcheck, actionlint, taplo, cospec,
mr-boxington), `packages/kuru-delivery/mise.lock` (cocogitto, communique) and
`apps/kuru-docs/mise.lock` (node) gain `windows-arm64` platform entries. Read
at main `501ab92d`, none of the three lockfiles has any `platforms.windows-arm64`
block; each of those tools has only `windows-x64` and `windows-x64-baseline`
blocks, although mr-boxington, hk, actionlint, taplo, communique and node publish
native arm64 assets upstream. The entries are refreshed
through the `mise lock --platform ...,windows-arm64` commands documented in
`docs/development.md`. A hand-written lock entry would be regenerated away by the
next `mise lock`, so the cospec selection lives in `mise.toml` and the lock only
records what mise resolved from it. The `platforms.<platform>.asset_pattern`
syntax is confirmed against the pinned release's own documentation
(`docs/dev-tools/backends/github.md` at jdx/mise tag `v2026.9.4`, read
2026-09-26): `asset_pattern` replaces asset autodetection entirely for that
platform key, whereas `matching` and `matching_regex` only narrow the candidate
set while keeping autodetection and are silently ignored when `asset_pattern` is
set. The arm64 pin is therefore a hard override with no autodetected fallback:
if the x64 asset is renamed upstream, resolution fails rather than choosing
another asset. The documented examples use `linux-x64` and `macos-arm64` keys, so
the `windows-arm64` key itself is still confirmed at the first `mise lock`. `aqua:cocogitto/cocogitto` needs no
option: the aqua backend has no `asset_pattern`, and the aqua registry entry's
`windows_arm_emulation: true` makes aqua select the x64 asset on
`windows-arm64`; task 2.9 records the resolved asset from the generated lock as
that check.

Contingency, only if step zero shows mise cannot select or run the x64 cospec
asset on arm64: drop `github:aligned-team/cospec` from the arm64 jobs'
`install_args` and add a host-keyed exclusion table to
`packages/kuru-delivery/src/coverage.rs`:

```text
pub const EXCLUDED_ARTIFACTS: [(&str, &str, &str); 1] = [(
    "aarch64-pc-windows-msvc", "kuru-delivery/cospec_contract",
    "cospec 0.7.1 publishes no windows-arm64 asset and the x64 asset does not run under emulation; the contract still runs on x64 and Ubuntu",
)];
```

`write_selection` validates each exclusion against the inventory (the artifact
must exist, or the exclusion is stale and fails), `runnable_artifacts` omits it
for the matching host only, and the shard receipt records `excluded_artifacts`
with the reason so `coverage:windows:collect` and the report show it. Rejected:
removing the test from the shard's package list (not a named exclusion, and the
package must stay in `SHARDS`, which is test-bound to the workflow matrix) and
`#[cfg_attr(target_arch = "aarch64", ignore)]` (a silent skip with no receipt).

Lead ruling 5 (kuru-implement-phase2-sep26a, 2026-09-26) accepts this mechanism on one condition: the
exclusion applies only while the arm64 job actually has no runnable cospec after
the emulated x64 asset (the `asset_pattern` pin above, lead decision 1) has been
tried first. If cospec runs there, nothing is excluded and `EXCLUDED_ARTIFACTS`
carries no arm64 entry. Every arm64 shard receipt records which case applied:
the `cospec_contract` exclusion with its reason, or that cospec ran under
emulation and nothing was excluded.

### 8. CI job mirror for `windows-11-arm` (design only; edits in phase 3)

The PR6a owner reviews this wiring before task 3.4 edits any workflow (lead
decision 4). Every new `uses:` step, including the `actions/download-artifact`
import step, is pinned by commit SHA.

The mirror is designed against the workflow owner's confirmed post-PR5 and
post-PR4b shapes (Current state), not the `501ab92d` files: the x64 twin runs on
`windows-latest` and the arm64 leg on `windows-11-arm`; `native-tests.yml` is the
uniform five-shard `coverage:shard` job plus one collect job per OS and PR5's
per-OS install/update job; `release.yml` has `verify-staged` as a three-OS
matrix. The job ids, step names, gate script and `release_workflow.rs`
expectations quoted below describe that shape and are **pending the PR5 and
PR4b merge notices**; task 3.4 rebases onto the merged files and takes their
exact text from them, without changing the wiring decisions here.

Step zero, a throwaway job on `windows-11-arm` in the first phase-3 commit,
removed before merge: print `[Environment]::Is64BitProcess`,
`$env:PROCESSOR_ARCHITECTURE`, `PROCESSOR_ARCHITEW6432` and the `IsWow64Process2`
native machine under `powershell.exe` 5.1 and `pwsh`; `$PSVersionTable`;
`vswhere` results for `VC.Tools.ARM64` and `VC.Tools.x86.x64` with the located
`dumpbin.exe` paths; `rustc --print host-tuple` after mise; `cospec --version`
and `cog --version` under emulation; whether an x64 PowerShell host (for
example x64 `pwsh`) can be launched, for the emulated-process bootstrap case in D2; and a
`cargo llvm-cov` instrumented smoke test recording whether the pinned toolchain
carries the rust-lang/rust#150123 fix (per lead ruling 1 the arm64 suites run
uninstrumented until one does). It confirms stock PowerShell 5.1 is native
ARM64 before the bootstrap is trusted on that assumption.

| x64 job | arm64 twin | Parameters | Aggregation |
|---|---|---|---|
| `ci.yml native-platform` | matrix leg `{os: windows-11-arm, target: aarch64-pc-windows-msvc}` of the same job beside the x64 `windows-latest` leg, name `Native platform primitives (${{ matrix.target }})` | `runs-on: ${{ matrix.os }}`, `CARGO_BUILD_TARGET: ${{ matrix.target }}`, rust-cache `shared-key: native-platform-${{ matrix.target }}`, artifact `coverage-native-platform-${{ matrix.target }}` (the current name would collide) | none; `ci-gate` sees the job result |
| `ci.yml native-tests` matrix | add `windows-11-arm` beside `windows-latest` in `os` | — | `native-gate` below |
| post-PR4b shard job (`coverage:shard`, five shards, uniform on every OS) | `windows-11-arm` leg of the same job | `runs-on: ${{ inputs.os }}`; artifacts os-qualified as `<prefix>-coverage-<inputs.os>-<shard>-attempt-<n>` and `...-diagnostics-...` (whatever os qualification PR4b's uniform names already carry is kept, not duplicated); rust-cache key per os; the PR6a import step (`download-artifact bundle-input-aarch64-pc-windows-msvc` then `bundle:prepare --target aarch64-pc-windows-msvc --archive <file> --offline` into the job's private `KURU_DOLT_BUNDLE_DIR`) conditioned on the os before `coverage:shard`; `install_args` per D7; matrix stays byte-equal to `coverage::SHARDS` (five); `KURU_COVERAGE_JOB_MINUTES` stays equal to the timeout | `native-gate` |
| post-PR4b per-OS collect job | `windows-11-arm` leg of the same job | `runs-on: ${{ inputs.os }}`; download patterns `<prefix>-coverage-<inputs.os>-<shard>-attempt-*` so the two Windows OS calls never cross-contaminate; os-qualified upload; the collector requires every receipt to share one host triple | `native-gate` |
| PR5 per-OS install/update job (today's `windows-install` plus `test:previous-release-update`) | `windows-11-arm` leg of the same job | `runs-on: ${{ inputs.os }}`; rust-cache key per os; the same PR6a import step; tasks generalized per D3; `test:previous-release-update` with `KURU_UPDATE_CANDIDATE_BINARY` at #108's documented `.exe` path, `GITHUB_TOKEN` from CI and network on exactly as PR5 sets them for `windows-latest`; until a release carries `aarch64-pc-windows-msvc` the leg passes on D5's `no predecessor for aarch64-pc-windows-msvc: ...` branch | `native-gate` |
| `native-gate` (rewritten by PR5 around shard, collect and install) | same job | the rewritten gate recognizes `windows-11-arm` alongside `windows-latest` for its shard, collect and install checks, keeps the Unix labels on the uniform path, and fails any other Windows label with the label in the message (fail-closed allowlist); `release_workflow.rs`'s gate test (today `native_workflow_gate_rejects_incomplete_windows_results`, which extracts the gate script from the live `native-tests.yml`) gains the `windows-11-arm` accept case, each arm64 reject case and an unknown-label reject case in the same commit, with the `windows-latest` cases unchanged; exact script and test names pending the PR5 notice | itself |
| `release.yml build` Windows leg | add `{os: windows-11-arm, target: aarch64-pc-windows-msvc}` | already keyed on `matrix.target`/`runner.os`; `needs` gains PR6a's release-scoped `dolt-windows-arm64` job built from `needs.bump.outputs.sha`, and the asserted `needs` literal in `release_workflow.rs` (post-PR5, without `verify-tests`) changes with it; the artifact download and offline import steps carry `if: matrix.target == 'aarch64-pc-windows-msvc'` (`build` is the one job allowed step conditions) | `assemble-candidate` validates against the catalog, so D1 lands in the same commit |
| `verify-staged` (PR5 three-OS matrix) | fourth leg `{windows-11-arm, aarch64-pc-windows-msvc}` mirroring the `windows-latest` mise-route leg (`verify:staged-windows`), not the interim Unix leg | `runs-on: ${{ matrix.os }}`; `KURU_STAGED_WINDOWS_ARCHIVE: .../kuru-<version>-${{ matrix.target }}.zip`; the predecessor rule runs inside the task (D5); the Windows leg's step-condition rules follow whatever PR5 asserts for the matrix | `deploy-docs` and `publish` need `verify-staged`, so a failing arm64 leg blocks both; `release_workflow.rs` expectations move with the matrix |
| `verify-published-windows` (unchanged by PR5) | arm64 twin: matrix over `{windows-latest, x86_64-pc-windows-msvc}` and `{windows-11-arm, aarch64-pc-windows-msvc}` | `runs-on`, `KURU_PUBLISHED_TARGET`, receipt `published-windows-<target>-receipt.json`, artifact `published-windows-<target>-<version>-<attempt>`; stays the file's final step (asserted) | none; post-publication |

Caveats carried into the tasks: after PR5, `release.yml` makes no
`native-tests` call; its only pre-bump test job is `tests` (`mise run test` on
`ubuntu-latest`), so release time runs no Windows coverage for either
architecture. Release-time Windows evidence stays staged and published
acceptance (the former Open Question 6, settled by PR5's shape), while Unix
gains the interim staged leg; the docs say so. Public-repo Arm runners have 4 vCPU; the 90-minute shard
budget is measured on the first run before any change, and a fifth-or-more
shard needs a `SHARDS` code change first. `windows-11-arm` is GA for public
repositories and the repository is public; minutes are billed at the Arm rate
only for private repositories.

Per lead ruling 1 (Open Question 7), until a pinned toolchain carries the
rust-lang/rust#150123 fix, the arm64 legs of `native-platform` and the shard
job run the same test suites uninstrumented; the instrumented arm64 shards and
the arm64 `native-platform` coverage upload are held. Those runs are named
separately in their receipts as arm64 behavioral evidence and never contribute
to or satisfy the 90% gate, which the instrumented x64 and Unix runs keep
enforcing.

### 9. Documentation and the support-claim rule

The support claim rests on the five native checks AGENTS.md names, all of which
run in ordinary CI on the branch before merge: memory, process cleanup, terminal,
installation and update (D11). Update is covered natively by the per-OS
install/update job (today's `windows-install`), whose PR5
`test:previous-release-update` runs in ordinary CI and whose `test:embedded-runtime` runs
`embedded_runtime.rs::packaged_install_and_update_preserve_complete_offline_memory`.
Staged acceptance (the Windows legs of `verify-staged`) and post-publication
verification (`verify-published-windows`) exist only in `release.yml`, which runs from main
after merge; they are release-time gates, not pre-merge evidence. Staged
acceptance already blocks `deploy-docs` and public promotion in the same run, so
a failing arm64 leg keeps the release, and the docs that claim support, from
publishing.

- `docs/install.md`: the supported-platforms row `Windows 11 or newer | ARM64 |
  aarch64-pc-windows-msvc`, the bootstrap text (native machine detection, both
  targets, explicit `-Target` must match; an emulated x64 installation keeps
  updating x64), and the archive-naming sentence. The row and every sentence
  that says Kuru "supports" or "ships" Windows on Arm land only in the final
  phase-3 commit, after the `windows-11-arm` platform, coverage shards and
  report, and install and offline runtime (including install-and-update) jobs
  are green natively on the branch. Until then the docs describe the target as
  "in verification" or do not mention it.
- `docs/release.md`: both Windows legs in the build list, the staged and
  published matrix, the target-parametrized receipt name, the predecessor
  sentence from D5, the note that release-time Windows evidence is staged and
  published acceptance only (release-time tests are Ubuntu `mise run test`),
  and that staged acceptance gates docs deployment and promotion per target.
- `docs/development.md` (A37, A38): the `windows-arm64` lock platform and the
  cospec `asset_pattern` pin in the refresh commands, an arm64 offline import
  example using PR6a's built archive, the `bundle:test-fixtures` note (A18),
  the `dumpbin` host-tools sentence, and (lead decision 4) that `linux-x64` is
  the only engine build host.
- Notices for built assets are shipped and documented by PR6a; this change
  links to that section and documents only that the Windows verifiers require
  them (D6).
- Verification rows 5.1 (staged arm64 leg) and 7.1 (published arm64 receipt)
  need a maintainer-authorized Release run after merge. At archive they are
  recorded as explicit post-merge deferrals naming the first release that will
  carry the arm64 target, and are observed and recorded on that run.

### 10. Sequencing and the x64-stays-green guarantee

- Phase 1 (this change): design, specs, this ledger. No product code.
- Phase 2 (on `feat/windows-arm64`, x64 and Unix only): D2 (installer), D3
  (tasks, scripts, fixtures), A39 (staged fixture asset ids), D5 (resolver;
  requires #108 merged), D6 (verifiers, without the built-asset notices), and D7
  lock entries. The `release_workflow.rs` gate cases are not in phase 2: the
  test reads the gate script from the live workflow, so they move with the
  workflow edit into task 3.4. None of these touch the catalog or workflows,
  so `assemble-candidate`, `expected_assets` and every x64 job are unchanged in
  behavior; x64 CI proves the generalizations on x64. A29/A30 wait
  for PR6a's manifest data.
- Phase 3 (after the PR5 and PR4b merge notices and PR6a's archive, which is a hard gate
  here even though `cospec apply` sees it as soft): step zero, then the PR6a
  owner's review of D8, then D1 catalog entry + D8 workflow edits +
  `release_workflow.rs` text expectations and gate cases in one commit, then the
  built-asset notice checks (D6), then D9 docs in the final commit after the
  five native checks are green on the branch. Staged and published arm64
  acceptance follow on the first maintainer-authorized Release run after merge.
- Rebase: `feat/windows-arm64` is created from `origin/main` at `501ab92d`;
  before phase 2 it rebases onto main once #106 (which removes Intel macOS and
  leaves `CATALOG: [Target; 4]`), #107 and #108 merge. PR6a's merge triggers the
  phase-3 rebase.
- "Phase 1/2/3" names stages inside this change (design and ledger; x64
  generalizations; arm64 enablement), not roadmap phases.

### 11. Acceptance mapped to the five native checks

| Native check (AGENTS.md) | Evidence on `windows-11-arm` |
|---|---|
| memory | `coverage:shard` shards `memory` and `runtime` (post-PR4b five-shard matrix, uninstrumented per lead ruling 1) pass with the PR6a engine imported offline; `test:embedded-runtime` cold offline conversation |
| process cleanup | `native-platform` arm64 leg (uninstrumented per lead ruling 1; the 90% platform gate stays with the instrumented x64 and Unix runs) and the `connectors-core-platform` shard's owned-process and supervisor tests |
| terminal | the `application` shard's ConPTY tests (uninstrumented per lead ruling 1) |
| installation | the per-OS install/update job: `mise run install` (source), `bundle:verify-native-build`, `verify:windows-imports`, plus the bootstrap tests in the `delivery-archive` shard |
| update | the per-OS install/update job's `test:embedded-runtime` (`packaged_install_and_update_preserve_complete_offline_memory`) and PR5's `test:previous-release-update` in ordinary CI on the branch (the `no predecessor` branch until a release carries the target) |
| staged acceptance (release-time, after merge) | `verify-staged` `{windows-11-arm, aarch64-pc-windows-msvc}` green on the first maintainer-authorized Release run, with the predecessor decision in its log; gates `deploy-docs` and promotion |
| post-publication (release-time, after merge) | `verify-published-windows` arm64 receipt retained on that run |

## Operational surface

Every arm64 delivery path runs on the same hosts and with the same secrets as the
x64 one, whose label is `windows-latest` after PR5 (`windows-2025` before it).
The arm64 jobs mirror the post-PR4b shard and per-OS collect jobs, PR5's per-OS
install/update job (which runs `test:previous-release-update` on Windows in
ordinary CI), the `release.yml` build leg, the `windows-11-arm` leg of the
three-OS `verify-staged` matrix and the arm64 twin of the unchanged
`verify-published-windows`; exact job text is pending the PR5 and PR4b merge
notices. `windows-11-arm` GitHub-hosted runners (image `20260920.174.1` with
Visual Studio 2022 or, after the actions/runner-images#14602 migration completes
by 2026-09-30, `20260920.164.1` with Visual Studio 2026; 4 vCPU in public
repositories; zstd 1.5.7 installed) run the native jobs; `ubuntu-24.04` runs PR6a's
engine build and hands the archive over as a workflow artifact that each arm64
job imports offline into its own runner-created private `KURU_DOLT_BUNDLE_DIR`.
No job binds a network listener beyond the existing loopback mise fixture in
staged acceptance; no new secret is introduced, and the only token used is
`github.token` for mise-action tool downloads and, for
`test:previous-release-update` (PR5's install/update job and the staged
legs), `GITHUB_TOKEN` from CI for the GitHub REST metadata request. Asset
downloads stay anonymous over HTTPS. Binary versions and architectures: Rust
1.98.1 `aarch64-pc-windows-msvc`, mise 2026.9.4 native arm64, cargo-llvm-cov
0.9.1 built from source (instrumented collection subject to rust-lang/rust#150123), cocogitto 7.0.0 and cospec 0.7.1 x64 assets under
emulation, communique 1.3.5 and hk 1.58.1 native arm64, Dolt v2.3.3 built by
PR6a for windows/arm64. Connection limits and deadlines are unchanged: the
resolver's 60-second HTTPS client, 64 KiB `SHA256SUMS` bound, 4 MiB metadata
bound and the existing archive limits. Installed applications never invoke MSVC
tools; `dumpbin` is a build-time acceptance dependency only.

## Integration contract

- GitHub Releases: the predecessor rule reads the release listing
  (`/repos/replygirl/kuru/releases`, 30 per page, following `Link: rel="next"`
  on the `None` branch) and, for a release whose listing lacks the target's
  archive, its `SHA256SUMS` from the immutable `releases/download/v<version>/`
  path, authenticated against the listed digest. Asset names are owned by
  `archive::archive_name`/`shell_support::archive_name` from the catalog; no new
  naming scheme. Listing and manifest must agree or the release is rejected.
- Windows API: `IsWow64Process2` (kernel32, Windows 10 1511+) through the
  existing audited `Add-Type` bridge only; the Rust side keeps `consts::ARCH`.
  Machine ids: `0x8664` x64, `0xAA64` ARM64.
- MSVC tools: `vswhere` component ids `Microsoft.VisualStudio.Component.VC.Tools.x86.x64`
  and `Microsoft.VisualStudio.Component.VC.Tools.ARM64`; `dumpbin.exe` under
  `bin/Hostx64/x64` or `bin/Hostarm64/arm64`. The check reads imports only.
- mise: root `mise.toml` pins cospec's `windows-arm64` `asset_pattern` to the
  x64 ZIP; the github and aqua backends' lock entries keyed `windows-arm64` (key
  confirmed at the first `mise lock`) are generated from configuration; cocogitto
  resolves its x64 asset through the aqua registry's `windows_arm_emulation`.
  `MISE_LOCKED=1` stays on.
- PR6a: the arm64 manifest asset is selected by Cargo `TARGET` exactly as the
  upstream ones; this change reads `target`, `stem`, `format`,
  `executable_name` and, for the verifier notice check only, the built-asset
  notice list PR6a's v2 schema defines under `source.built.*`. The workflow artifact is
  named `bundle-input-aarch64-pc-windows-msvc` and imported with
  `bundle:prepare --target aarch64-pc-windows-msvc --archive <file> --offline`.
- Coverage receipts: shard receipts may carry `excluded_artifacts`
  (`[{artifact, reason}]`) only when the contingency in D7 is active; the
  collector reconciles it against the inventory and the report shows it. Per
  lead ruling 5, every arm64 shard receipt records which D7 case applied
  (exclusion with reason, or cospec ran and nothing was excluded). Per lead
  ruling 1, arm64 shard receipts identify uninstrumented behavioral runs that
  never count toward the 90% gate.

## Risks / Trade-offs

- [Stock PowerShell 5.1 on the arm image is not native ARM64 or lacks .NET
  Framework 4.8.1] → step zero prints the process and native machine under both
  shells before any bootstrap test runs; the Windows 11 floor is documented with
  the .NET 4.8.1 reason.
- [The x64 cospec asset fails under emulation] → the named, receipt-recorded
  exclusion mechanism in D7 is designed now so the fallback is a small diff, not
  a redesign; the reason is written in the receipt and the docs. Per lead
  ruling 5 it applies only while cospec cannot run after the emulated x64 asset
  is tried first.
- [Emulated x64 installations on Arm keep updating x64] → deliberate (D1),
  documented in `install.md`; migration is a follow-on change (Open Question
  1, lead ruling 3).
- [Adding the catalog entry before the workflows breaks release assembly] →
  D10 makes the entry and the workflow legs one commit; phase 2 never touches
  the catalog.
- [Artifact-name collisions between the two Windows OS calls in one CI run] →
  every artifact and download pattern carries `inputs.os` or the target; the
  report collector requires one host triple across its receipts.
- [The gate accepts an unknown Windows label on the Unix branch] → explicit
  allowlist that fails on unknown labels, with test cases for each branch.
- [4 vCPU pushes a shard past 90 minutes] → measure the first run; add a shard
  through `SHARDS` if needed rather than raising the budget.
- [The `windows-11-arm` label resolves to either the Visual Studio 2022 or 2026
  image during the actions/runner-images#14602 migration week] → `dumpbin`
  discovery already goes through `vswhere` component queries
  (`verify-windows-imports.ps1`) with no hard-coded Visual Studio year or
  install root; step zero records the image version and Visual Studio edition,
  and the README is re-read when phase 3 starts.
- [rust-lang/rust#150123 breaks instrumented coverage on
  `aarch64-pc-windows-msvc`] → decided by lead ruling 1 (Open Question 7):
  arm64 runs the same shard suites uninstrumented as separately named
  behavioral evidence outside the 90% gate, and instrumented arm64 shards are
  held until a pinned toolchain carries the fix; step zero records the issue's
  `llvm-profdata merge` step on the pinned toolchain; an uninstrumented run
  never reports or satisfies coverage.
- [`SHA256SUMS` cross-check adds one small download per published release on a
  first release] → bounded to 64 KiB each on immutable paths, authenticated
  against listed digests; only on the no-predecessor path.
- [#106 changes `targets.rs` and the catalog length] → phase 2 rebases onto
  #106; the phase-3 edit goes from `[Target; 4]` to `[Target; 5]` and keeps its
  asserts.
- [The staged fixture's positional asset ids collide or go stale when the
  catalog changes] → A39 derives them from the catalog and asserts uniqueness.
- [`windows-arm64` is not the lock platform key mise writes] → verified at the
  first `mise lock` run; the docs command is updated to whatever mise emits.

## Open Questions

1. Resolved 2026-09-26 by lead ruling 3 (kuru-implement-phase2-sep26a): "Yes: an x64
   kuru.exe under emulation keeps self-updating to x64; migration is a separate
   change (add it to tmp/roadmap/dx-followons.md)." Migration of an x64
   `kuru.exe` running emulated on Arm hardware to the arm64 target (design note
   Q3) is a separate follow-on change, recorded in the shared follow-ons list;
   `kuru update` keeps the installed executable's target (D1).
2. Resolved 2026-09-26 by lead ruling 6 (kuru-implement-phase2-sep26a): "Yes, a job-log line
   plus the two unit-tested branches; "no stable older release at all" stays a
   distinct error." (Was: designer default, needs lead confirmation, that the
   predecessor decision is evidenced by a log line only, not a field in a
   future staged-acceptance receipt.) Applied in D5.
3. Needs lead confirmation: lead decision 4 says PR6a is "merged before PR6b
   starts"; this change runs phase 2 before PR6a merges and treats PR6a as soft
   for `cospec apply` but hard for phase 3 and archive (blocking-changes Phase
   Gates). Blocks the start of phase 2. Not addressed by the 2026-09-26
   rulings; lead ruling 7 (a verification draft PR later rebased onto PR6a's
   branch) presupposes phase 2 work before PR6a exists, but this question
   stays open until the lead answers it.
4. Resolved 2026-09-26 by lead ruling 2 (kuru-implement-phase2-sep26a): "Reject an explicit
   -Target that mismatches the native machine, with a message that names the
   native target." (Was: designer default, needs lead confirmation, that an
   explicit `-Target` differing from the native machine is rejected, including
   an explicit x64 install on Arm.) Applied in D2 and the native-windows spec
   delta.
5. Resolved 2026-09-26 by lead ruling 4 (kuru-implement-phase2-sep26a): "Yes: Windows 11 floor
   for Arm only; x64 keeps Windows 10 1809." (Was: designer default, needs lead
   confirmation, that Windows 11 is the Arm floor because of .NET Framework
   4.8.1 native Arm64; Windows 10 on Arm is out of scope.)
6. Resolved 2026-09-26 by the workflow owner's confirmed PR5 shape (was:
   designer default, needs lead confirmation, that release-time Windows
   coverage stays staged/published-only for both architectures). PR5 removes
   the release-time `native-tests` call and the `verify-tests` coverage rerun,
   runs pre-bump `mise run test` on `ubuntu-latest` only, and keeps Windows
   release-time evidence to the `verify-staged` Windows legs and
   `verify-published-windows`; Unix gains the interim staged leg. The default
   stands for both architectures (D8 caveats) and no lead answer is needed.
7. Decided 2026-09-26 by lead ruling 1 (kuru-implement-phase2-sep26a): "[b] accepted:
   uninstrumented arm64 runs of the SAME shard suites (not a subset) are the
   behavioral evidence for memory, cleanup, terminal, install and update; they
   are named separately in the receipts and never counted toward the 90%
   gate. Hold instrumented arm64 shards until a pinned toolchain has the
   llvm-profdata fix; record the upstream issue number in docs and the
   follow-ons file." The arm64 legs run the same shard suites as x64 and Unix,
   uninstrumented, as separately named behavioral evidence (D8 caveats, D11);
   they never report or satisfy coverage, so AGENTS.md's rule against
   substituting an uninstrumented executable in a coverage run holds, and x64
   and Ubuntu keep enforcing the 90% gate. Instrumented arm64 shards (and the
   arm64 `native-platform` coverage upload, which uses the same
   `llvm-profdata` path) are held until a pinned toolchain carries the fix for
   rust-lang/rust#150123; step zero records the pinned toolchain's state. The
   upstream issue number appears in the docs task (3.9) and in the shared
   follow-ons list (`tmp/roadmap/dx-followons.md`).
