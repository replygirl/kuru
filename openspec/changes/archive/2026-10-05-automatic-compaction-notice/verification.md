# Verification

## 1. Exact accepted checkpoint and private-safe parity [critical]

- [x] 1.1 @integration (agent) trigger automatic and manual compaction for independent part/relationship sources on actual Dolt with private sentinels -> the seven selected runtime compaction fixtures plus closing guard passed 8/8 (94030), including threshold/exact-range notices and topology-ordered manual results; private sentinels stay out of notice text and original rows remain retained
- [x] 1.2 @integration (agent) exercise refusal, empty output, provider failure and typed stale checkpoint -> invalid outcomes and stale checkpoint cases passed in 94030; the two amended cases passed 2/2 (66148), explicitly asserting no accepted notice and preservation of prior usable context

## 2. Cancellation, aborted reply and superseded cursor [critical]

- [x] 2.1 @integration (agent) hold an actual accepted checkpoint reply and cancel/abort the caller, then reconcile its exact existing receipt -> managed runtime aborted-reply recovery in 94030 passed, delivering one exact summary-ID notice after successor recovery without replay; accepted cancellation also passed in runtime and the real CLI/TUI fixture 57820
- [x] 2.2 @integration (agent) advance the cursor after an accepted checkpoint and confirm its earlier exact identity through the selected-view memory projection -> memory selection 77324 passed 4/4, including retained superseded-ID metadata, body absence and unchanged revision; direct retained primary-key confirmation does not depend on the current cursor
- [~] 2.3 @integration (agent) prove a checkpoint absent and retain an unresolved transport failure separately -> defer: actual absent-ID and refused/stale paths passed in 77324/66148, and exact lost-reply recovery passed in 94030; a separate confirmation-read transport-error injection was not run. Independent source review confirms the awaited read returns before acceptance/rejection on error, retaining the unresolved attempt for later exact-fence recovery, without a guessed success

## 3. Candidate and memory boundary [critical]

- [x] 3.1 @integration (agent) compact a candidate actor and inspect live/candidate confirmations plus typed recovery -> local candidate metadata isolation passed in 77324 and runtime candidate-labelled notices passed in 94030; amended managed candidate confirmation passed 1/1 (85863), with live returning absent and complete reply excluding the private body
- [x] 3.2 @integration (agent) submit malformed/mismatched exact identity/provenance and inspect complete metadata replies -> 77324 checks invalid input refusal and complete body-free replies; facade validates same-ID response and all provenance/range fields before return, independently reviewed with the read-only contract and documented protocol 1.10 golden gate

## 4. Observable TUI and headless delivery [critical]

- [x] 4.1 @integration (agent) force activity receiver lag and operation success/failure/cancellation through the app's real settlement path -> deterministic UI late-event/2,048-operation/actual broadcast-lag case passed in 32556, preserving once-only presentation and bounded retained dedup; confirmed interactive cancellation passed in corrected 57820 with a completed Compacted/Cancelled composer frame
- [x] 4.2 @e2e (agent) trigger automatic compaction through an isolated provider-fixture real PTY at practical widths and synchronize on completed frames/settlement -> existing automatic-compaction PTY passed 1/1 in 32556 at 120/80 with exact usage and resumed-summary assertions; completed frames display actor/range/retained-record metadata and complete PTY output excludes the private sentinel
- [x] 4.3 @e2e (agent) run real headless answer and JSON commands with automatic compaction, then deliver Ctrl-C after causally observing an accepted checkpoint -> 32556 and amended 57820 passed: exact plain answer stdout, parseable JSON, checkpoint-confirmed root-only SIGINT with empty interrupted stdout and confirmed stderr, owned cleanup/reap; the appended TUI cancellation leg observes confirmed metadata before normal restored-terminal quit

## 5. Repository integration and gates [critical]

- [x] 5.1 @integration (agent) integrate final B/U3 commits normally, run scoped closing guards, relevant owning checks, host/Windows lint, format and docs build/content -> normal integrations and the scoped behavior selections recorded below passed; all three owning host lints, Windows-target lints and all-target typechecks exited 0, as did root format, managed drift and full docs build/content/local-link checks; independent final source/evidence review is clear
- [~] 5.2 @integration (agent) run actual strict validation, apply and archive plus normal hooks -> defer: strict validation and actual apply cleared with exit 0 and all seven contexts read; final strict validation passed 0 errors/warnings. Actual archive is the immediately following required local action before delegated final Git commit; archive existence/output will be checked and handed to the delivery owner. Normal final commit/push hooks and hosted native/coverage/live acceptance have not run and are not claimed by this local readiness record

## Artifact preparation evidence

The change was scaffolded through the owning mise/Cospec task on checked
origin/main `e6fa5d864f9c884c394cfb481516c4528e156013`. Proposal and each required
artifact instruction were read before authoring. Initial strict validation
exited 1: a required integration-contract section was missing and the declared
B/U3 dependency archives were absent from this worktree. The section was added;
actual apply then exited 1 solely for those two missing dependency references.
That result is not an implementation clearance.

B's clean archived commit `101add0255c1122e94fe11cb594429576049020b` was subsequently
integrated by normal fast-forward, preserving these untracked C1 artifacts.
U3's clean archived commit `57e6e311c4871244ff0d0efb84ee16c897e99daa` was then
integrated by ordinary merge `b514e8c`, with normal hooks and without copying
source or archives. The first merge-hook validation refused two archived but
unchecked blocker boxes; actual apply synchronized both and exited 0, clear,
with nine pending tasks. Subsequent strict validation exited 0 with no errors
or warnings, and the normal merge commit completed. All seven returned context
files were read fully. The later B native fixture correction commit `a774f07e`
was integrated by ordinary merge `c1d9f0f9`, retaining C1 changes separately.

## Current scoped local evidence

The selected-view exact summary metadata seam and runtime notice collector are
implemented. The owning memory selection passed 4/4: retained superseded-summary
confirmation, candidate isolation, managed lost-reply recovery and the documented
protocol surface gate (minor 1.10). Complete serialized metadata excludes the
private summary body. This confirms checkpoint presence, not body equivalence:
the existing summary identity hashes provenance and source range, not the body.

The owning runtime selection passed seven compaction fixtures plus the existing
closing-scope guard, 8/8 (94030), with process-local fd4096 and a dedicated verified
cache. The two subsequently amended provider-failure/stale cases passed 2/2
(66148); the amended managed candidate metadata case passed 1/1 (85863). No passed
broad suite was repeated. These cover automatic thresholds, ordered manual
results, cancellation after an accepted checkpoint, aborted-reply exact recovery,
candidate isolation, invalid provider outcomes and stale checkpoint refusal.

The app selection passed 4/4 (32556): UI late accepted-event interleaving, 2,048
settlements and real broadcast receiver lag with bounded dedup; the established
closing guard; the automatic-compaction PTY at 120/80; and real headless plain,
JSON and accepted-checkpoint SIGINT. The headless fixture then added accepted
interactive Escape cancellation. Its first run (80023) reached the completed
notice/Cancelled frame and private sentinel checks but failed at a quit helper
that incorrectly awaited another idle frame after exit. Only that teardown was
changed to send/exit/restoration; corrected 57820 passed 1/1 (11.65 seconds).
Earlier literal word-wrap assertions failed despite visible wrapped notices;
those assertions were corrected without changing production wording. The first
headless turn creates eligible private rows through its ordinary seven-peer
Deliberate phase before the selected actor's Speak request compacts its own
history; it does not rely on a hidden seed or concatenate peers' histories.

The final individual owning host lints (19945, 60617, 21057) and all-target
typechecks (86746, 45732, 33152) exited 0. Full docs build/content/local-link checks
(74807), root formatting (55481) and managed drift (18111) exited 0. The initial
multi-address lint invocations (44155/66326) failed because mise forwarded the
additional task names as compiler arguments; they established no source failure
or check pass and were replaced by individual owning tasks. Concurrent docs
tool setup printed an hk config-lock warning, but the owning docs graph completed
successfully. All three individual Windows-target owning lints (61579, 43262,
30300) exited 0. Final strict validation exited 0 with no errors or warnings.
All changed production/tests remained frozen during these terminal checks.

Independent Architecture final source/artifact/docs review is clear, including
the exact read-only memory
projection, acknowledgement/drain ordering, bounded UI lifecycle, Run
cancel/await/cleanup seam and owned signal/PTY fixtures. The separate confirmation
read-error injection remains unrun as stated in 2.3; reviewed code retains the
unresolved attempt on read error. These are local real-Dolt/subprocess/HTTP-fixture
results, not paid live-model proof. Full native behavior, hosted coverage and paid
live-provider acceptance have not run for this C1 branch. Normal final Git hooks
and publication belong to the delegated delivery owner after actual archive;
neither is claimed here.
