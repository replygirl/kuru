# Tasks

## 1. Survey

- [ ] 1.1 Re-read each site in `accounting_tests.rs` (~3543, ~3596, ~3644) and `dream.rs` (~790, ~1211) and record, per site, what it waits on, whether cancellation precedes it, and which budget bounds the product teardown it encloses; verify by a per-site list in the PR description, including any site left unchanged and why

## 2. Event-driven joins

- [ ] 2.1 Replace each in-scope flat 10 s post-cancel join with `progress_wait::until_event` on the cancelled task finishing, under a gap bound derived from stated budgets and strictly above any product bound it encloses; verify by reading the diff for no remaining flat post-cancel join in these files and by a unit assertion of the strict inequality where the bound is derived
- [ ] 2.2 Give each site the #181 expiry diagnostics (step timings, observed events, in-flight hooks, task state), reusing shared test support rather than copies; verify by a temporary forced stall at one site showing the panic text (record, do not commit)
- [ ] 2.3 Leave every assertion after each join byte-identical; verify by diff

## 3. Regression

- [ ] 3.1 Paused-clock test: a cancelled task whose teardown keeps progressing for more than 10 s; the old flat shape fails it and the new wait accepts it; a stalled teardown fails naming its last step; verify by running it and by temporarily swapping the old shape in and observing the failure (record, do not commit)

## 4. Checks

- [ ] 4.1 Run `mise run //packages/kuru-runtime:test`, `:lint`, `:lint:windows`, `mise run format:check` and record observed results and pass counts; state that local runs do not reproduce the loaded CI runner
- [ ] 4.2 Validate with `mise run cospec -- validate runtime-post-cancel-joins --strict`, apply gate clear, and archive before the final commit
