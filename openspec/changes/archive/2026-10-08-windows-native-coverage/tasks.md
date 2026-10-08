# Tasks

## 1. Native process, pipe and console contracts

- [x] 1.1 Extend the existing Windows console/process fixture and `packages/kuru-platform/tests/windows_process.rs` to exercise public console capture/VT/focus through inherited handles, real peer death during queued pipe I/O, Drop before runtime shutdown, and batch representation/working-directory refusal. Retain explicit owned-child cleanup and existing bounded fixture waits.

## 2. Native filesystem and access-policy contracts

- [x] 2.1 Extend existing fixtures in `packages/kuru-platform/src/fs.rs`, `src/fs/windows.rs` and `src/windows/security.rs` for deletion-sharing blockers, READONLY/pending removal, substituted retained directories, NULL/empty/unprotected/changed DACL refusal and finalization without write authority. Verify exact identity, bytes and adjacent-state preservation; restore fixture ACLs/attributes before assertions and cleanup.

## 3. Verification and delivery

- [x] 3.1 Run formatting and the owning platform lint including the Windows target; review native test cleanup and observable assertions. Record actual results and explicitly identify native Windows execution as pending PR CI when unavailable locally. Archive before the final branch commit. All native PR and exact-main behavior/95% coverage checks remain required before merge and completed delivery.

Final source passes Windows-target and host platform lint and repository
formatting. Independent reviews found no concrete defect and verified retained
ACL/attribute restoration, explicit owned-child cleanup, pending observations
and exact state assertions. Native Windows execution is unavailable locally;
PR CI must establish the added console, pipe, deletion-sharing, pending removal
and DACL outcomes and the unchanged 95% gate. No deterministic background pipe
reaper branch coverage is claimed. All PR and exact-main delivery gates remain
incomplete after this source archive.
