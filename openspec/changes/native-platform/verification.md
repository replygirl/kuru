## 1. Private filesystem and identity [critical]

- [ ] 1.1 @integration (agent) Create/open private native roots and inherited owner-only descendants under Unicode/spaced paths -> actual ACL/mode and full identity checks accept legitimate objects on Windows and Unix.
- [ ] 1.2 @regression (agent) Present unsafe ACLs, junction/symlink ancestors and leaves, hardlinks and special files -> checked operations reject them without changing outside content or permissions.
- [ ] 1.3 @integration (agent) Contend for a real lock and replace names while movable/pinned handles are held -> lock ownership remains exclusive and substitution is prevented or detected before protected work.

## 2. Publication outcomes [critical]

- [ ] 2.1 @integration (agent) Publish new files/directories and replace checked ordinary files on native filesystems -> content and identity are correct, occupied new-only targets remain intact and unsupported moves fail without copy/delete fallback.
- [ ] 2.2 @regression (agent) Exercise pre-move failure and a controlled post-move error boundary using real filesystem operations -> old bytes survive rejection and uncertain results retain enough identity/outcome evidence for caller reconciliation rather than blind retry.

## 3. Process and inheritance ownership [critical]

- [ ] 3.1 @integration (agent) Round-trip Windows argv/environment through compiled fixture executables -> empty, quoted, Unicode and metacharacter values remain exact and duplicate-key handling is explicit.
- [ ] 3.2 @regression (agent) Terminate a Windows fixture owner at creation/startup phases under its Job policy -> no owned child/grandchild persists beyond bounded cleanup and unrelated processes survive.
- [ ] 3.3 @integration (agent) Run concurrent Windows children with distinct handles and a root that exits before a grandchild retaining output -> no inheritance leak, root-only false completion or orphaned output reader occurs.

## 4. Private IPC and cancellation [critical]

- [ ] 4.1 @integration (agent) Stall Windows pipe connect/read/write at parent-controlled handshakes, then cancel and shut down the runtime -> bounded completion with no stranded worker or handle leak.
- [ ] 4.2 @regression (agent) Present preexisting Windows endpoints and unexpected peer processes -> first-instance/private-access/identity checks reject them before private payload transfer.

## 5. Independent native quality gate [critical]

- [ ] 5.1 @integration (agent) Run package-owned format/lint/test/coverage tasks for all primitives on Windows and shared filesystem contracts on Unix without Dolt, bundles or consumer compilation -> actual fixture counts and native coverage pass with no required skips and the existing workspace floor is preserved.
- [ ] 5.2 @integration (agent) Observe the required package-only windows-2025 job and aggregate gate at the candidate commit -> all real primitive checks pass and an injected fixture failure in local gate validation is not hidden by successful Unix/product jobs.
- [x] 5.3 @manual (agent) Review API/dependency tree, unsafe allowances, mise ownership and CI labels -> the library has no domain dependency; Windows FFI is locally contained under package deny while consumers retain forbid; package mise/CI labels and guidance distinguish primitive proof from pending Windows product acceptance.

### Local filesystem evidence, 2026-09-10

On macOS arm64, `mise run //packages/kuru-platform:coverage` passed its ordinary
and instrumented runs: fifteen filesystem integration tests and one real
post-move uncertainty unit test. Native line coverage was 446/477 (93.50%):
316/336 in the shared filesystem API and 130/141 in its Unix implementation.
No production source was excluded. The raw-name fixture uses a direct native
control: APFS rejected the invalid UTF-8 name, and Kuru preserved that error
without creating a lossy replacement. Accepting filesystems must round-trip
the exact bytes.

Strict Unix lint and Windows-target Clippy passed after the native ACL/junction
fixtures were added. Evidence is recorded in
`/tmp/kuru-platform-fs-local-evidence.md`; the measured artifact is
`target/kuru-platform-coverage.lcov`. These results do not execute Windows code.
Native Windows evidence and the subsequent owned-pipe implementation remain
pending; the combined acceptance rows above are therefore still unchecked.

Two publication regressions subsequently failed against the initial code and
passed after correction: weak candidate permissions are rejected before replacing
a private destination, and directory moves reject implicit access-policy changes
in either direction. Both preserve the source and destination on rejection.
Streaming identity revalidation reduced transient descriptor use without dropping
ancestor checks. The final macOS run passed seventeen integration tests and one
unit, with 458/486 native lines covered (94.24%). Raw RED/GREEN, strict lint and
coverage logs are `/tmp/kuru-platform-fs-publication-{red,green,clippy,coverage}.log`.

The final Windows cross-target typecheck and strict Clippy passed with sixteen
process/IPC integration cases authored. Source review drove owned native I/O
completion, independent split-reader/writer wakeups and atomic fixture receipts.
Both initial startup and established-tree owner-loss fixtures are present;
atomic assignment before execution is the native JOB_LIST creation contract,
not a claim of testing every instruction boundary. Actual Windows execution
and coverage remain pending. Logs are
`/tmp/kuru-platform-windows-{typecheck,clippy}.log`.

Tool freshness review verified mise 2026.9.4 from the official release metadata
and matching local SHA-256 before execution. CI and Release use that exact
version; the release job structure and publication rules are unchanged.

The complete `mise run check` passed locally with mise 2026.9.4 in 430.37 seconds,
including actual Dolt behavior, new filesystem tests, strict lint/format, docs,
cospec validation/managed drift and workspace coverage. Measured coverage was
12,204/12,538 lines (97.3361%), without exclusions or a threshold change.
Evidence: `/tmp/kuru-native-platform-check.log` and `target/coverage.lcov`.
The native Windows job remains the next unobserved acceptance step.

After forwarding the LLVM profile destination through cleared fixture
environments, Windows-target Clippy and the complete gate passed again in
343.39 seconds with the same 12,204/12,538 covered lines. No generated profile
files remained in source directories. Logs are
`/tmp/kuru-platform-profile-windows-lint.log` and
`/tmp/kuru-native-platform-profile-check.log`.

### First native Windows run

Required Windows job
<https://github.com/replygirl/kuru/actions/runs/34499528190/job/102946492180>
ran for candidate `3d00bebda8e277858cb9adafccf717db1fe4b372`. Tool setup and
native compilation/lint reached the real tests: seven unit tests passed, then
nine filesystem integration tests passed and four failed. Failures exposed
open-target/open-descendant rename restrictions and a default token-owner
difference for inherited private objects. The dedicated sixteen process/IPC
tests did not execute because Cargo stopped after the failed filesystem binary;
native coverage also remains unmeasured. Package test tasks now request
`--no-fail-fast` to collect independent binary failures while retaining a failing
gate. No test retry or threshold relaxation was added.

The lock-contention fixture did execute its first native child successfully:
an owned Job, inherited output pipe and bounded wait returned the expected
busy result. This is narrow evidence only, not a substitute for the dedicated
process suite. Diagnosis and corrections remain in progress.
Evidence: `/tmp/kuru-native-platform-windows-first.md` and corresponding `.log`.

The corrections retain write-through publication and exercise its real Windows
restrictions explicitly. A held replacement target must remain unchanged on
failure; closing it permits publication. Directory tests close descendant data
handles while retaining directory authority, and keep the lifecycle lock beside
the directory being moved. Unix old-handle behavior remains tested.

New private Windows descriptors inherit a zero-access OWNER RIGHTS entry.
Alternate ownership is accepted only for the actual TokenOwner with effective
suppression of implicit owner access; positive grants remain restricted to
TokenUser. Added native ACL cases inspect ordinary creation and sealing, then
verify missing, inherit-only and nonzero owner-rights entries are handled without
repair. Windows-target typecheck and strict Clippy passed; macOS passed its one
unit and seventeen filesystem integration tests, plus strict lint and formatting.
These corrected Windows cases still await native execution. Evidence:
`/tmp/kuru-platform-windows-fs-fix-evidence.md`.

The integrated repository gate then passed in 467.28 seconds with 97.3207%
workspace coverage. This validates the retained Unix behavior and shared build
without establishing the corrected Windows runtime results. Log:
`/tmp/kuru-embedded-platform-corrected-check.log`.
