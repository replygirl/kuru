## 1. Acceptance and regression

- [x] 1.1 Add a real checked-copy regression that fails before the fix because its three completed byte-work phases publish no ticks; verify exact completed 8 MiB units and unchanged bytes/identity checks after the fix.
- [x] 1.2 Prove a blocked operation advances no progress while blocked, then advances only for accepted bytes after release; retain the existing corrupt-source/corrupt-copy refusal regressions.

## 2. Scoped implementation

- [x] 2.1 Thread existing OpenTicks, ByteTicks and TickingWriter through cold preparation without new timers or fake milestone progress.
- [x] 2.2 Add explicitly gated, bounded phase/refusal/cleanup observations and retained-child plus byte/EOF snapshots without logging output bodies, changing deadlines or releasing owned cleanup early.
- [x] 2.3 Exercise pipe partial/EOF/limit facts and gated diagnostics with isolated fixture state; document the observed fields and the unproved native-stall diagnosis.

## 3. Verification and delivery

- [x] 3.1 Run owning focused memory regressions, host/Windows static checks, formatting, required docs checks and strict Cospec validation; record actual results below and distinguish native CI still unrun.
- [x] 3.2 Obtain independent source review of production, fixture and documentation changes and confirm local acceptance for normal archive and delivery.

## Observed evidence

The owning pre-fix command `KURU_DOLT_CACHE=/private/tmp/kuru-cold-probe-test-cache mise run //packages/kuru-memory:test -- cold_probe_checked_copy_counts_only_completed_byte_work` reached the real checked-copy fixture and exited 101: source hash/copy progress count was 0, expected 2. The corrected same fixture and corrupt source/copy tests passed (2/2); the byte fixture then moved to the shared native module without a Unix gate.

Final `ulimit -n 8192` followed by `KURU_DOLT_CACHE=/private/tmp/kuru-cold-probe-test-cache mise run //packages/kuru-memory:test -- probe` exited 0, 20/20 passed. A prior run under the inherited 256-file limit passed 17 and failed three selected storage/service fixtures: two reported OS24 during template capture and one refused unrecorded interrupted staging quiescence. The identical selection passed all three under the adequate process-local budget, with strict cleanup unchanged and failed roots preserved. The default cache had refused before tests, so it was preserved and the existing isolated override prepared a fresh cache. A new fixture receiver-capture compile error was corrected before the observed pre-fix failure.

Owning memory host lint, Windows-target lint, root format check and docs check (including build/content links) exited 0. Windows lint first caught the test-only convenience wrapper's unused Windows compilation; it is now scoped to its actual Unix-only callers. Independent source/local acceptance was approved after reviewing final production, cross-host regression, isolated child gate/refusal, pipe and docs changes. The original exact CLI command `ulimit -n 4096` followed by `KURU_DOLT_CACHE=/private/tmp/kuru-cold-probe-test-cache mise run //apps/kuru-tui:test -- cli_memory_progress_is_bounded_and_keeps_json_on_stdout` exited0 on macOS. Native Windows and full hosted coverage remain pending the PR's CI; local results cannot identify the earlier Windows stall.

Final strict validation exited0. Actual `mise run cospec -- archive cold-probe-diagnostics --skip-specs` exited0: archived to `openspec/changes/archive/2026-10-05-cold-probe-diagnostics/`, specs skipped. The complete moved archive was confirmed; the tool's no-delta warning was retained without inventing a spec delta or editing generated bookkeeping. Normal hook-backed commit and publication follow the observed archive.
