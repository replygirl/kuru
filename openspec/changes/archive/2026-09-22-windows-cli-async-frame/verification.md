## 1. Instrumented Windows CLI startup remains functional [critical]

- [~] 1.1 @regression (agent) run the existing native Windows application CLI and terminal coverage cases on the failing #54/#56 heads and a dependent head with this fix -> defer: pre-fix `0xc00000fd` is recorded below; post-fix dependent integration requires the committed fix to be pushed and exercised by native CI. Delivery owns that exact-head check and will record observed pass or failure in the PR.
- [~] 1.2 @e2e (agent) run the exact fix PR's normal native Windows application matrix -> defer: native Windows CI runs only after this archived change is committed and pushed; delivery will report the exact result in the PR, including unrelated failures separately.

## 2. Dispatch and local behavior stay equivalent

- [x] 2.1 @equivalence (agent) compare the only production diff with current main and the dependent service branch -> all argument checks, internal helper dispatch, awaits, returned errors, and process cleanup retain their original order; no diagnostic marker or stack-size override ships.
- [x] 2.2 @integration (agent) run focused owning TUI CLI and PTY cases with verified bundled Dolt inputs, plus owning lint/typecheck/format and strict Cospec/diff checks -> affected child-process behavior and static checks pass on the fix source; normal pre-push coverage separately exercises the full suite and packaged runtime.

## Observed evidence before implementation

- #54 Windows application job `106645554330` and #56 corrected-head Windows application job `106653771343` reported `0xc00000fd` in spawned `kuru` CLI children. Existing shell fixture stage files reached completion before those failures.
- #58 diagnostic head `bf8c629` produced many more child overflows before its earliest Rust marker after it bound and measured the inline `execute_inner` future. Those failures are layout-sensitive diagnostic evidence, not a claim of additional independent product defects.
- #58 macOS coverage separately failed a known memory test's `WouldBlock` lock reacquire; that result does not establish this Windows fix.

## Observed local evidence on the fix source

- A temporary, test-only host layout probe measured `dispatch()` at 18,024 bytes on aarch64 macOS and its boxed future handle at 8 bytes. The probe was removed; these host sizes are layout evidence, not a Windows stack measurement or native fix proof.
- With `KURU_DOLT_BUNDLE_DIR=/private/tmp/kuru-phase1-bundles`, `KURU_DOLT_BUNDLE_OFFLINE=true`, and `CARGO_NET_OFFLINE=true`, `mise run //apps/kuru-tui:typecheck` and `mise run //apps/kuru-tui:lint` both exited 0. The bundle preparer reused the verified archive.
- Focused `cargo test -p kuru --test cli --all-features --locked cli_file_crud_and_shell_require_real_capabilities -- --exact --nocapture` passed 1/1; `cargo test -p kuru --test terminal --all-features --locked real_pty_accepts_chat_navigation_commands_and_restores_terminal -- --exact --nocapture` passed 1/1, each via `mise exec` with the same verified offline bundle environment.
- `mise run format:check`, strict Cospec validation and actual apply gate exited 0; `git diff --check` passed. The final `main.rs` production diff has only the Tokio entry's heap-pinned dispatch boundary and unchanged dispatch body.
- Native Windows behavior on the fix head and on the previously failing dependent heads, plus normal pre-push coverage and packaged-runtime checks, remain pending delivery/CI. No local host result establishes that failure is cured.
