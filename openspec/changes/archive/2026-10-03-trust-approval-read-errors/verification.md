# Verification

Authored 2026-10-03 before the fix landed; rows are ticked as evidence lands.

## 1. An OS read failure is named as such and never approves [critical]

- [x] 1.1 @regression (agent) `trust::tests::an_unreadable_record_names_its_os_error_and_never_matches`: approve, chmod the record 000 (non-root runner), `inspect` and `inspect_nested` -> `Unreadable` carrying `EACCES`, displayed as the OS's standard text without a path; no review generation; `approve_command` fails; restored mode 600 reads `Matching`; fails on the unfixed tree (`Invalid`) -> observed 2026-10-03 macOS arm64: before the fix FAILED (`left: Invalid`); after, ok with `Unreadable(EACCES)` displayed as `Permission denied (os error 13)`
- [x] 1.2 @regression (agent) `instruction_gate::tests::an_unreadable_record_at_publication_reports_its_read_error`: review against a readable base approval, answer once, chmod 000, `publish()` -> error chain names "workspace approval record could not be read" and the `EACCES` text, not "changed during review"; base approval unchanged; fails on the unfixed tree -> observed 2026-10-03 macOS arm64: before the fix FAILED (`workspace approval changed during review; review current authority again`); after, ok
- [x] 1.3 @e2e (agent) `tests/cli.rs` `an_unreadable_approval_record_reports_its_read_error_and_remedy`: real `kuru` binary, `trust approve --yes`, chmod 000, `trust status` and `run` -> status names the I/O error and a permissions remedy, not "invalid or unsafe"; `run` fails before memory effects; fails on the unfixed tree -> observed 2026-10-03 macOS arm64: before the fix FAILED (`Status: approval state is invalid or unsafe`); after, ok (1 passed)

## 2. Structural invalidity is unchanged [critical]

- [x] 2.1 @integration (agent) `trust::tests::a_directory_in_the_records_place_never_matches` and the unchanged `symlinked_and_permissive_approval_objects_never_match` and `malformed_oversized_and_hard_linked_records_never_match` -> a directory, symlink, non-private mode, hard link, malformed or oversized record stays `Invalid` on Unix; the directory never approves on any platform -> observed 2026-10-03 macOS arm64: all ok before and after; the directory fixture is rejected by the checked filesystem's synthesized error, so mode 000 is the fixture that reaches `Unreadable`
- [x] 2.2 @integration (agent) `mise run //apps/kuru-tui:test` (full) -> every target passes -> observed 2026-10-03 macOS arm64: exit 0, 405.6 s, every target 0 failed (lib 137, cli 44, terminal 43, trust 22)

## 3. Static checks

- [x] 3.1 @unit (agent) `mise run format:check`, `lint`, `lint:windows`, `typecheck`, `docs:check`, `cospec -- validate --all --strict` -> each exit 0 -> observed 2026-10-03 macOS arm64: each exit 0 (`NODE_OPTIONS` unset for docs toolchain steps); validate 0 errors, 0 warnings
- [~] 3.2 @runtime (agent) Windows execution of `a_directory_in_the_records_place_never_matches` and native Linux runs -> defer: PR CI runs after push; CI reruns are not used as evidence here
