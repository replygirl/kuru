# Tasks

## 1. Bounded source inventory and snapshot

- [x] 1.1 Add typed bounded inventory over one readonly SQLite/WAL snapshot without opening each Dolt project; verify multi-scope, committed-WAL, malformed, changing, and over-bound source fixtures
- [x] 1.2 Preserve `run_backup` progress-resetting stall behavior and remove any whole-copy deadline; verify progressing-over-30-seconds, full no-progress stall, and backup-error cases

## 2. Explicit import mapping and refusal

- [x] 2.1 Add explicit exact-scope and moved-scope import with boundary-only key remapping and durable bounded receipt; verify ordering, opaque bytes, scope mapping, and source/snapshot retention
- [x] 2.2 Add typed no-effect refusal for ambiguous scope/family, transformed collision, active or suppressed destination, and changed source; verify no new snapshot/candidate and unchanged source/target/suppression state
- [x] 2.3 Preserve automatic exact-scope first-open behavior and establish inventory/import CLI output without private row disclosure; verify exact legacy skip and real command machine/human results

## 3. N4 owner admission and cleanup lifetime

- [x] 3.1 Acquire checked native/session maintenance admission before staging, distinguish `WouldBlock` from other I/O errors with a typed result, and recheck destination after permit acquisition; verify admitted refusal plus the underlying N4 native draining barrier before filesystem changes
- [x] 3.2 Retain permit and owned resources through local connection close and service cleanup/reap when caller is cancelled; verify the real cleanup order and uncertain outcome handling with an owned fixture

## 4. Documentation and acceptance

- [x] 4.1 Document provider-free inventory, explicit source selection, preserved DB/WAL and rebuildable SHM, refusal remedies, privacy bounds, and import receipt; verify owning docs build and content/link checks
- [x] 4.2 Run focused owning memory/app behavior fixtures, host/Windows lint and typecheck, formatting, Cospec strict/apply/context, and managed drift; record exact commands and terminal results in verification.md
- [x] 4.3 Complete the required rows in verification.md with observed behavior evidence and accurate post-archive hosted/native/coverage deferrals; verify final strict validation and actual archive readiness
