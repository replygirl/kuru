# Tasks

## 1. Name Dolt's reason in the fixture's failure output

- [x] 1.1 In `apps/kuru-tui/tests/support/memory.rs`, parse `private diagnostics: <path>` from the owner diagnostic and append a labelled tail of that file bounded by an existing tail constant (derivation cited at the site) only when the path is strictly inside the fixture root; verify the appended text names the path and ends with the file's last bytes
  - Evidence: `ServiceCleanup::owner_diagnostic` keeps `owner stderr (<file>):\n<text>` unchanged and appends `named_private_diagnostics(root, text)`. The root is the owner diagnostic file's parent, not `self.root`, which a failed cleanup has already taken (root kept on disk).
    - Parse: each line's occurrences of the exact producer prefix `Dolt startup/lifetime failed; private diagnostics: ` (`packages/kuru-memory/src/server.rs`), taken through the first `server.log` that ends the line or is followed by `:`, so a Windows drive colon cannot end the path. Distinct paths only, one `named private diagnostics (<path>): …` line each.
    - In-root rule: absolute, `strip_prefix(root)` with a non-empty remainder of `Normal` components only (rejects `..`, `.`), and `canonicalize(path)` strictly beneath `canonicalize(root)` (rejects a symlinked escape).
    - Read and bound: `kuru_memory::test_support::fixture_server_log`, made `pub` with its body unchanged (the one file touched outside the fixture). It is the checked private read capped at `STARTUP_LOG_BYTES` and cut to the last `STARTUP_TAIL_BYTES`, cited at the use site against the server's own `LOG_LIMIT` on `server.log`; no new literal.
    - `an_in_root_log_tail_is_appended_with_its_label` and `the_owner_report_appends_the_named_log_after_owner_stderr` assert the exact appended text, labelled with the path; `only_the_bounded_tail_of_a_large_log_is_appended` asserts it ends with the file's last bytes (`xEND`).
- [x] 1.2 Report an out-of-root path as not read and a missing or unreadable file as such, never panicking; verify with the tests in 2.1
  - Evidence: an out-of-root path reports `not read: outside the fixture root <root>` or `not read: resolves outside the fixture root <canonical root> to <canonical path>`; a missing file reports `not read: missing or unresolvable: <io error>`; a refused checked read reports `not read: the checked private read refused it (…)`. The diagnostic path has no `expect`, `unwrap` or indexing beyond `match_indices` offsets.

## 2. Test

- [x] 2.1 Pin in a kuru-tui test: tail appended for an in-root file, bound respected, out-of-root path not read, missing file reported; verify each fails with its behaviour reverted and passes with it
  - Evidence: six tests in the fixture's own `mod tests` (compiled into every kuru-tui binary that includes `support/memory.rs`; engine-free, files created through `kuru_platform::fs::Directory` private primitives so the checked read accepts them on Windows):
    - `an_in_root_log_tail_is_appended_with_its_label` (also: a repeated mention appends once);
    - `only_the_bounded_tail_of_a_large_log_is_appended` (8 KiB + markers: `END` present, `BEGIN` absent, tail at most half the file);
    - `a_log_outside_the_root_is_reported_and_not_read` (absolute path in a second fixture root and a `<root>/../../<other>/private/stage/server.log` escape: sentinel absent, reported; a literal `C:\fixture\stage\server.log` reaches the report whole);
    - `a_symlinked_escape_is_reported_and_not_read` (`#[cfg(unix)]`);
    - `a_missing_in_root_log_is_reported`;
    - `the_owner_report_appends_the_named_log_after_owner_stderr` (end to end through `ServiceCleanup::new`/`owner_diagnostic`/`release`).
  - Pass, local macOS: `mise run //apps/kuru-tui:test -- memory::tests` exit 0; 6 passed in each of cli, lease, preferences, server, terminal, trust and unix_shell_turn.
  - Reverted-behaviour runs (local, not committed; `cargo test -p kuru --test lease -- memory::tests`, each exit 101, the file restored byte-identical after each):
    - append removed from `owner_diagnostic`: the end-to-end test FAILED, others ok;
    - both containment checks removed: the outside and symlink tests FAILED;
    - lexical check removed: the outside test FAILED (the `..` escape no longer reports `outside the fixture root`);
    - canonical check removed: the symlink test FAILED;
    - helper replaced by an unbounded `std::fs::read`: the bound test FAILED;
    - missing file reported as empty: the missing test FAILED.
  - Not run locally: the Windows build of these tests (no Windows host); `lint:windows` compiles them for `x86_64-pc-windows-msvc`. Native Windows execution is CI-only.

## 3. Checks

- [x] 3.1 Run `mise run //apps/kuru-tui:test`, `mise run format:check`, `mise run lint` and `mise run cospec -- validate --all --strict`, and verify each exits 0; the next CI occurrence of "Dolt exited before readiness" printing Dolt's reason is observed later and is not a gate
  - Evidence, local macOS, each exit 0:
    - `mise run //apps/kuru-tui:test`: every kuru-tui target ok, none failed (lib 137 passed; cli 51 passed, 1 ignored; terminal 50 passed, 1 ignored; trust 28; unix_shell_turn 11; lease 10; preferences 9; server 7; embedded_runtime 7; visual 12, 1 ignored; others as on main).
    - `mise run format:check`, `mise run lint` (every package's clippy `-D warnings`, shell and docs lint), `mise run typecheck`.
    - `mise run //apps/kuru-tui:lint:windows` and `mise run //packages/kuru-memory:lint:windows` (clippy for `x86_64-pc-windows-msvc`, compiling the new tests' Windows build).
    - `mise run //packages/kuru-memory:test -- test_support`: 77 passed, including `fixture_diagnostic_tests::startup_log_capture_is_opt_in_exact_and_bounded`, which pins `fixture_server_log`'s unchanged behaviour.
    - `mise run cospec -- validate fixture-names-dolt-exit-reason --strict` and `mise run cospec -- validate --all --strict`.
  - Not run: native Windows and Linux execution and coverage (CI-only). Open, not a gate: whether a real failing owner's `server.log` still exists when the fixture reports is unobserved; the owner's own startup cleanup may remove the staging directory first, in which case the report says `missing or unresolvable`. The next CI occurrence will show which.
