## Context

Current state, `main` at `501ab92d` unless noted:

- `packages/kuru-delivery/src/targets.rs` holds the five-entry `CATALOG`; every
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
  No CI job runs #108's `test:previous-release-update` yet.
- Tooling on `windows-11-arm` (design note §7 and the runner image
  `20260105.41.1`): Rust, mise, mise-action, mr-boxington, hk, taplo, actionlint,
  cargo-llvm-cov, node and communique have native arm64 assets; cocogitto and
  shellcheck are x64-only and run emulated; `github:aligned-team/cospec` 0.7.1 has
  no arm64 asset (blocker B2). The image omits zstd, so `rust-cache` falls back to
  gzip; public-repo Arm runners have 4 vCPU. Whether stock Windows PowerShell 5.1
  is native ARM64 on the image is not itemized. Windows 11 is the Arm floor
  because .NET Framework 4.8.1's native Arm64 support is Windows 11+.
- Constraints: AGENTS.md forbids compile-only support claims, host fallbacks and
  unbundled builds; platform mechanics live in `kuru-platform`; `linux-x64` is the
  only engine build host (lead decision 4); PR6a owns the manifest, both parsers,
  `bundle build`, the Linux determinism job and the arm64 asset data; another
  session owns `.github/workflows` until its PR5 merge notice; #106 defers the
  Intel macOS catalog removal to a follow-on.

Lead decisions applied verbatim: (1) cospec x64 asset under emulation first, else
named exclusion with the reason recorded; (2) generic target-scoped "no
predecessor" rule, unit-tested both ways; (3) built-asset notices belong to
PR6a; (4) PR6a/PR6b split, linux-x64 only build host; (5) PR4a, PR5, PR4b
before PR6a.

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

- Building or hosting the arm64 Dolt engine (PR6a), manifest schema v2, ICU/LLVM
  /mingw-w64 notices, or an upstream arm64 cospec build.
- Migrating an emulated x64 installation on Arm hardware to arm64 (Open
  Questions).
- Removing `x86_64-apple-darwin` from the catalog (#106's follow-on).
- Adding an arm64-only coverage shard, changing `KURU_COVERAGE_JOB_MINUTES`, or
  tuning runner time budgets before a measured first run.
- Windows 10 on Arm.

## Decisions

### 1. Target catalog entry and host selection

Add `Target { triple: "aarch64-pc-windows-msvc", os: "windows", arch: "aarch64",
executable: "kuru.exe", format: ArchiveFormat::Zip }` to `CATALOG` after the x64
Windows entry, and flip the test at `targets.rs:115` to assert
`for_platform("windows", "aarch64")` selects it with `kuru.exe`/`zip`. Do not
assert a final array length anywhere new; the Intel removal follow-on changes it
independently.

`host()` keeps mapping `std::env::consts::ARCH`. That constant is the compiled
target, which is the true native architecture for a native executable and is the
correct answer for self-update: an x64 `kuru.exe` running emulated on Arm keeps
updating x64. Rejected: querying `IsWow64Process2` from Rust so an emulated x64
binary could pick arm64. That would make the updater replace an executable with
one of a different machine type mid-flight and is the migration question, not
target selection.

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
  the native machine throws "target does not match the native machine"; a 32-bit
  process still fails the `Is64BitProcess` check. Rejected: allowing an explicit
  x64 install on Arm, which would silently ship the emulated executable users
  are trying to leave. This is a lead decision (default: reject).
- `Pe()` takes the expected machine (`0x8664` or `0xAA64`) instead of the
  constant; the error text names the expected architecture.
- The manifest pattern, support name, helper cache name
  (`"$Target-$sha.exe"`), receipt helper check and the `PublishSupport` call
  all use `$Target`.
- `tests/fixtures/install-v0.4.2.ps1` stays x64-only: it is the frozen historical
  bootstrap the update fixtures replay.
- Bootstrap tests (`bootstrap_windows.rs`) gain a case that runs the bootstrap
  with `PROCESSOR_ARCHITECTURE=AMD64` injected on every host and asserts the
  selected target equals the test executable's native target, proving the
  environment is ignored; on an arm64 runner the same test proves the emulated
  view is not trusted.

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
  mapping `consts::ARCH` `x86_64` → `AMD64` and `aarch64` → `ARM64` (compile
  error on anything else), and use it at `mise_isolation.rs:64`,
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
| A5 | `consts::ARCH` behavior | out of scope: migration is Open Question 1; emulated x64 keeps x64 |
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
| A29 | `kuru-memory/tests/bundle_build.rs:26-32` | add the sixth target once PR6a's data exists (phase 2, after PR6a merges, or phase 3) |
| A30 | `provision/native_tests.rs:225` | add the sixth target, same timing as A29 |
| A31 | `published_windows.rs:20,365,380` | `WINDOWS_TARGET` becomes an `Options.target` validated against `host_target()` (D6) |
| A32 | `mise_acceptance.rs:917-921` | target-scoped predecessor rule (D5) |
| A33 | `openspec/specs/native-windows/spec.md:15,200` | delta in this change |
| A34 | `openspec/specs/repository-delivery/spec.md:127` | delta in this change |
| A35 | `docs/install.md:135-150,154` | phase 3, final commit (D9) |
| A36 | `docs/release.md:151,180` | phase 3 (D9) |
| G1 | `embedded_runtime.rs:1129,1198,1250` | same helper as A21 (D3) |
| G2 | `windows_cli.rs:34-35,82,163,195-196` | out of scope: passes the real machine environment through and asserts it is non-empty; correct on any host |
| G3 | `tools.rs:116-117,5966-5967` | `:116-117` is the passthrough allowlist, benign; `:5966-5967` is the same synthetic fixture class as A25, same disposition |
| G4 | tool locks with `windows-x64` entries | add `windows-arm64` entries (D7) |
| G5 | #106 leaves the Intel catalog entry | out of scope for this change; rebase plan in D10 |
| G6 | resolver moved to `published.rs` (#108) | this change targets `published.rs` (D5) |
| — | `openspec/specs/embedded-runtime/spec.md:26-38` | out of scope: PR6a's built-source provenance delta |

### 5. Previous-release rule in `published.rs`

Replace the target-blind `select_previous` with a pure, target-scoped selection:

```text
pub enum Predecessor {
    Release(PreviousRelease),
    None { target: &'static str, candidate: String, inspected: Vec<String> },
}
fn select_previous(releases: &[PublishedRelease], candidate: Version, core_name: &dyn Fn(&str) -> Result<String>)
    -> Result<Selection>   // Selection::Release(version) | Selection::None { inspected }
```

Rules, in order, over the unpaginated listing (GitHub's 30 most recent):

1. Keep the existing fail-closed checks: every stable release tag must be
   canonical `vX.Y.Z`; a stable release newer than the candidate is an error; the
   candidate itself is skipped.
2. Walk the remaining stable releases in descending version order. The first
   whose asset listing names `archive::archive_name(version, target)` is the
   predecessor; `previous_release` then downloads and authenticates it exactly as
   today and it must be exercised.
3. A release whose listing lacks the name is not yet "no predecessor": fetch its
   `SHA256SUMS` (64 KiB bound, immutable version path) and require that it does
   not name the archive either. Listing and manifest disagreeing in either
   direction is an error ("previous release vX lists/omits <name> inconsistently").
   Rejected: trusting the listing alone, which would let a release with a missing
   asset-listing entry pass as "no predecessor".
4. If no stable older release carries the archive, return
   `Predecessor::None { inspected }` with every inspected version. If there is no
   stable older release at all, keep today's error: the first release of the
   project is a different situation from the first release for a target.

`previous_release` returns `Result<Predecessor>`. Callers:

- `tests/support/previous_updater.rs::previous_updater_accepts_candidate` takes
  `&Predecessor`; on `None` it prints exactly
  `no predecessor for <target>: inspected v0.9.0, v0.8.0 ...` and returns `Ok`
  without running any updater. On `Release` it runs unchanged.
- `mise_acceptance.rs::run_staged` and `tests/previous_release_update.rs` pass
  the host target and print the same line, so the `verify-staged-windows` job log
  and the CI acceptance log carry the evidence. No new receipt file: the staged
  check has none today and the published verifier does not run the previous
  updater. Adding a `previous_release` field to a future staged receipt is a lead
  decision (default: log line only).
- The rule cannot be a workflow `if:` (`release_workflow.rs` forbids it in
  `verify-staged-windows`), which is why it lives here.

Unit tests (pure, fixture listings, no network):

- `predecessor_is_the_greatest_older_release_carrying_the_target` — Windows x64
  candidate with v0.10.0 (asset present), v0.9.0 (present): selects v0.10.0.
- `predecessor_skips_releases_without_the_target_when_a_later_one_exists` —
  v0.10.0 lacks arm64, v0.9.0 has it: selects v0.9.0 and reports v0.10.0 inspected.
- `no_predecessor_when_no_stable_release_carries_the_target` — arm64 candidate
  over a listing of x64-only releases: `Selection::None` naming every inspected
  version; draft and prerelease entries are not inspected.
- `no_predecessor_requires_the_manifest_to_agree` — the manifest cross-check
  fails when `SHA256SUMS` names the archive the listing omitted, and when the
  listing names it but the manifest does not (existing `listed_digest` path).
- Existing tests keep passing: newer-than-candidate error, malformed tags, the
  project-wide "no stable release precedes" error, synthetic next-patch selection.

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
  `published-windows-<target>-<version>-<attempt>`.
- `apps/kuru-tui:verify:staged-windows` keeps its single test; `mise_acceptance`
  derives `TARGET` from the host (D3) and the workflow passes
  `KURU_STAGED_WINDOWS_ARCHIVE` with `${{ matrix.target }}` (D8).

### 7. Tooling on `windows-11-arm` and the cospec contingency

| Tool | On arm64 | Action |
|---|---|---|
| rust 1.98.1, cargo-llvm-cov, llvm-tools-preview | native | none |
| mise 2026.9.4, mise-action v4.3.0 | native | none |
| mr-boxington 1.17.0 | native asset; CI sets `KURU_MBX=0` | lock entry for maintainers |
| hk 1.58.1 | native; CI sets `MISE_NO_HOOKS=1` | lock entry |
| taplo, actionlint, node/npm, communique | native | lock entries |
| cocogitto 7.0.0 | x64 asset, `windows_arm_emulation: true` in the registry | lock entry to the x64 asset; smoke-test in step zero |
| shellcheck 0.11.0 | x64 only; not installed on Windows jobs | lock entry to the x64 asset for `MISE_LOCKED=1` completeness |
| cospec 0.7.1 | no arm64 asset (B2) | lead decision 1: `windows-arm64` lock entry pointing at `cospec-0.7.1-windows-x64.zip` via the github backend's per-platform asset selection; step zero runs `cospec --version` under emulation and records it |

Lockfiles: root `mise.lock` (hk, shellcheck, actionlint, taplo, cospec,
mr-boxington), `packages/kuru-delivery/mise.lock` (cocogitto, communique) and
`apps/kuru-docs/mise.lock` gain `windows-arm64` platform entries, refreshed
through the `mise lock --platform ...,windows-arm64` commands documented in
`docs/development.md`. Implementation check: confirm `windows-arm64` is the key
mise 2026.9.4 writes for that platform before relying on it.

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

### 8. CI job mirror for `windows-11-arm` (design only; edits in phase 3)

Step zero, a throwaway job on `windows-11-arm` in the first phase-3 commit,
removed before merge: print `[Environment]::Is64BitProcess`,
`$env:PROCESSOR_ARCHITECTURE`, `PROCESSOR_ARCHITEW6432` and the `IsWow64Process2`
native machine under `powershell.exe` 5.1 and `pwsh`; `$PSVersionTable`;
`vswhere` results for `VC.Tools.ARM64` and `VC.Tools.x86.x64` with the located
`dumpbin.exe` paths; `rustc --print host-tuple` after mise; `cospec --version`
and `cog --version` under emulation. It confirms stock PowerShell 5.1 is native
ARM64 before the bootstrap is trusted on that assumption.

| x64 job | arm64 twin | Parameters | Aggregation |
|---|---|---|---|
| `ci.yml native-platform` | matrix leg `{os: windows-11-arm, target: aarch64-pc-windows-msvc}` of the same job, name `Native platform primitives (${{ matrix.target }})` | `runs-on: ${{ matrix.os }}`, `CARGO_BUILD_TARGET: ${{ matrix.target }}`, rust-cache `shared-key: native-platform-${{ matrix.target }}`, artifact `coverage-native-platform-${{ matrix.target }}` (the current name would collide) | none; `ci-gate` sees the job result |
| `ci.yml native-tests` matrix | add `windows-11-arm` to `os` | — | `native-gate` below |
| `native-tests.yml coverage` (Unix) | not run on arm64 | `if: !startsWith(inputs.os, 'windows')` | gate |
| `windows-coverage` | same job | `runs-on: ${{ inputs.os }}`, `if: startsWith(inputs.os, 'windows')`; artifacts `<prefix>-coverage-<inputs.os>-<shard>-attempt-<n>` and `...-diagnostics-...`; rust-cache key `native-coverage-${{ inputs.os }}`; the PR6a import step (`download-artifact bundle-input-aarch64-pc-windows-msvc` then `bundle:prepare --target aarch64-pc-windows-msvc --archive <file> --offline` into the job's private `KURU_DOLT_BUNDLE_DIR`) conditioned on the os before `coverage:windows:shard`; `install_args` per D7; matrix stays byte-equal to `coverage::SHARDS` (five after #107); `KURU_COVERAGE_JOB_MINUTES` stays equal to the timeout | gate |
| `windows-coverage-report` | same job | `runs-on: ${{ inputs.os }}`; download patterns `<prefix>-coverage-<inputs.os>-<shard>-attempt-*` so the two Windows OS calls never cross-contaminate; upload `<prefix>-coverage-${{ inputs.os }}-attempt-<n>`; the collector requires every receipt to share one host triple | gate |
| `windows-install` | same job | `runs-on: ${{ inputs.os }}`; rust-cache key per os; the same PR6a import step; tasks generalized per D3 | gate |
| `native-gate` | same job | replace the `windows-2025` literal with an explicit allowlist: `windows-2025` and `windows-11-arm` take the Windows branch, `ubuntu-*`/`macos-*` the Unix branch, any other label fails with the label in the message; `release_workflow.rs::native_workflow_gate_rejects_incomplete_windows_results` gains the arm64 accept case, each arm64 reject case and an unknown-label reject case; its Unix case tracks #106's labels | itself |
| previous-release-update CI job | does not exist on main (#108 documents it, nothing runs it) | flagged to the workflow owner; when it is added, one leg per supported native OS including `windows-11-arm`, `KURU_UPDATE_CANDIDATE_BINARY` with `.exe`, `GITHUB_TOKEN: github.token` on the listing step only | must join `native-gate` or `ci-gate` needs |
| `release.yml build` Windows leg | add `{os: windows-11-arm, target: aarch64-pc-windows-msvc}` | already keyed on `matrix.target`/`runner.os`; `needs` gains PR6a's release-scoped `dolt-windows-arm64` job built from `needs.bump.outputs.sha`, and the asserted literal `needs: [plan, bump, verify, verify-tests]` in `release_workflow.rs` changes with it; the artifact download and offline import steps carry `if: matrix.target == 'aarch64-pc-windows-msvc'` (`build` is the one job allowed step conditions) | `assemble-candidate` validates against the catalog, so D1 lands in the same commit |
| `verify-staged-windows` | matrix `{windows-2025, x86_64-pc-windows-msvc}`, `{windows-11-arm, aarch64-pc-windows-msvc}` under the same job id | `runs-on: ${{ matrix.os }}`; `KURU_STAGED_WINDOWS_ARCHIVE: .../kuru-<version>-${{ matrix.target }}.zip`; no `if:` anywhere (asserted); the predecessor rule runs inside the task (D5) | same job id keeps `deploy-docs`/`publish` `needs` unchanged; `release_workflow.rs:101-126` move to the matrix form |
| `verify-published-windows` | matrix over the same two legs | `runs-on`, `KURU_PUBLISHED_TARGET`, receipt `published-windows-<target>-receipt.json`, artifact `published-windows-<target>-<version>-<attempt>`; stays the file's final step (asserted) | none; post-publication |

Caveats carried into the tasks: release-time `native-tests` calls pass no `os`
and therefore run no Windows coverage for either architecture; arm64 coverage at
release time exists only through staged and published acceptance, and the docs
say so. Public-repo Arm runners have 4 vCPU and gzip caches; the 90-minute shard
budget is measured on the first run before any change, and a fifth-or-more
shard needs a `SHARDS` code change first. `windows-11-arm` is GA for public
repositories and the repository is public; minutes are billed at the Arm rate
only for private repositories.

### 9. Documentation and the support-claim rule

- `docs/install.md`: the supported-platforms row `Windows 11 or newer | ARM64 |
  aarch64-pc-windows-msvc`, the bootstrap text (native machine detection, both
  targets, explicit `-Target` must match), and the archive-naming sentence. The
  row and every sentence that says Kuru "supports" or "ships" Windows on Arm land
  only in the final phase-3 commit, after the `windows-11-arm` platform,
  coverage shards and report, install and offline runtime, and staged acceptance
  jobs are green natively. Until then the docs describe the target as
  "in verification" or do not mention it.
- `docs/release.md`: both Windows legs in the build list, the staged and
  published matrix, the target-parametrized receipt name, the predecessor
  sentence from D5, and the note that release-time coverage is Ubuntu only.
- `docs/development.md`: the `windows-arm64` lock platform in the refresh
  commands, an arm64 offline import example using PR6a's built archive, the
  `bundle:test-fixtures` note (A18), and the `dumpbin` host-tools sentence.
- Notices for built assets are PR6a's; this change links to that section rather
  than duplicating it.

### 10. Sequencing and the x64-stays-green guarantee

- Phase 1 (this change): design, specs, this ledger. No product code.
- Phase 2 (on `feat/windows-arm64`, x64 and Unix only): D2 (installer), D3
  (tasks, scripts, fixtures), D5 (resolver; requires #108 merged), D6
  (verifiers), D7 lock entries, and the `release_workflow.rs` gate test cases
  that do not depend on workflow text changes. None of these touch the catalog
  or workflows, so `assemble-candidate`, `expected_assets` and every x64 job are
  unchanged in behavior; x64 CI proves the generalizations on x64. A29/A30 wait
  for PR6a's manifest data.
- Phase 3 (after the PR5 merge notice and PR6a's archive): step zero, then D1
  catalog entry + D8 workflow edits + `release_workflow.rs` text expectations in
  one commit, then D9 docs in the final commit after the native jobs are green.
- Rebase: `feat/windows-arm64` is created from `origin/main` at `501ab92d`;
  before phase 2 it rebases onto main once #106 and #108 merge, and again onto
  the Intel catalog follow-on if it lands first (the catalog edit is a
  two-line conflict at most). PR6a's merge triggers the phase-3 rebase.

### 11. Acceptance mapped to the five native checks

| Native check (AGENTS.md) | Evidence on `windows-11-arm` |
|---|---|
| memory | `windows-coverage` shards `memory` and `runtime` (five-shard matrix) pass with the PR6a engine imported offline; `test:embedded-runtime` cold offline conversation |
| process cleanup | `native-platform` arm64 leg (90% platform coverage) and the `connectors-core-platform` shard's owned-process and supervisor tests |
| terminal | the `application` shard's ConPTY tests |
| installation | `windows-install`: `mise run install` (source), `bundle:verify-native-build`, `verify:windows-imports`, plus the bootstrap tests in the `delivery-archive` shard |
| update | `test:embedded-runtime` install-and-update; `verify-staged-windows` arm64 leg with the predecessor rule; the previous-release-update job once the workflow owner adds it |
| staged acceptance | `verify-staged-windows` `{windows-11-arm, aarch64-pc-windows-msvc}` green on a real Release run |
| post-publication | `verify-published-windows` arm64 receipt retained on the run |

## Operational surface

Every arm64 delivery path runs on the same hosts and with the same secrets as the
x64 one. `windows-11-arm` GitHub-hosted runners (image `20260105.41.1`, 4 vCPU
in public repositories, no zstd) run the native jobs; `ubuntu-24.04` runs PR6a's
engine build and hands the archive over as a workflow artifact that each arm64
job imports offline into its own runner-created private `KURU_DOLT_BUNDLE_DIR`.
No job binds a network listener beyond the existing loopback mise fixture in
staged acceptance; no new secret is introduced, and the only token used is
`github.token` for mise-action tool downloads and, on the previous-release
listing step only, `GITHUB_TOKEN` for the GitHub REST metadata request. Asset
downloads stay anonymous over HTTPS. Binary versions and architectures: Rust
1.98.1 `aarch64-pc-windows-msvc`, mise 2026.9.4 native arm64, cargo-llvm-cov
0.9.1 built from source, cocogitto 7.0.0 and cospec 0.7.1 x64 assets under
emulation, communique 1.3.5 and hk 1.58.1 native arm64, Dolt v2.3.3 built by
PR6a for windows/arm64. Connection limits and deadlines are unchanged: the
resolver's 60-second HTTPS client, 64 KiB `SHA256SUMS` bound, 4 MiB metadata
bound and the existing archive limits. Installed applications never invoke MSVC
tools; `dumpbin` is a build-time acceptance dependency only.

## Integration contract

- GitHub Releases: the predecessor rule reads the unpaginated release listing
  (`/repos/replygirl/kuru/releases`, 30 most recent) and, for a release whose
  listing lacks the target's archive, its `SHA256SUMS` from the immutable
  `releases/download/v<version>/` path. Asset names are owned by
  `archive::archive_name`/`shell_support::archive_name` from the catalog; no new
  naming scheme. Listing and manifest must agree or the release is rejected.
- Windows API: `IsWow64Process2` (kernel32, Windows 10 1511+) through the
  existing audited `Add-Type` bridge only; the Rust side keeps `consts::ARCH`.
  Machine ids: `0x8664` x64, `0xAA64` ARM64.
- MSVC tools: `vswhere` component ids `Microsoft.VisualStudio.Component.VC.Tools.x86.x64`
  and `Microsoft.VisualStudio.Component.VC.Tools.ARM64`; `dumpbin.exe` under
  `bin/Hostx64/x64` or `bin/Hostarm64/arm64`. The check reads imports only.
- mise: the github and aqua backends' per-platform lock entries keyed
  `windows-arm64` (key confirmed at the first `mise lock`); cospec and cocogitto
  entries point at their x64 assets. `MISE_LOCKED=1` stays on.
- PR6a: the sixth manifest asset is selected by Cargo `TARGET` exactly as the
  five upstream ones; this change reads `target`, `stem`, `format`,
  `executable_name` and nothing from `source.built.*`. The workflow artifact is
  named `bundle-input-aarch64-pc-windows-msvc` and imported with
  `bundle:prepare --target aarch64-pc-windows-msvc --archive <file> --offline`.
- Coverage receipts: shard receipts may carry `excluded_artifacts`
  (`[{artifact, reason}]`) only when the contingency in D7 is active; the
  collector reconciles it against the inventory and the report shows it.

## Risks / Trade-offs

- [Stock PowerShell 5.1 on the arm image is not native ARM64 or lacks .NET
  Framework 4.8.1] → step zero prints the process and native machine under both
  shells before any bootstrap test runs; the Windows 11 floor is documented with
  the .NET 4.8.1 reason.
- [The x64 cospec asset fails under emulation] → the named, receipt-recorded
  exclusion mechanism in D7 is designed now so the fallback is a small diff, not
  a redesign; the reason is written in the receipt and the docs.
- [Emulated x64 installations on Arm keep updating x64] → deliberate (D1),
  documented in `install.md`; migration is Open Question 1.
- [Adding the catalog entry before the workflows breaks release assembly] →
  D10 makes the entry and the workflow legs one commit; phase 2 never touches
  the catalog.
- [Artifact-name collisions between the two Windows OS calls in one CI run] →
  every artifact and download pattern carries `inputs.os` or the target; the
  report collector requires one host triple across its receipts.
- [The gate accepts an unknown Windows label on the Unix branch] → explicit
  allowlist that fails on unknown labels, with test cases for each branch.
- [4 vCPU and gzip caches push a shard past 90 minutes] → measure the first
  run; add a shard through `SHARDS` if needed rather than raising the budget.
- [`SHA256SUMS` cross-check adds up to 30 small downloads on a first release] →
  bounded to 64 KiB each on immutable paths; only on the no-predecessor path.
- [The Intel catalog follow-on and this change conflict in `targets.rs`] →
  neither asserts a length; rebase resolves a two-line conflict.
- [`windows-arm64` is not the lock platform key mise writes] → verified at the
  first `mise lock` run; the docs command is updated to whatever mise emits.

## Open Questions

1. Migration of an x64 `kuru.exe` running emulated on Arm hardware to the arm64
   target (design note Q3). Deferrable: it changes no spec, task or approach here;
   `kuru update` keeps its compiled target until a separate change decides.
2. Whether a later staged-acceptance receipt should carry the predecessor
   decision as a field rather than a log line (D5). Deferrable: the log line is
   the evidence surface this change commits to.
