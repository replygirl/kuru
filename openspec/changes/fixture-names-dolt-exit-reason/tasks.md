# Tasks

## 1. Name Dolt's reason in the fixture's failure output

- [ ] 1.1 In `apps/kuru-tui/tests/support/memory.rs`, parse `private diagnostics: <path>` from the owner diagnostic and append a labelled tail of that file bounded by an existing tail constant (derivation cited at the site) only when the path is strictly inside the fixture root; verify the appended text names the path and ends with the file's last bytes
- [ ] 1.2 Report an out-of-root path as not read and a missing or unreadable file as such, never panicking; verify with the tests in 2.1

## 2. Test

- [ ] 2.1 Pin in a kuru-tui test: tail appended for an in-root file, bound respected, out-of-root path not read, missing file reported; verify each fails with its behaviour reverted and passes with it

## 3. Checks

- [ ] 3.1 Run `mise run //apps/kuru-tui:test`, `mise run format:check`, `mise run lint` and `mise run cospec -- validate --all --strict`, and verify each exits 0; the next CI occurrence of "Dolt exited before readiness" printing Dolt's reason is observed later and is not a gate
