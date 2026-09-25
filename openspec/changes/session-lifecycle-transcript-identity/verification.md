The checked historical rows and dated observations in this ledger refer to the
reviewed P11 integration through `e30e25b7`. They are retained as provenance,
not as acceptance of this P09-based reconstruction. Current-lineage verification
remains open until the named checks run on its exact final source.

## 1. Durable turn identity and legacy migration [critical]

- [ ] 1.1 @integration (agent) upgrade a real v6 Dolt store containing several saved sessions, typed transcript rows, a safe pending journal, ambiguous legacy rows and operation-attributed context/private sidecars to v7, then lose and reconcile the accepted safe-journal resume reply -> proven sessions remain readable in order, one prefix+journal-bound legacy continuation completes without another user row, absent historical speaker/turn fields render unknown, v6 provenance rows remain exact, the usage branch remains v4, and every original row remains in full-memory export
- [ ] 1.2 @integration (agent) admit, complete and interrupt real turns through the harness, dropping replies at each accepted checkpoint -> each user entry and pending state is atomic; a safe pre-dispatch marker remains visible and pending until same-ID completion without another user row; completed and terminally interrupted turns settle once with exact speaker/content; typed recovery creates no duplicate transcript node
- [ ] 1.3 @integration (agent) safely interrupt one ID, complete a distinct later turn, then resume the older ID through a dropped accepted continuation reply -> the old interrupted prefix and fork snapshot stay immutable, exactly one assistant-only continuation settles after the intervening head, and no user/raw row or provider dispatch is repeated
- [x] 1.4 @integration (agent) safely interrupt and supersede an older ID more than once before it completes -> the same pending continuation identity is reactivated at the latest settled head, the original fixed marker remains single, and every settled predecessor/content tuple remains immutable
- [x] 1.3 @regression (agent) open the v7 store with the preceding v6 schema validator and compare exact refs/rows before and after refusal -> the older writable path refuses without rewriting session, transcript, context/private provenance or receipt state

## 2. Reversible session lifecycle and recovery [critical]

- [ ] 2.1 @integration (agent) create, rename, remove, refuse resume/continue, restore and resume a session through the managed facade on real Dolt -> the same ID, transcript, mode, private memory, summaries, usage and revision history survive and ordinary listings exclude only the removed interval
- [x] 2.2 @integration (agent) drop accepted rename/remove/restore replies and race a conflicting exact-generation request -> retained request outcomes recover once, changed inputs conflict, uncertain work fences later mutation and definite validation failures do not leave a fence
- [x] 2.3 @regression (agent) cancel each lifecycle operation before acceptance and during an accepted reply wait -> pre-acceptance cancellation changes nothing; accepted work finishes or remains explicitly recoverable with no partial catalog state

## 3. Settled public-prefix fork [critical]

- [ ] 3.1 @integration (agent) fork through a completed answer and through a terminal interruption, then append distinct sentinels to parent and children -> each child keeps the exact shared public prefix and independent suffix while current project notes/policy summaries remain shared and raw actor/relationship histories never cross sessions
- [x] 3.2 @integration (agent) attempt fork through pending, missing, removed-source and unrelated-session turns -> each returns a typed no-effect result, no child is listable, and parent/catalog heads remain unchanged
- [ ] 3.3 @integration (agent) fork a session with a proven legacy prefix, fork that child again, rename/remove/restore its ancestors and append later ancestor turns -> each child retains the exact legacy descriptor and shared prefix, descendant transcript/provenance stays byte-stable, later writes are absent, and every fork discloses current shared project memory
- [x] 3.4 @integration (agent) cancel before fork acceptance and lose the reply after atomic child publication -> the first case publishes nothing; exact outcome recovery proves one complete child in the second, with no duplicate or partially resumable session

## 4. Bounded transcript projection and export [critical]

- [ ] 4.1 @integration (agent) read a transcript exceeding 1,024 turns and 32 MiB through local and managed stores while another turn appends -> opaque pages stay on one captured revision, cover every eligible public turn once, preserve typed blocks/speaker/status, and exclude the concurrent append and all private/sibling rows
- [ ] 4.2 @e2e (agent) export that long parent and a fork to Markdown and JSONL through the real CLI -> both outputs are chronological, JSONL decodes record by record, fork/settlement/speaker/legacy provenance is exact, private reasoning/context rows are absent, and peak in-memory buffering remains one bounded page/record
- [x] 4.3 @integration (agent) replace or invalidate the selected output during checked publication and exhaust the staging/output bound -> no partial final file is reported as complete, source memory is unchanged, and cleanup or retained recovery state is explicit

## 5. Resume, continue and authority boundaries [critical]

- [x] 5.1 @e2e (agent) create several sessions, rename one, remove the newest and invoke exact resume and `--continue` in fresh normal CLI processes -> continue deterministically chooses the latest nonremoved catalog order, exact resume keeps its mode/transcript, and removed/missing selections give actionable no-provider diagnostics
- [x] 5.2 @integration (agent) grant one session-only tool permission, close, then resume/continue/fork in a fresh process -> every path retains conversation data but requires fresh authority before the tool can run. Native completed-frame PTY `fresh_resume_continue_and_fork_processes_reset_session_only_file_authority` passed 1/1 in 10.31 seconds on macOS (2026-09-23): process one used choice 2 for an exact session-only file_write scope and wrote only after approval; fresh exact-resume, deterministic-continue and CLI-fork-child resume processes each rendered the inherited original turn, showed no session or always grants, re-prompted with the file absent, and left it absent after denial. The first test attempt failed because it expected the ordinary composer footer inside the permission modal; the corrected completed-modal-frame expectation passed. Independent source review found no false proof.
- [x] 5.3 @regression (agent) hold the existing project conversation-driver lease and start another P11 process -> the second does not gain concurrent driver authority or claim active-session status; P30 remains the only concurrency admission change

## 6. Shared CLI/TUI session surfaces [critical]

- [ ] 6.1 @e2e (agent) drive `/new`, `/sessions`, `/resume` and `/export` plus rename/remove/restore/fork picker actions in a real PTY across narrow and wide frames -> help/completion/dispatch agree, drafts survive failed actions, removed and pending states are distinct, and no lifecycle command invokes the provider
- [ ] 6.2 @integration (agent) perform each lifecycle action through CLI and observe it after TUI restart, then reverse the direction -> both surfaces show identical identity, label, ordering, status, provenance, disclosure and transcript rows
- [ ] 6.3 @regression (agent) run the existing permission/instruction modals, P04 file recovery and P27 memory-candidate recovery commands with all session IDs registered -> modal priority and the seven existing recovery exceptions remain intact with one command table

## 7. Repository and hosted native gates

- [ ] 7.1 @regression (agent) run focused migration/store/facade/runtime/CLI/TUI fixtures plus owning all-target typecheck, lint, format, docs and strict Cospec -> every scoped check passes on the exact final source
- [ ] 7.2 @regression (agent) run the normal combined coverage hook -> the exact final lineage passes the 90% workspace line gate without exclusions
- [ ] 7.3 @e2e (agent) run native Windows, macOS and Linux memory/application jobs with real bundled Dolt and PTY support -> restart, lifecycle, fork, export and cleanup pass on each supported platform; unavailable host-specific evidence remains explicitly deferred

## Current-lineage local verification on P09 `ccfa67a4`

The reconstructed P11 source passed owning memory, runtime and TUI all-target
typechecks. Focused real-Dolt memory filters passed for malformed/foreign public
rows, atomic mode checkpoint, managed lifecycle receipts and candidate isolation,
v5→v6→v7 provenance/export, stopped-store backup/restore, and accepted public-turn
reply loss (six exact filters, 1/1 each). The first malformed-row attempt could
not bind private loopback in the sandbox; its identical native rerun passed. The
public-turn fixture initially used an obsolete hardcoded transcript namespace;
it now derives the same canonical project scope used by the service, without
changing its pending/completed, receipt, no-duplicate or candidate assertions,
and its native rerun passed. Delivery independently reviewed that test correction.

Four focused real-Dolt runtime filters passed 1/1 each: aborted-turn FIFO
recovery and permit release, typed public-prefix fork/resume, changed-catalog
resume refusal, and new/resume/fork session-only authority reset. Two normal
CLI filters passed 1/1 each: provider-free lifecycle/restart/selected export,
and the greater-than-32-MiB chronological parent/fork export. Real-PTY
CLI↔TUI lifecycle parity passed 1/1 at 120 columns with an added 80-column
same-row pending identity/status/label assertion; fresh-process exact resume,
continue and fork session-only file authority reset passed 1/1. The parity
fixture first assumed all shared-registry help entries fit one viewport, then
found the pending picker row clipped behind a full UUID. It now pages help;
the Sessions-only popup and compact state-first row keep pending/fork visible
at 80 columns without changing exact ID selection. The picker hides its cursor,
so its resize assertion observes the actual row instead of waiting for a
composer cursor trailer. Delivery's bounded review cleared the final UI and
fixture correction, including the valid removed-plus-pending state.

Owning memory, runtime and TUI all-target Clippy passed after boxing the private
checkpoint variant, removing a stale RPC lint expectation made obsolete by the
boxed View operation, and correcting a test-only cloned slice. Rust formatting
passed. `mise run docs:check` passed its build, local-link and content gates;
an initial sandboxed attempt stopped during pinned Node/npm setup before docs
validation, then the same owning task passed with tool-install access. A full
combined coverage run, hosted supported-platform jobs and archive remain open.

## Focused schema-boundary checks on release-based P09 `be261442`

P11 was rebased without dropping the reviewed session behavior onto the corrected
P09 source and signed v0.8.0 main. Its real schema is v7; the migration test
registry uses a synthetic v8, whose unknown future authority begins at v9.
Strict Cospec validation and apply both exited 0 with a clear gate. On this
rebased source, the package-owned Dolt prefetch passed, all 17 focused memory
migration tests passed, the separate store migration validation passed 1/1,
and memory all-target Clippy passed. The 17 cases include real v6-to-v7
upgrade/reopen/export, synthetic v8 receipt and attempt validation, retained
v2 progression, and future-version/no-mutation refusals. A scoped ordinary
workspace-local artifact clean preceded these checks; it preserved the separate
live coverage target. These checks do not replace final combined coverage,
supported-platform native jobs, or archive.

## Current-lineage correction evidence after the first combined hook failure

The normal P11 push from `c4a78e79` failed during combined coverage and did
not advance a remote ref. Its exact log is retained at
`/private/tmp/kuru-p11-push-c4a78.log`. The failures exposed distinct test
fixture drift across v7 catalog authority, selected-session public context,
PTY paging, and instrumented package size, plus one runtime interruption-retry
defect. The fixture corrections retain the original no-effect, privacy,
identity, and exact-session assertions. The interruption correction requires
an already-settled exact public turn before treating a repeated marker as a
no-op; its real-Dolt regression passed 1/1 with unchanged revision, public
page, and event count for the exact retry, refusal for a different turn, and
an untouched later pending admission. Delivery reviewed that product delta.

Focused real-Dolt reruns on this corrected local source passed the released-v1
stage upgrade and managed fork lost-reply fixtures 1/1 each. The candidate
dream-save definite pre-send refusal, failed preference update, failed
mode/focus/relationship saves, and failed undo recovery each passed 1/1 with
their no-effect or recoverability assertions. Three first-run notice tests
passed 1/1 each, covering visible notice, failure-before-marking, and absence
from the provider request. The real PTY help paging and selected-session
preference failure tests passed 1/1 each; the latter binds the injected catalog
fault to the newly created selected ID, observes the fixed public uncertain
write error without the private SQL value, restores by exact mode CAS, and
checks unchanged preference/HEAD, clean Dolt status, and all four zero-turn
session modes in the catalog's newest-first order. The UI adapter's real
relationship completion and selected public speaker test passed 1/1. The
real PTY trust fixture passed 1/1 after the two preflight-only refusals kept
their five-second bound and the persistent approval path used the configured
cold-memory startup bound; it required the exact missing Responses key error,
no alternate screen, and the saved approval. A sandbox-only attempt of the v1
fixture failed before its logic because native Dolt could not bind loopback;
the identical native rerun passed with loopback access.

An earlier independent-copy measurement on the previous instrumented source
reduced 148,132,824 bytes to 132,622,296 with macOS `strip -x -S`; that was
feasibility only. The exact filtered `cargo llvm-cov` embedded-runtime test
then passed 1/1 on this corrected local source. Its selected instrumented Kuru
executable was 147,840,200 bytes, inode 664816725, one link, SHA-256
`be2b446fc1ec0ae3025b31ccd6c0620e3b3fa85bb0bd308d49d7631f2fae9c4c`.
The fixture verified an independent exact copy before stripping and an
unchanged named source identity/digest afterward; the prepared private input
was 132,393,624 bytes under the unchanged 134,217,728-byte cap. Actual direct
install and self-update each persisted a chat from an empty offline engine
cache using the 58,795,497-byte bundled archive and Dolt 2.3.3. The fixture
also required one nonempty profile from that prepared executable; the matching
profile under the shared nested `llvm-cov-target` was 536,776 bytes. This is
focused instrumented behavior and profile evidence, not the final 90% workspace
coverage result. Affected memory/runtime/TUI all-target Clippy, Rust format,
strict Cospec validation (0 errors, 0 warnings), and diff check passed after
these corrections. Full combined coverage, supported-platform CI, and archive
remain open.

## Historical local evidence on the reviewed P11 source (2026-09-23)

The observations below belong to the earlier reviewed P11 integration through
`e30e25b7`. They document why the corresponding source and fixtures were retained;
they are not test results for this reconstruction on P09 `ccfa67a4`.
On the reconstructed source, strict Cospec validation and the apply gate both
exited 0 and reported a clear gate. The current-lineage focused checks above
supersede the former local-check gap; coverage and supported-platform jobs
remain unrun on this lineage.

- A focused real-Dolt abrupt-caller-abort regression passed after adding same-Harness recovery: two provider calls held both permits while a third peer Work had entered its actor and waited; abort dropped both providers and released both permits, and a new turn completed only after a FIFO drain acknowledgement from every actor and exact pending-journal interruption settlement. The waiting cancelled Work never reached the provider, the usage invocation count matched only the two entered cancelled calls plus the new turn, and retrying the old possible-dispatch ID refused without a provider call. The first sandboxed attempt could not bind Dolt's loopback port; the same filtered test passed with process permission. This closes the specific integrated cancellation/new-turn failure, while the accepted checkpoint reply-loss cases in row 1.2 remain open.
- A focused real-Dolt mode checkpoint passed 1/1: before any public turn, the checked v7 transaction rejected a mismatched runtime mode without changing the revision, then committed catalog mode and matching runtime state together; stale expected mode retried without changing that commit. Its managed-service fixture passed 1/1 after a foreign-scope mode request and a malformed combined mode/public-turn request both reconciled as exact no-effect with unchanged revision, then dropping an accepted mode checkpoint reply: a sibling observed catalog and state together, exact receipt reconciliation reported committed, and no public turn was fabricated. The runtime exact-resume fixture passed 1/1 after mode switch, full store close/reopen before its first turn, another turn, and a second close/reopen; it restored Jungian mode and transcript, rejected a wrong-project bound store before catalog-only fallback, and opened the other project through its own bound store. The existing publication-pause fixture passed 1/1 with the catalog already changed while the in-memory profile remained old, then reconciled the accepted checkpoint and published the new profile. A separate real-Dolt stale-generation refusal passed 1/1: failed mode save left catalog/runtime revision, live mode and topology unchanged, and exact resume retained the old mode and renamed label. A test-support one-shot, instance-scoped pre-send state-save refusal is available for the remaining focus/dream/undo no-partial-publication fixtures; those fixtures are not yet claimed here.
- Focused real-Dolt session-listing interleavings passed 2/2: a rename after the pinned catalog page and a newly created session after that page both reject stale projection before return. The first sandboxed attempt could not bind Dolt's private loopback port; the same test passed with the required process permission.
- Normal-process CLI lifecycle coverage passed 1/1 in 7.35 seconds on the latest fixture: provider-free rename/remove/restore/fork/JSONL/Markdown, exact resume, deterministic continue after two removals, selected existing export replacement, chronological child export and parent suffix exclusion. A seeded private note/state sentinel is absent from both session formats and present in full-memory export. The earlier narrower versions passed 1/1 in 6.36 and 6.07 seconds.
- Both chronological renderers passed a 1,025-record, greater-than-32 MiB length-framed spool check, decoding every JSONL and Markdown record in order without omitting the page boundary (1/1 in 2.79 seconds). A test-only variable shadowing error failed compilation on the first attempt; the corrected test passed.
- Real PTY session picker, settled-boundary fork, narrow resize and restarted child resume passed 1/1 in 8.74 seconds without a lifecycle provider request. It passed again 1/1 in 6.68 seconds after the TUI dispatch enum was boxed for all-target lint. The shared typed TUI session command regression passed 1/1 in 5.15 seconds on that final UI layout.
- Real-Dolt v5→v6→v7 migration/reopen/export passed 1/1 in 6.51 seconds with explicit ambiguous state/message export assertions. It retained an exact legacy journal continuation, unknown-attribution prefix, v6 context/private reasoning provenance and independent usage v4. Original ambiguous state/message values matched their database bytes after upgrade and remained in full-memory export. The earlier version passed 1/1 in 4.25 seconds.
- The populated migration fixture passed again after invoking the preceding v6 validator against the upgraded v7 store: it refused with the expected future-schema reason, left the exact Dolt head/refs/status unchanged, and left every full-memory export record byte-equivalent after the refusal. The fresh-v7 older-validator regression also passed 1/1. This closes the separate v6-validator regression row; accepted-reply-loss migration cases remain open.
- The same populated v5→v6→v7 fixture passed 1/1 after replacing its local safe-journal resume with a managed accepted-reply-loss interleaving. A sibling observed the exact migrated legacy continuation pending before the client was cancelled; the retained operation receipt reconciled as committed. The reopened store then settled one assistant-only answer, kept one original user row, and passed its existing v6 provenance, usage-v4, full-export and older-validator checks. The first extension failed to compile because it named the local store type for a managed facade constructor; correcting that test-only type passed.
- Three existing migration validations passed focused real-Dolt 1/1 each after correcting v7 test drift seen in the frozen integration coverage. The released-v1 upgrade fixture now expects a writable upgrade to v7, six ordered migration ancestors and receipts 2–7; the forged-receipt fixture inserts future version 8 instead of colliding with the genuine version-6 primary key. A captured ordinary reproduction of the historical/out-of-order fixture failed only because its current-v7 branch was still expected to report version 6/5; the updated expected version 7/6 retained its exact nonmutation checks and passed. These are fixture corrections, with no migration implementation change.
- Local/managed public-transcript paging and candidate isolation passed 3/3 in 8.06 seconds, including 1,025 records and a real managed page boundary.
- The long-page real-Dolt regression passed 1/1 in 5.39 seconds after strengthening its drift case: 1,025 pinned turns were covered once, then an actual 1,026th turn was admitted and settled; a fresh page saw it and the old opaque cursor explicitly refused revision drift.
- Atomic raw-row admission passed 1/1 on real Dolt: a stale namespace count refused with no catalog, public, raw or journal effect; the correct count bound row two, and the exact logical receipt replayed without duplication after a later raw append. The runtime restart safe-retry regression passed 1/1 with the same persisted journal anchor before and after resume. The first storage-test attempt failed to compile due to test-only enum update syntax; the corrected test passed.
- The inherited real-Dolt `older_safe_turn_continues_after_intervening_settled_turns_without_another_user_row` proof passed and its unchanged source was reviewed here: two later settlements park/reactivate one continuation node, preserve its original interruption marker, and settle one assistant-only suffix after the latest head. No checkpoint code for this path changed during this checkpoint.
- The managed real-Dolt public-turn lost-reply fixture passed 1/1 after extending its accepted admission pause with an accepted settlement pause. Each cancelled reply reconciled from the exact retained operation receipt as committed; the resulting catalog exposed one completed turn and raw history held exactly one user and one assistant row. The fixture had previously supplied a changed first-turn label and therefore received a correct `LabelChanged` rejection before its admission assertion; using an initially empty label matches ordinary new-session admission. Reattaching the cancelled primary service connection before the second reply barrier kept the test on the intended transport boundary.
- The same managed fixture passed 1/1 after adding an older retryable turn, one later completed turn and a fork through the older interrupted boundary. Its accepted older `Resume` reply was dropped only after a sibling saw the assistant-only continuation at the later head; exact receipt reconciliation reported committed. The old primary's user/marker stayed single, final raw history appended one assistant answer after the later turn, and the child's captured public prefix stayed byte-equivalent. The existing runtime repeated-safe-retry fixture separately proves its provider was not called for a second safe stop; the lost-reply interleaving here is storage-scoped.
- The managed real-Dolt lifecycle fixture passed 1/1 after extending its accepted-reply-loss path to create, remove and restore. Each create/rename/remove/restore future was also polled behind the held shared mutation gate then cancelled before send; catalog/revision stayed unchanged and its following real operation succeeded. During the accepted rename's held reply, a sibling stale-generation request returned a typed no-effect refusal. Every accepted lost reply reconciled as committed with the retained catalog generation/state; the existing exact-receipt replay after later removal, changed-request conflict, clone-wide uncertainty fence, and candidate isolation also passed. The first extended attempt placed a pause on an attachment retired by the earlier cancelled rename; that call completed before the pause. The fixture now reconnects that test attachment before each new paused operation and passed. The managed fork fixture passed 1/1 with the same pre-acceptance cancellation proof plus its existing one-child accepted lost-reply recovery and candidate isolation.
- A focused real-Dolt terminal-interruption fork passed 1/1: the settled interrupted boundary retained exact speaker, typed terminal marker and predecessor; later completed parent/child suffixes stayed independent, and their raw transcript namespaces had 4 versus 2 rows. The existing runtime completed-boundary fork/restart fixture passed 1/1 after adding a post-fork current project-note assertion: the child sees the newly written shared note and inherited public prefix while its own raw session transcript starts empty.
- The same completed-boundary fork/restart fixture passed 1/1 after adding exact parent-only actor and relationship history sentinels. Both child session-scoped private windows were empty before its turn, and every child provider request excluded both private sentinels while retaining the inherited public prefix and current shared project note. The first run could not bind Dolt's private loopback port in the sandbox; the identical filtered test passed with the required process permission. The combined P09 policy-summary sharing assertion remains open for row 3.1.
- The inherited real-Dolt legacy-prefix fork-of-fork fixture passed 1/1 after adding missing-node and unrelated-session settled-node refusals. Its existing pending and removed-source refusals plus both new cases return distinct typed reasons, keep the parent row and Dolt revision unchanged, and leave no refused child in the final catalog. This closes row 3.2 without repeating separate fork suites.
- The existing real-process project writer-lease regression passed 1/1 in 9.38 seconds: a second conversation driver was refused and the first lease released on close.
- Deterministic session-export abort and injected capacity failure passed 2/2 in 0.08 seconds. Both tests left an existing destination unchanged and no orphan private stage/spool; abort occurred while the spool writer was active. The auxiliary cleanup identity and drop order had an independent bounded source review.
- A real-Dolt permission regression passed 1/1 in 3.44 seconds: new, exact resume and fork each cleared a granted session-only tool permission. Existing fresh-Harness construction coverage checks startup reset. The critical fresh-process authority row remains open pending that exact process acceptance.
- Public docs build, formatting, links and content passed through `mise run docs:check` after the session and command references were updated. Rust formatting passed through `mise run //:format:rust`.
- The first capacity test attempt failed because an outer `anyhow` context hid the inner capacity error in `to_string`; the assertion now prints the full chain and the corrected test passed. The first Rust format check after CLI assertions found one long line, which was corrected. Final Rust formatting, `git diff --check`, and strict Cospec validation passed (0 warnings).
- The first privacy fixture revision tried to open a second writable Dolt owner after normal CLI processes had left their managed service alive. Its startup timed out before assertions. Moving the private fixture writes before the first CLI process gave that setup its own complete lifecycle; the corrected test passed 1/1. It did not change application code.
- Memory, runtime and TUI owning all-target typechecks and Clippy tasks passed on the final adjusted source. The first lint attempts exposed enum layout and test-only lifetime/type warnings; each was corrected before the final passes.
- The first normal pre-push combined-coverage run on the P11 branch failed in one
  runtime fixture after 194/195 runtime library cases passed. The selected-source
  fit-retry test still expected one public row after `observe_selection` began
  settling a user and assistant pair; the actual `(history, public, notes)`
  count was `(4, 2, 1)` against its stale `(4, 1, 1)` expectation. The adjacent
  unbounded fixture already expected two. Only that assertion was corrected;
  the measured window, retry count, omission and current-receipt checks remain.
  The corrected focused real-Dolt test passed 1/1 in 9.91 seconds. Its first
  sandboxed attempt could not reserve Dolt's private loopback port; the same
  filtered test passed with process permission. The normal combined coverage
  gate on the corrected source, hosted Linux/Windows/macOS jobs and archive
  remain open. The critical rows still open above distinguish exact end-to-end
  interleavings/surfaces not proved by the focused local checks from those
  later repository gates.

## Open after this local checkpoint

- Rows 1.1–1.3 still need the broader several-session/operation-attributed migration matrix and remaining accepted checkpoint reply-loss paths. The focused proofs now cover abrupt same-Harness cancellation with parallel and queued actor work, exact nonreplay of the old possible-dispatch ID, a real migrated safe-journal accepted resume reply loss, accepted older assistant-only continuation reply loss after an intervening settlement with a stable fork, migration bytes/provenance, v6-validator refusal without changed refs/rows, accepted public admission and completed settlement recovery, same-ID retry and repeated safe continuation.
- Rows 2.1, 3.1 and 3.3 retain complete metadata-survival, combined P09 policy-summary sharing and the remaining fork provenance matrix. Managed tests prove all lifecycle pre-acceptance cancellations and accepted lost replies, exact receipt replay after later removal, candidate isolation and one lost accepted fork; store/runtime tests now prove both completed and terminally interrupted fork boundaries, independent suffixes, actor/relationship private-history isolation, current shared project notes, all four typed fork refusals and fork-of-fork ancestor independence.
- Rows 4.1–4.2 still need a concurrent append interleaving for pinned local/managed paging and exact legacy provenance in an actual CLI export. Current tests prove stable 1,025-turn local/managed paging, explicit cursor drift after an actual later turn, bounded spool and greater-than-32-MiB parent/fork JSONL and Markdown through the normal CLI.
- Rows 6.1–6.3 still need the remaining picker rename/remove/restore and draft behaviors in narrow/wide PTYs, full bidirectional CLI/TUI restart parity, and the full P04/P27 recovery/modal regression with new session IDs. The current-lineage fresh-process exact resume/continue/fork permission-reset and CLI↔TUI lifecycle picker fixture passed as recorded above; they do not by themselves cover every action in these rows.
- Row 7 requires the final integration line's coverage and supported-platform native jobs before archive. No coverage or hosted result is claimed from this P11 worktree.
