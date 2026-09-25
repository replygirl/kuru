## 1. CI and release builds run without mbx [critical]

- [x] 1.1 @runtime (agent) with mbx uninstalled, run `KURU_MBX=0 MISE_LOCKED=1 MISE_TASK_RUN_AUTO_INSTALL=false mise run format:rust` and the equivalent `mise exec -- cargo` -> plain rustup Cargo ran and mbx was neither installed nor invoked; without `KURU_MBX=0` the same run force-installed mbx, so the workflow setting is required
- [x] 1.2 @integration (agent) `mise lock` on mise 2026.9.13 and a repeat run -> nine signed mr-boxington entries, none for `macos-x64`, `lockfile_version = 1` kept for CI's mise 2026.9.4, repeat run byte-identical
- [x] 1.3 @integration (agent) read mise v2026.9.4 source for `add_rust_wrapper`, tool-option templating and `os_selector_matches` -> all present at the `min_version` floor, so the Intel macOS exclusion and template apply there
- [~] 1.4 @runtime (human) native ubuntu, ubuntu-arm, macos-14, macos-15-intel and windows-2025 jobs on the pushed branch -> defer: branch is intentionally unpushed; the exact-head CI matrix owns this
- [~] 1.5 @runtime (human) Intel macOS mise skips mr-boxington and runs Cargo unwrapped -> defer: no local Intel host; verified from mise source only, macos-15-intel CI owns execution

## 2. Source installation and update stay outside the cache [critical]

- [x] 2.1 @integration (agent) run `scripts/install.sh --source` against a recording fake `mise` -> all three mise invocations received `KURU_MBX=0`
- [~] 2.2 @runtime (human) Windows `kuru update --source` (`apps/kuru-tui/src/cli.rs` `build_windows_source`), `scripts/install.ps1` scoping and restoration, and the `windows_cli` fixture assertions -> defer: Windows-only code was not compiled or run locally; native Windows CI owns it

## 3. Maintainer builds share the cache

- [x] 3.1 @e2e (agent) `mise run //packages/kuru-core:test` cold, in a second worktree, and after `mbx clean` with a real `target/` -> 57.9 s cold with 520.7 MiB stored; 7.5 s and 4.8 s warm with 181 hits and 0 misses
- [x] 3.2 @e2e (agent) `mise run //packages/kuru-memory:test` through mbx with `MBX_TARGET_VIEWS=0` -> engine preparation, supervisor prefetch and 291 tests passed in 960 s; with managed views it failed "open bundle cache directory: Not a directory"
- [x] 3.3 @integration (agent) cargo-llvm-cov 0.9.1 on `kuru-core` through the wrapper -> mbx reported 0 hits, 0 stored and 166 bypassed, deferring to cargo-llvm-cov's `RUSTC_WRAPPER`
- [~] 3.4 @integration (agent) hard-link restore on a filesystem without cloning (ext4) against packaging and snapshot tests -> defer: only APFS clones were exercised locally; behavior is documented from mbx's own documentation
