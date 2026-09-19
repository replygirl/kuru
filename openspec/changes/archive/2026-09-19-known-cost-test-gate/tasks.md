## 1. Wire the test-support seam into the plain test build

- [x] 1.1 Add `kuru-core = { workspace = true, features = ["test-support"] }` to `apps/kuru-tui/Cargo.toml`'s `[dev-dependencies]` and verify `cargo test -p kuru --test terminal -- real_pty_status_bar_renders_known_cost` passes with no `--all-features` (previously failed with "not applied: invocation price").
- [x] 1.2 Verify the shipping `kuru` binary's resolved features are unchanged and verify with `cargo tree -e features -p kuru --no-dev-dependencies | grep -c test-support` returning `0`.
- [x] 1.3 Verify the CI-equivalent mise task still runs and passes the test and verify with `mise run //apps/kuru-tui:test -- real_pty_status_bar`.
- [x] 1.4 Verify formatting and lint are unaffected and verify with `mise run format:check` and `mise run //apps/kuru-tui:lint`.
