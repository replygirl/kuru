## 1. Refresh owned pins

- [x] 1.1 Update the exact TOML dependency and lock entries; verify the official patch/MSRV evidence, Cargo resolution and existing configuration/delivery behavior tests without application source changes.
- [x] 1.2 Update standalone cospec and tool provenance, regenerate integrations through its own command and retain the exact-bundle compatibility preload when required; verify actual standalone clear/blocked/error behavior, archive gates and managed-file drift.
- [ ] 1.3 Pin current npm in the docs app through native mise ownership; verify selected npm/Node versions, clean locked dependency installation and actual docs build/lint/format/link checks in the required documentation CI job, and fail explicitly if task activation selects another version.
- [x] 1.4 Refresh contributor pin/compatibility documentation from observed official metadata and commands; verify ownership, no root language workspace and accurate separation of Codex's provider-bundle update.

## 2. Verify integration

- [ ] 2.1 Run the required granular mise/hk checks and native CI with the refreshed pins; record actual results and at least 90% meaningful workspace line coverage, preserve release/Pages topology, then strictly validate and archive this maintenance record through cospec.

## Observed checks

The exact TOML update resolved without other dependency changes; existing core
and delivery tests passed. The cospec 0.7.1 task reports the expected version,
passes the existing standalone command contract, and regenerates all three
harness integrations without managed drift.

All five supported cospec artifacts passed actual lock-time checksum and
attestation verification. Disposable archive fixtures observed success for an
identical already-synchronized ADDED requirement, rejection of a same-count
scenario-name swap, and rejection of conflicting ADDED content. Both failures
preserved the active change and living spec bytes. The exact 0.7.1 instructions
command returns one JSON document through the task preload and fails without it,
so the narrow workaround remains. Evidence and command logs are indexed in
`/tmp/kuru-cospec071-maintenance-evidence.md`.

The docs app's setup selected Node 26.8.2 and npm 12.0.2, verified the npm tarball
through its pinned SHA-512 tool option, and installed 191 locked packages. The
optional fsevents install script remained blocked under npm's default policy.
The actual docs build and lint passed, and the repository format task completed.
The app lock records the npm backend/version; that backend supplies no per-target
archive entries. The integrated local `mise run check` passed in 460.00 seconds,
including docs build/link/format checks, strict lint, behavioral tests and
14,304/14,731 covered workspace lines (97.10%). The full log is
`/tmp/kuru-native-corrections-check.log`. Native Windows tool selection and
required CI remain pending, so these local results do not complete those tasks.

The user's subsequent `parallel-quality-gates` instruction moves static checks
to Ubuntu and reserves the platform matrix for native application behavior.
Accordingly, this maintenance task now requires exact npm/Node activation in
the owning documentation job, rather than reinstating Windows documentation
checks. Native Windows runtime acceptance remains required independently.

The new d66780f docs job installed npm 12.0.2 but actually activated Node's bundled
npm 11.19.1 under the root monorepo task runner. Its green build is not pin
acceptance: the log reports an npm backend/lock mismatch before setup. The
correction must preserve app-owned versions, resolve the canonical npm backend
at monorepo scope and assert actual versions even when dependency installation
is cached. Task 1.3 remains open; evidence is in
`/tmp/kuru-windows-d66780f-docs.log`.
