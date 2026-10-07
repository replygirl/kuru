# Verification

Acceptance was authored before implementation. The rows below record observed local evidence and explicit limitations; the chronological observations retain earlier failures without counting them as passes. Native Windows/macOS/Linux hosted coverage and remote publication remain separate from local checks.

## 1. A native cut restores complete memory while writers continue [critical]

- [x] 1.1 @integration (agent) real managed Dolt backup while two checked independent session writers advance at Kuru-owned barriers -> 62447 passed the two-writer captured-head/history/usage and offline restore case; pinned native source locks end before transfer. The held barrier follows the positively completed copy, so no literal byte-copy overlap is claimed.
- [x] 1.2 @integration (agent) seed session journals/private histories/summaries, candidate refs/history, released migration receipts and same-dataset usage history before native cut -> 37708 passed retained-session journal/history and candidate CLI restore/resume; 29966 retained a real checkpointed private summary body/provenance, main/usage ancestry and candidate head through ready-stage recovery; 33368 retained released migration history. Exact payload inventory contains only the native image, excluding external credentials/endpoint/claim/cache records.

## 2. Verification proves native restorability, not only byte integrity [critical]

- [x] 2.1 @integration (agent) verify and restore a real image using bundled offline native restore/fsck plus actual SQL/ref/working-set reads -> 41754, 11203 and 15514 passed native root/graph and exact committed/staged/working selectors; 60798 completed prepare/publication/independent verify; 37708 passed actual offline CLI verification. Original image/root/inventory remained unchanged.
- [x] 2.2 @integration (agent) tamper metadata, physical file/name/size/digest and a native corrupt/invalid image with recomputed physical inventory -> 56150 passed strict metadata/inventory/provenance refusal; 62447 passed recomputed-inventory checksum-corrupt native-image rejection without activation or input repair/deletion. Exact retained directory identity is checked in source; a forced concurrent pathname-substitution injection is not claimed.
- [x] 2.3 @unit (agent) malformed/duplicate/path/root/manifest bounds and streamed output beyond retained prefix -> initial 11464 retained four passing cancellation/closing/output/NBS checks; corrected dedicated-directory metadata fixture passed 56150. Native output was drained beyond its retained prefix to EOF without exposing body content.

## 3. Compatible absent-target restore retains history with fresh authority [critical]

- [x] 3.1 @integration (agent) native restore of supported historical main schema and current dirty main/usage with distinct staged/working data, historical candidate and usage refs -> 33368 passed released schema-1 dirty actual-value migration; 29966 passed distinct main/usage snapshots, original ancestry, clean writable reopen/recovery and unchanged candidate/image. A different engine release's native writer was not exercised; compatibility claims remain bounded to bundled native reader and released schema evidence.
- [x] 3.2 @e2e (agent) CLI backup/verify/restore to absent explicit remapped root then resume its retained session offline -> corrected 37708 passed real offline verify/remap refusal/restore/list/resume/export and backup-of-restored flow under fresh target authority, without a provider route for the image commands.
- [x] 3.3 @integration (agent) backup-of-restore and source/stale-target handles -> 37708 independently verified backup-of-restore; 29966 proved fresh instance and retained checked history namespace while the original source remained separately owned. Canonical target/project/instance and stale-generation refusal use the existing checked attachment/claim paths, independently source-reviewed; no new stale-handle fault injection is claimed.
- [x] 3.4 @integration (agent) active and retained-cleanup driver against public restore, existing target, unsupported future schema/invalid receipts -> 44389 and retained 29966 proved active and sole retained cleanup native holds refuse restore before target creation; 37708 proved existing-target refusal. Future-schema and receipt refusal use unchanged released registry/authority validation, source-reviewed rather than a new backup-specific future-schema fixture.

## 4. Cancellation and publication preserve concrete ownership [critical]

- [x] 4.1 @integration (agent) accepted backup client drop/cancellation and checked publication lost reply at existing causal barriers -> 62447 passed actual connection EOF and lost already-written reply, with independent native verification of the published target. Discarded actual CALL result is a test hook modeling absent completion proof, not an actual network outage; it and completed-cut cancellation retained named private images without hash/manifest/publication/deletion.
- [x] 4.2 @integration (agent) restore/native validation setup/wait failure, dropped caller and bounded cleanup refusal -> 44389 passed actual native-spawn caller drop retaining maintenance until same-owner cleanup/reap, with absent target and preserved stage. Earlier native setup/error paths retained original errors through checked fixture quiescence. Source review covered retained cleanup after bounded refusal; no forced cleanup-refusal injection is claimed.
- [x] 4.3 @integration (agent) stopped ready/interrupted restore stage and substituted source/destination identity -> 29966 passed stopped ready-stage movement under exact maintenance/seal and ordinary format-2 recovery before reopen. Source identity checks and exact seal-directory comparison were independently reviewed; simultaneous forced close plus pathname substitution remains unrun, without being inferred from the happy-path recovery.

## 5. Owning protocol, documentation and native scope stay truthful

- [x] 5.1 @regression (agent) owning protocol operation classification/golden and changed lifecycle/schema/session scope fixtures -> 33368 passed four wire-1.13 checks with documented golden regeneration, classification/no SQL receipt/existing four-hour reply budget preserved; 37708 passed the corrected historical checkpoint prefix through real resumed turn and 29966 passed ready recovery. Existing authority/claim validation remains unchanged.
- [x] 5.2 @regression (agent) owning host/Windows lint, typecheck, format, docs and strict/managed validation -> final production memory/runtime/app host and Windows lints all passed; after the last test-only extension, memory 78016/29056 and format 68020 passed. Docs 66501, separate managed check and actual strict/apply 76813 passed; all eight returned context files were read. No pin/private validation/coverage weakening.
- [~] 5.3 @runtime (agent) supported-host native CI and 90% workspace coverage after authorized publication -> defer: ordered hosted CI runs after archive/normal commit and authorized branch publication. No local coverage writer or native Windows/Linux execution was run; cross-target lint is not native support acceptance.

## Observed checks and causal corrections

- Owning memory all-target/all-feature typecheck passed (19475, then corrected
  manifest/native diagnostic source 4811 and copied-image reader 69591,
  99365, then reviewed root/registry correction 1184, exit 0); Rust formatting and
  `git diff --check` passed. At that foundation stage, unconnected APIs produced temporary
  dead-code warnings; those compiler outcomes alone did not establish final
  lint, protocol, docs or application acceptance.
- Initial owning `backup::tests` selection 11464 exited 101: four passed and
  two failed. Actual passing cases cover cancellation notification after a
  real request, existing closing-scope structure, draining beyond the retained
  output prefix to EOF, and pinned NBS header/root rejection. Those passes are
  retained without repeating them.
- Native-only diagnostic 6832 exited 101 (one failed, 8.44 seconds): the
  lease-only restored validation stage lacked a fixture quiescence record.
  The fixture now awaits existing exact-stage quiescence after worker
  settlement and releases its root with the original Result.
- Affected two-case rerun 23099 exited 101 (zero passed, two failed,
  8.25 seconds). It exposed the native exit-status failure after cleanup;
  metadata writing also created an unintended `staging` directory through
  the ordinary replace-record helper. Unpublished metadata now uses checked
  exclusive creation, sync and name verification before outer publication.
- Affected two-case diagnostic 56150 exited 101 (one passed, one failed,
  10.39 seconds): exact metadata/inventory/provenance passed. Bounded
  isolated fake-data diagnostics proved native **restore** rejected an empty
  author name while initializing its absent destination. The fixed Kuru
  author metadata is now confined to the fresh private validation home;
  its successful rerun is recorded below. This diagnostic failure itself did
  not establish copy/restore/fsck success.
- Corrected native-only retry 41754 exited 0 (one passed, 8.31 seconds;
  owning task 100.45 seconds). The bundled cold fixture completed dedicated
  SQL sync-url plus positive finished-command observation, made a later live
  write, restored the captured dataset root through native Dolt, passed
  native fsck, and retained the unchanged source-image root. Native-step
  cancellation remained unset after success, and the restored stage/source
  completed checked cleanup. Full retained-ref/working-set SQL validation,
  concurrent-writer, cancellation/lost-response, historical migration,
  restored-session and CLI acceptance remain unrun.
- Exact native selector case 11203 exited 0 (one passed, 3.24 seconds;
  owning task 78.67 seconds). Real dirty state established three distinct
  committed/staged/working root hashes, and fixed same-branch `AS OF`
  selectors returned each exact marker; staged table enumeration also
  succeeded without a source commit/reset. This covers selector semantics,
  not the unfinished independent copied-image SQL validator.
- SQL-reader compiler attempts 87091 and 34114 exited 101 before behavior:
  SQLx 0.9's `describe` is an offline-feature API, and ordinary `prepare`
  requires its explicit `SqlStr`. The reader now uses the existing public
  prepare/statement metadata API. Corrected compiler outcomes above do not
  establish the unrun native validator integration fixture.
- Copied-image validator 95379 exited 101 (one failed, 14.29 seconds),
  surfacing its original working-set table refusal after checked fixture
  cleanup. A bounded phase-only diagnostic rerun 33508 exited 101 (one
  failed, 11.62 seconds): branch ref 10's working root lacked the main-only
  `context_summaries` table returned by the committed `AS OF 'HEAD'` read.
  Pinned native source resolves that selector through the repository head
  ref; committed reads now use the exact captured commit hash instead,
  while fixed `WORKING`/`STAGED` stay branch-relative. The affected rerun
  was then pending; neither failure established full validator acceptance.
  Retained-stage cleanup now requires the quiescence seal's directory
  identity to match the held stage and revalidates its name around root
  observations. This source correction is not a substitution fault-test
  pass.
- Exact-selector retry 25428 exited 101 (one failed, 11.73 seconds):
  native `DOLT_HASHOF_DB` accepts named refs through its non-special argument
  path, not raw commit hashes. Pinned source separately proves `HEAD` reads
  the already checked DSess selected commit root. The reader keeps captured
  commit hashes for committed `AS OF` reads and uses that supported selected
  head root only after exact `DOLT_HASHOF('HEAD')` proof.
- Corrected copied-image validator 15514 exited 0 (one passed, 11.38 seconds;
  owning task 73.70 seconds). Actual native restore/fsck plus SQL inspection
  retained current main schema, exact candidate committed/staged/working
  roots and dirty state, candidate tag, usage branch and arbitrarily named
  usage-head tag, and unopened native branch. Original image root and live
  candidate working root remained unchanged; checked stage/source cleanup
  completed. This does not establish the unfinished public API, historical
  migrations, restored session remap, concurrent writer or copy cancellation/
  lost-response cases.
- New prepare/verify compiler 93181 exited 101 before behavior (Result type
  inference and a partially moved supervisor path). Corrected memory
  all-target/all-feature typecheck 13136 exited 0; this is not final lint or
  public API acceptance.
- Owning preparation selection 34409 exited 101: the existing structural
  closing guard passed, while the new preparation/publication/independent
  verification fixture failed at checked private-stage removal. The removal
  used inherited privacy, but checked tree removal requires owner privacy;
  it now reopens with stricter `OwnerOnly` validation. The fixture also
  retains its source outside the fallible body and awaits exact source and
  preserved-stage quiescence before releasing its root on all Result paths.
- Changed-case retry 29914 exited 101 before libtest (a Unix-only fixture
  `Option` type annotation), with no new behavioral result. Corrected same
  single-fixture command 60798 exited 0 (owning task 92.45 seconds). Its
  final assembled output truncated the libtest count; no count or test-body
  duration is asserted. The scoped command exercises prepare without target
  activation, checked absent publication, unchanged manifest/image through
  separate native verification, later source writes, existing-target refusal
  and pre-staging cancellation. It does not establish managed RPC, concurrent
  capture, accepted cancellation/EOF, restore/remap or historical migration.
- Dirty restore preparation was refined from the owning clean-write and
  usage-content invariants: main and usage append exact staged/working
  snapshots in the private target at any supported schema, with original
  heads/coordinates retained and unchanged clean validation before activation.
  Actual strict validation and apply exited 0; all eight returned context
  files were read. This gate is artifact acceptance, not an unrun restore pass.
- Restore compiler 93203 exited 101 before behavior because the checked
  stopped-stage move requires a mutable seal. Corrected memory all-target/
  all-feature typecheck 61725 exited 0. Fixture compiler 78668 then rejected
  three test-only API uses (private migration constant, digest formatting,
  and an unnecessary generic argument); those were corrected without changing
  restore semantics.
- Native remapped dirty restore 85226 exited 0 (one passed, 6.31 seconds;
  owning task 103.49 seconds). Current-schema main and usage retained distinct
  staged/working snapshot provenance and original-head ancestry; their target
  branches became clean and writable. The remapped target had fresh external
  identity, retained the original checked history namespace and working private
  value, preserved the candidate head, and accepted later state and usage
  writes. The input inventory/image remained unchanged. This is not historical
  schema, service concurrency, accepted cancellation/lost-response or CLI
  behavioral acceptance.
- App all-target/all-feature typecheck 24528 exited 0 with the public backup,
  verify and restore CLI draft; no real command execution is implied. Memory
  typecheck 49358 exited 0 after the new released-v1 dirty restore fixture and
  protocol 1.13 contract/sample draft. At that point the historical fixture and documented
  protocol golden regeneration remained unrun.
- Historical restore and protocol selection 33368 exited 0 (five passed,
  7.29 seconds; owning task 89.95 seconds). A real released schema-1 native
  database retained separate staged/working values through original-image
  validation, then restored and migrated the actual working private value,
  interpreted retained message text as `text-v1`, preserved original-head
  ancestry, and accepted a later write. The backup image stayed unchanged.
  Protocol 1.13 regenerated through the documented `KURU_BLESS_PROTOCOL_PIN=1`
  owning command; backup mutation classification, no SQL receipt, and exact
  existing four-hour reply budget each passed. This does not claim a native
  image written by a different engine release or service/CLI/cancellation
  acceptance.
- Pinned native source lock inspection: `SyncRoots`/`pull.Clone` captures a
  source root and table-source snapshot before transfer; `NomsBlockStore::Root`
  and `Sources` release their source locks on return, before the clone
  transfer. Destination root/manifest updates own separate destination locks.
  This source evidence supports the forthcoming real managed writer case;
  it is not a measurement or assertion of overlapping byte-copy timing.

- Normal dependency integration `52eaf43f` preserved exact M1 WIP stash
  `7034054a`, all older refs and protocol 1.13. The retained startup-signal
  helper now receives the same invocation listener for all three image
  commands. Integrated compiler 40971 failed only an inherited Doctor
  activation literal missing new optional fields; test-only `None` defaults
  corrected it. Memory typecheck 99440 and app typecheck 67992 exited 0.
- Managed concurrent backup 47074 exited 0 (one passed, 14.40 seconds;
  owning task 109.30 seconds). A real selected writer committed before the
  cut, acknowledged a second private-history and usage write while the
  positively completed SQL cut was held, and continued after publication.
  The immutable image retained exact earlier main and usage heads. Separate
  native verification passed, then the source owner closed and reaped before
  offline remapped restore proved the original private history/session and
  usage marker were present and later writer rows were absent. Together with
  the pinned native lock-scope inspection this proves continued admission;
  the test pause is after copy, not evidence of literal in-copy overlap.
  Attempt 37193 completed preparation but ran no tests because its libtest
  separator was wrong; only the corrected invocation ran this case.
- Focused coupled memory selection 62447 exited 0: five passed in 8.40 seconds
  (owning task 96.85 seconds). The existing finite writer fixture now exercises
  two independently claimed sessions, both acknowledging later writes and
  continuing after publication; offline restore retains each exact captured
  private history and excludes its later rows. The selected EOF/lost-publication
  fixture proves actual connection EOF settles the same handler, while a lost
  already-written reply leaves a destination that independent native verification
  accepts. A test-only discarded actual CALL result models missing completion
  proof, not an observed network outage; it and completed-cut cancellation keep
  named private images without manifest/publication/deletion. Recomputed physical
  hashes do not authorize an actual checksum-corrupt native image. Closing-scope
  coverage passed for all new async fixtures. No literal byte-copy overlap is
  claimed.
- First CLI selection 19793 failed during linking with ENOSPC before any test
  ran. After generated-output recovery, 31497 ran the real CLI flow and the
  structural guard: the guard passed, capture/safe output/existing-image refusal
  passed, but offline verify failed because the new explicit data root did not
  yet exist at the outside-workspace check. The early command now uses the
  existing private-root creation inside the retained-signal operation, before
  engine/image work. Its failed-case rerun was then pending; the guard pass is retained.
- CLI rerun 41836 exited 101 after 13.29 seconds of fixture execution: offline
  verification, explicit-remap refusal, remapped native restore and retained
  session listing passed, then resumed turn rejected its historical transcript
  prefix as a different canonical project. The owner turn/mode checkpoints and
  facade turn checkpoint now validate only the positively checked historical
  prefix for a restored store; ordinary canonical checks and unbound fixtures
  retain their prior behavior. Remaining production project-scope checks were
  inspected and all govern canonical attachment, claim or selection identity.
  Memory all-target/all-feature typecheck 67133 exited 0; the corrected real
  resumed-turn flow was then pending; 37708 below records its terminal pass.
- Initial final lint tasks failed before acceptance on three helper corrections:
  an unnecessary returned local binding, an import only used on Unix, and a
  Unix-only fixture spawn gate referenced by the Windows native helper. Those failures
  did not establish successful static acceptance. Docs build/local-link/content
  check 66501 and separate managed-file check exited 0.

## Final local closure

- Corrected CLI flow 37708 exited 0: one passed, 16.99 seconds of fixture
  execution, 183.96 seconds owning task. It completed actual offline backup,
  independent verification, explicit-remap refusal and restore, retained session
  listing, resumed turn/export with historical checkpoint namespaces, and
  backup-of-restore verification. Original source bytes were parked intact after
  actual source quiescence, so restore did not rely on the source pathname.
- Native ownership selection 44389 exited 0: three passed in 7.90 seconds,
  118.05 seconds owning task. Native restore's actual spawned child retained the
  maintenance permit through dropped-caller cancellation and same-owner reap;
  active and sole retained-cleanup driver holds refused public restore before
  creating its target. Existing closing-scope coverage also passed.
- Final changed dirty-remap fixture 29966 exited 0: one passed in 7.60 seconds,
  65.77 seconds owning task. A real private context checkpoint retained its exact
  record, body and source provenance. After restore, exact maintenance ownership
  and a stopped-directory seal moved the ready target to the normal sibling
  staging name; ordinary reopen recovered its format-2 activation. Distinct
  main/usage root coordinates, original ancestry, fresh authority, unchanged
  candidate head, later writable state/usage and unchanged backup inventory
  assertions remained intact. No forced close or concurrent pathname substitution
  was injected.
- Final production-source owning lints all exited 0: memory host 11290 and
  Windows target 78754; runtime host 36029 and Windows target 97769; application
  host 85501 and Windows target 66274. After the test-only ready-recovery
  extension, owning memory host lint 78016 and final format check 68020 exited 0;
  Windows target lint 29056 also exited 0 (80.04 seconds owning task).
  Memory all-target/all-feature typecheck 67133 and integrated app typecheck
  67992 passed. Docs build/local-link/content check 66501 and separate managed
  drift check passed. Cross-target lint is compiler evidence, not native Windows
  execution.
- Root and independent Luna source reviews cleared exact native child/pipe/reap
  lifetime, retained maintenance ownership, SQL uncertainty versus positive
  completed-cut proof, fresh target/origin separation and the three narrowly
  checked historical checkpoint-prefix consumers. Root accepted the final
  ready-stage fixture scope. No review is counted as an unrun behavior pass.
- Explicit remaining limits: a different engine release's native writer,
  simultaneous forced native-close failure plus pathname substitution, actual
  transport-discard of the SQL CALL reply (the hook models that missing proof),
  and separate stale source-handle/future-schema fault injections were not run.
  Existing checked instance/registry/authority and exact directory validation
  remain unchanged and source-reviewed. Hosted supported-platform behavior,
  90% workspace coverage and merged-main checks await authorized publication.

- Final actual strict validation and apply gate 76813 exited 0 with no warnings,
  a clear gate and all eight returned contexts read. All nine scoped local tasks
  are complete; actual coupled archive and normal hooked branch commit follow
  separately. Hosted acceptance remains explicitly deferred above.
