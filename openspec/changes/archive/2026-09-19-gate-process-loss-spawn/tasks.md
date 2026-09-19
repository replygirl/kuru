## 1. Gate the missed process-loss spawn

- [x] 1.1 Take `crate::spawn_gate::spawning()` across the `Command::spawn()`
      call in the `#[cfg(unix)]` `spawn_process_loss_creator` in
      `packages/kuru-memory/src/store/recovery_tests.rs`, matching the
      "held across the spawn; see `crate::spawn_gate`" pattern used at every
      other raw spawn site this gate covers; verify with `grep -n
      "spawn_gate" packages/kuru-memory/src/store/recovery_tests.rs` that the
      site is now covered. Done: `grep -c "spawn_gate::spawning"` on the file
      now returns 2.
- [x] 1.2 Take `crate::spawn_gate::spawning()` across the
      `NativeSpawnSpec::spawn().await` call in the `#[cfg(windows)]`
      `spawn_process_loss_creator` in the same file, for parity with the
      Unix branch; verify the change compiles under `--cfg windows` review
      (no Windows CI host available; inspect the diff against the Unix
      branch for the same guard placement). Done: guard placement mirrors
      the Unix branch exactly (same comment, same binding, immediately
      before the spawn call).
- [x] 1.3 Verify no assertion, timeout, retry or `#[ignore]` was introduced
      anywhere in `recovery_tests.rs`; the diff touches only the two
      `spawn_process_loss_creator` bodies. Done: `git diff --stat` shows one
      file, four added lines, no removed lines.
- [x] 1.4 Run `mise run format:check`, `mise run //packages/kuru-memory:lint`,
      `mise run typecheck`, then run
      `cargo test -p kuru-memory --lib --all-features -- store::recovery_tests::`
      and
      `cargo test -p kuru-memory --lib --all-features -- server::tests::supervisor_rejects_bad_configuration_and_parent_eof_without_spawning`
      three consecutive times each; record observed pass/fail evidence for
      every run. Done: all four static checks pass; `store::recovery_tests::`
      19/19 three times (23.33s, 30.13s, 20.77s); the flake target run
      together with `store::recovery_tests::` 20/20 three times (24.35s,
      25.02s, 22.97s), the named test `... ok` in every run.
- [x] 1.5 Run `mise run cospec -- validate gate-process-loss-spawn --strict`
      and `mise run cospec -- archive gate-process-loss-spawn` before the
      final commit on this branch.
