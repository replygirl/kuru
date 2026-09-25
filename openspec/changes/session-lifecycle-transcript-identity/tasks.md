## 1. Session schema and migration

- [x] 1.1 Add the v7 catalog, pending/settled public-turn and fork-provenance schema plus shared validators, and verify a fresh store and exact v6→v7 upgrade retain v6 context/private provenance, the independent v4 usage registry and clean working sets.
- [x] 1.2 Migrate validated saved sessions and transcript namespaces into catalog/legacy-prefix metadata without rewriting source rows, add exact prefix+journal-bound legacy continuation admission for safe pre-v7 retries, and verify proven rows remain readable with unknown speaker/turn fields while ambiguous source rows retain their original bytes and remain present in full-memory export.
- [ ] 1.3 Add typed catalog, turn, settlement, speaker, fork and page DTOs with row/byte/identity bounds, and verify malformed, oversized, cross-project and unsupported-format records refuse before SQL or RPC effect.

## 2. Managed memory contract

- [x] 2.1 Implement revision-pinned bounded catalog and predecessor transcript pages in store, facade and managed RPC, and verify local/remote long-session pagination, exact counts/order, continuation revision checks and sibling/private-row exclusion.
- [x] 2.2 Implement receipt-bearing create, rename, remove, restore and atomic fork publication with typed definite conflicts, and verify accepted lost replies, exact retry, changed-request conflict, cancellation and clone-wide uncertainty fencing on real Dolt.
- [ ] 2.3 Extend the existing turn checkpoint operations to create pending public turns, retain a safe pre-dispatch interruption marker while pending, atomically supersede marked pending work for a distinct turn, and admit/reactivate one typed assistant-only continuation for an older safe ID; verify restart, same-ID retry, intervening turns and lost-reply recovery never duplicate or split user/terminal entries.
- [ ] 2.4 Preserve the catalog, turn records, legacy prefix and fork provenance in full-memory export/restore, and verify round-trip counts, identities, unknown legacy fields and current live revision on real Dolt.

## 3. Runtime lifecycle and public transcript

- [ ] 3.1 Replace the runtime's session-list blob and role-only replay with the typed catalog/public-turn service, and verify ordinary new, exact resume and interruption replay preserve mode, typed content, stable speaker and existing turn retry semantics.
- [ ] 3.2 Implement deterministic continue plus rename/remove/restore policy and reset session-only permissions for every fresh process, and verify removed/missing diagnostics, latest-order tie-breaks and fresh authority through real harness restarts.
- [ ] 3.3 Implement completed/interrupted-turn fork policy over the immutable predecessor chain with atomic inheritance of the source's validated legacy-prefix descriptor, and verify pending/unrelated/removed-source refusal, parent/child divergence, later parent append isolation, legacy-prefix fork-of-fork, ancestor rename/remove/restore and current-memory disclosure.
- [x] 3.4 Retain the current project/conversation driver lease and expose no persisted liveness flag, and verify the existing writer-lease regression still refuses a second driver before P30.

## 4. Session export

- [x] 4.1 Implement one-pass newest-first traversal into a private length-framed spool and reverse chronological Markdown/JSONL rendering, and verify transcripts beyond one page/32 MiB export in bounded memory with valid typed records and exact fork/settlement/speaker provenance.
- [x] 4.2 Publish selected export destinations through checked file identity and atomic replacement, and verify changed destinations, capacity failure and cancellation leave no completed partial output or session mutation and clean or retain staging truthfully.
- [x] 4.3 Verify session export excludes raw actor/relationship histories, private provider summaries, context summaries, notes, candidates and unrelated sessions while full-memory export remains complete.

## 5. CLI, shared registry and TUI picker

- [x] 5.1 Add CLI session list/rename/remove/restore/fork/export operations and root `--continue` without breaking exact `--resume`, and verify real normal processes produce stable machine output and invoke no provider for lifecycle-only commands.
- [x] 5.2 Register `/new`, `/sessions`, `/resume` and `/export` once in the shared command registry and compose them with P04/P27 recovery IDs, and verify help, completion, parsing, modal priority and all existing recovery routing from one table.
- [x] 5.3 Add the real TUI session picker and lifecycle actions with ordinary/removed/pending/fork states, settled boundary selection, inline-rename cancellation restoring its saved input, and current-memory disclosure, and verify narrow/wide real-PTY navigation plus restart parity with CLI.

## 6. Documentation and acceptance

- [x] 6.1 Document session versus memory identity, rename/removal/restoration, settled fork, legacy unknown speaker, export formats, permission reset and the P30 concurrency boundary, and verify docs build, links and command examples.
- [ ] 6.2 Run and record the verification ledger's focused store/facade/runtime/CLI/TUI/PTY cases, package all-target typecheck/lint/format and strict Cospec, preserving failed attempts and deferred native evidence honestly.
- [ ] 6.3 Run the normal combined-coverage hook and hosted supported-platform memory/application jobs, record the exact 90% gate and native results, then archive the change only after every critical row is observed or explicitly deferred by the archive contract.
