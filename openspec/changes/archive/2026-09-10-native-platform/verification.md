## 1. Private filesystem and identity [critical]

- [x] 1.1 @integration (agent) Create/open private native roots and inherited owner-only descendants under Unicode/spaced paths -> Windows 6d55ac7 passed ordinary/inherited owner ACL, Unicode private-root and identity cases in both normal and instrumented runs; macOS unit plus seventeen filesystem integration cases and the full pre-push gate passed. Evidence: corrected native Windows run below and /tmp/kuru-embedded-runtime-push.log.
- [x] 1.2 @regression (agent) Present unsafe ACLs, junction/symlink ancestors and leaves, hardlinks and special files -> actual Windows weak/null/nonzero-owner-rights ACL, junction/alias/device tests and Unix symlink/mode tests passed without outside mutation; Windows foreign-owner rejection remained active. Evidence: nine Windows units and thirteen filesystem integration results below.
- [x] 1.3 @integration (agent) Contend for a real lock and replace names while movable/pinned handles are held -> native process-lock contention, pinned source substitution and directory authority revalidation passed on Windows; matching Unix lock/substitution tests passed. Evidence: real_process_lock_contention_survives_directory_publication and retained-name cases in the corrected native log.

## 2. Publication outcomes [critical]

- [x] 2.1 @integration (agent) Publish new files/directories and replace checked ordinary files on native filesystems -> all corrected native Windows publication cases passed, including held-target rejection followed by success after close, occupied new-only preservation and identity retention; Unix publication cases passed unchanged.
- [x] 2.2 @regression (agent) Exercise pre-move failure and a controlled post-move error boundary using real filesystem operations -> the native conflicting-handle and shared real-move uncertainty tests passed normally and instrumented, preserving rejected bytes and explicit uncertain publication evidence. No copy/delete fallback or blind retry was added.

## 3. Process and inheritance ownership [critical]

- [x] 3.1 @integration (agent) Round-trip Windows argv/environment through compiled fixture executables -> native_arguments_environment_stdio_and_working_directory_round_trip and invalid_creation_intent passed, including raw UTF-16, empty/quoted/Unicode/metacharacter values, explicit environment and duplicate-key rejection, normally and instrumented on Windows.
- [x] 3.2 @regression (agent) Terminate a Windows fixture owner at startup and established-tree handshakes under its Job policy -> both actual owner-loss tests passed with descendant lock release and unrelated-process survival. JOB_LIST atomic pre-execution assignment remains the native creation contract, not a claim of testing every instruction boundary.
- [x] 3.3 @integration (agent) Run concurrent Windows children with distinct handles and a root that exits before a grandchild retaining output -> both dedicated native tests passed normally and instrumented; the early root exit did not complete the owned tree or close descendant output, and concurrent NativeSpawnSpec handle sets stayed independent. This does not claim coordination with unrelated legacy spawn mechanisms.

## 4. Private IPC and cancellation [critical]

- [x] 4.1 @integration (agent) Stall Windows pipe connect/read/write at parent-controlled handshakes, then cancel and shut down the runtime -> actual cancelled-connect, pending-read, second-write/flush backpressure and explicit close-before-peer-exit cases passed; runtime shutdown and split duplex independent wakeups completed normally and instrumented. Pending native resources remained owned until completion.
- [x] 4.2 @regression (agent) Present preexisting Windows endpoints and unexpected peer processes -> actual first-instance binding, wrong-peer/nonlocal rejection and authenticated rendezvous tests passed before private payload transfer; trusted EOF lifetime cases also passed.

## 5. Independent native quality gate [critical]

- [x] 5.1 @integration (agent) Run package-owned format/lint/test/coverage tasks for all primitives on Windows and shared filesystem contracts on Unix without Dolt, bundles or consumer compilation -> Windows 6d55ac7 passed nine units, thirteen filesystem tests and sixteen process/IPC tests, repeated under coverage with zero ignored or failed cases; native package LCOV is 1834/1994 lines, 91.9759%. Format/lint/typecheck, lock drift check and coverage upload passed; Unix evidence below remains valid.
- [x] 5.2 @integration (agent) Observe required Windows success and real fixture-failure propagation through the aggregate -> corrected Windows job 102960966634 passed at 6d55ac7. Earlier run 34499528190 had all four Unix/product jobs succeed, actual Windows fixture failure and ci-gate 102958076220 fail. This directly proves the intended failure-propagation guard; no separate artificial local failure injection was performed or claimed.
- [x] 5.3 @manual (agent) Review API/dependency tree, unsafe allowances, mise ownership and CI labels -> the library has no domain dependency; Windows FFI is locally contained under package deny while consumers retain forbid; package mise/CI labels and guidance distinguish primitive proof from pending Windows product acceptance.
- [x] 5.4 @integration (agent) Observe the full aggregate CI result at ca68d7d2615dca1217c68a15172f97a8e7825320 -> all four application jobs, the Windows primitive job and ci-gate succeeded; the isolated Windows success alone was not used as the aggregate result.

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

### Corrected native Windows acceptance, 2026-09-10

The earlier pending statements above describe their historical checkpoints and
are superseded for the primitive package by this actual hosted result:

- Candidate: `6d55ac7432929f08713e330c7010e661e5aedda8`.
- The [required Windows job](https://github.com/replygirl/kuru/actions/runs/34503834945/job/102960966634)
  completed **success** at `2026-09-10T16:49:24Z` on windows-2025.
- Nine unit tests, thirteen filesystem integration tests and sixteen process/IPC
  tests passed normally and under LLVM coverage; there were no ignored or failed
  native cases. The fixture executable test harnesses contain zero unit tests;
  their actual behavior is exercised by those integration tests.
- Native LCOV contains five production source files and **1834/1994** covered
  lines (**91.9759%**), with the required 90 percent gate unchanged: shared FS
  348/368, Windows FS 285/309, pipe 393/456, process 352/378, security 456/483.
- Format, strict lint, typecheck, unchanged dependency locks and native coverage
  artifact upload passed. This package-only job did not compile or provision the
  application, memory engine or bundled runtime consumers.
- Evidence: `/tmp/kuru-native-platform-windows-second.log` and downloaded
  `/tmp/kuru-native-platform-windows-second-coverage/kuru-platform-coverage.lcov`.
  Normal pre-push also passed in 435.25 seconds at this checkpoint, recorded in
  `/tmp/kuru-embedded-runtime-push.log`.

Failure propagation was observed, not inferred: in the first native run all four
macOS/Linux jobs passed, while the Windows fixture errors caused both the Windows
job and [ci-gate](https://github.com/replygirl/kuru/actions/runs/34499528190/job/102958076220)
to fail. This fulfills the original injected-failure check's intent through a
real hosted failure; no artificial local injection is claimed. Current aggregate
success is still tracked separately in row 5.4 before archive. These results prove
native primitives, not the still-blocked Windows product integration.

### Final aggregate acceptance

At `ca68d7d2615dca1217c68a15172f97a8e7825320`,
[CI run 34507581204](https://github.com/replygirl/kuru/actions/runs/34507581204)
completed successfully, including all four application targets and
[aggregate job 102988252606](https://github.com/replygirl/kuru/actions/runs/34507581204/job/102988252606).
The [Windows primitive job](https://github.com/replygirl/kuru/actions/runs/34507581204/job/102973453946)
again passed all 38 native tests normally and instrumented, with unchanged
1834/1994 line coverage (91.9759%). The final local gate and normal pre-push
also passed, covering 12786/13138 workspace lines (97.3207%).
Evidence: `/tmp/kuru-native-platform-windows-ca68d7d.log`, its downloaded
coverage artifact, `/tmp/kuru-ci-profile-hosted-checks.md` and
`/tmp/kuru-ci-profile-full-check.log`. These results complete this independent
foundation; Windows application acceptance remains in its consuming change.
