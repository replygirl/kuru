## 1. rust-cache saves only from main [critical]

- [x] 1.1 @runtime (agent) inspect this PR's own quality/native-tests/native-build/native-platform rust-cache post-step logs for the touched keys (run [36280323517](https://github.com/replygirl/kuru/actions/runs/36280323517)) -> observed: every restricted job (`quality / Lint`, `native-build (ubuntu-24.04-arm, aarch64-unknown-linux-gnu)`, etc.) logs `save-if: false` on this non-main ref and only restores (e.g. `quality / Lint`: `... Restoring cache ... No cache found.`); before this push's fix, `native-platform-windows` (Windows native-platform primitives) logged `save-if: true` unconditionally and its Post step attempted a real save that hit `Cache reservation failed: You have reached your configured budget, your cache is now read only to prevent additional charges.` — it now carries the same main-ref condition as the other coverage keys, so that Post-step save no longer runs on PR refs
- [~] 1.2 @runtime (agent) inspect the next push to main for the same rust-cache post-step logs -> defer: not yet run; expect "Cache saved successfully" for each touched key. Note: the same run's cache-save attempts (including the mise tool cache) hit the same "configured budget ... read only" reservation failure repo-wide, so the "PRs restore main's warm caches" effect cannot be observed until that budget is raised or resets (a maintainer/billing action, not a code change here)

## 2. Prebuilt cargo-llvm-cov resolves and behaves identically

- [x] 2.1 @integration (agent) run mise install/mise lock for aqua:taiki-e/cargo-llvm-cov@0.9.1 -> observed: 2.8s download of cargo-llvm-cov-aarch64-apple-darwin.tar.gz (no compile), mise.lock gained checksummed entries for linux-x64, linux-arm64 (+musl), macos-arm64, macos-x64, windows-x64, and the stale cargo:cargo-llvm-cov lock entry was removed by mise lock
- [x] 2.2 @integration (agent) run the pinned binary's version check -> observed: cargo-llvm-cov llvm-cov --version printed "cargo-llvm-cov 0.9.1", matching LLVM_COV_VERSION and the version checks in kuru-delivery's mise.toml and windows-coverage.ps1
- [~] 2.3 @runtime (agent) compare cargo-llvm-cov install time in this PR's cold native-tests jobs before/after -> defer: not yet run; expect a download instead of the previously observed 53s (Linux) / 1m41-1m45 (Windows) / 1m24 (macOS) source compile

## 3. Static and delivery checks stay green

- [x] 3.1 @integration (agent) run mise run format:code ::: lint:rust ::: typecheck ::: lint:tooling ::: cospec:validate ::: cospec:managed:check ::: docs:check -> observed: all exited 0 (format:code, typecheck, lint:tooling, lint:rust, cospec:validate 0 errors 0 warnings, cospec:managed:check "no drift", docs:check "Public docs artifacts, local links and anchors passed")
- [x] 3.2 @integration (agent) run mise run //packages/kuru-delivery:test -> observed: all suites passed including native_workflow_shards_only_windows_and_keeps_the_aggregate_fail_closed (73+22+9+1+8+13+5+13+20+11+3+3 = 181 passed, 0 failed, 1 ignored across suites)

## 4. Cache usage baseline

- [x] 4.1 @integration (agent) `gh api repos/replygirl/kuru/actions/cache/usage` before merge -> observed: `active_caches_size_in_bytes: 11691126942` (11.69 GB), `active_caches_count: 25`, still over the 10 GB default limit. The after number is measured post-merge by a later orchestrator PR, not here.
