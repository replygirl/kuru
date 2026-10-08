# Tasks

## 1. Bounded native tool and permission acceptance

- [x] 1.1 Add `apps/kuru-tui/tests/native_live_tools_acceptance.rs` using the real interactive TUI and isolated scratch state; verify exact `gpt-5.6-luna`/low metadata and client resource bounds before dispatch, reject unexpected external tool operations and exercise meaningful offline guard failure cases.
- [x] 1.2 Prepare and review the fixture without paid inference; pass owning app format, lint, typecheck and selected offline guard checks, retaining the existing prepared-supervisor and bundled-engine setup.
- [x] 1.3 Run one authorized bounded native acceptance visibly in cmux `pane:105`; observe actual pending permission with no file effect, approve-once exact bytes, deny with no file effect, matching provider tool-result continuation, normal settlement and terminal/process cleanup. Record exact scalar usage and unobserved limits; do not retry until green.

## 2. Verification documentation and delivery

- [x] 2.1 Update `docs/verification.md` with the actually observed native tool/approval outcomes and explicit limits; pass the owning documentation content/link checks.
- [x] 2.2 Complete observed evidence here and pass strict validation with all local archive prerequisites satisfied; actual archive and normal hooked commit follow this local completion, followed by the authorized PR/main checks and worktree/cache cleanup.

## Observed acceptance — 2026-10-08

- Independent source review found no blockers. The app-owned test task passed
  its complete local suite, including the shared offline permission flow and
  four guard cases; the native paid case remained ignored. App lint and
  typechecking passed with existing pinned tools and prepared bundled engine.
- The visible native child ran in user-offered `pane:105` / `surface:122` with
  advertised `gpt-5.6-luna` / low. Before each decision, Root checked only the
  known synthetic paths: both absent before approve-once, then exact approved
  bytes and denied absence before deny. The settled turn retained those effects.
- One inference run passed: six streams/completions (three deliberation,
  three speaking), two matching current-message receipts, final answer and
  normal `/quit`/owned cleanup. Exact before/after terminal settings matched;
  the child exited 0. Duration was 78.43 seconds within the 180-second cap.
- Usage totals: input 6817, output 418, reasoning 230, cached input 0. Both
  client and native-measured input estimates totaled 11,539; observed payload
  was 988 bytes. No server output-token/billing ceiling is claimed. The first
  pane launch hit its inherited descriptor ceiling during template copy with
  zero inference streams; the successful fresh launch used the local test
  invocation's existing 4096 limit. No paid run was repeated.
- Other tools, API-key inference, every permission choice, fresh login and
  shell filesystem isolation were not exercised by this run. No production,
  authentication-storage, dependency, mise or HTML roadmap changes were made.
- The owning docs check passed its production build, formatting, lint, public
  content, links and anchors. This was local verification, not deployment.
- Root code formatting and strict change validation passed with zero errors
  and warnings. Local archive prerequisites are complete; remote PR/main
  results are tracked separately in the working document.
