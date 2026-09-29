# Verification

Local host: macOS arm64, 2026-09-28, the pinned Rust 1.98.1 toolchain. Every
mise call ran with `MISE_LOCKED=1` and
`MISE_CEILING_PATHS=<repo>/tmp/worktrees`. Logs are in the session
scratchpad at `flake/lint-rules/`.

## 1. Thread-scoped and ad hoc global subscriber installation is rejected [critical]

- [x] 1.1 @regression (agent) A scratch edit added `let _guard = tracing::subscriber::set_default(NoSubscriber::default());` to a kuru-memory test, and `registry().set_default()` and `registry().try_init()` to a kuru-tui test. Command: `cargo clippy -p kuru-memory -p kuru --all-targets --all-features --locked -- -D warnings` -> exit 101, with `error: use of a disallowed method` for `tracing::subscriber::set_default` (native_tests.rs), `SubscriberInitExt::set_default` and `SubscriberInitExt::try_init` (diagnostics.rs), each note carrying the configured reason (`red-ban.log`). The edits were reverted.
- [x] 1.2 @integration (agent) Before any expectation was added, host clippy found exactly 10 sites: `diagnostics.rs` 65, 505, 532, 574, 649, 684, 701 and 738; `auth.rs` 275; `native_tests.rs` 160. There was no unreachable-path warning (`host-clippy-ban.log`). After the statement-level expectations, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` -> exit 0 with no unfulfilled expectation (`host-clippy-expect.log`), and `mise run lint` -> exit 0.

## 2. Nothing can silently switch the ban off

- [x] 2.1 @regression (agent) A scratch `packages/kuru-core/clippy.toml` made `mise run //packages/kuru-delivery:check:repo` exit 1 with `packages/kuru-core/clippy.toml: Clippy would use this file instead of the root clippy.toml ...` (`red-repo-check.log`). After removing the file -> exit 0, `Repository metadata invariants passed.` (`green-repo-check.log`).
- [x] 2.2 @unit (agent) `mise run //packages/kuru-delivery:test` -> exit 0, including 5 `repo::lints` unit tests, the five new `repo_validation` rule tests, the actual-repository test, and `windows_only_rust_is_linted_by_a_required_static_job` (`delivery-test.log`).

## 3. Windows-only code is linted [critical]

- [x] 3.1 @regression (agent) Cross-target clippy of `origin/main` 686eb16b for every workspace package with the stand-in toolchain, without `-D warnings` -> 25 distinct warnings that `-D warnings` rejects (`red-windows-main.log`): 8 in kuru-memory, 5 in kuru-delivery, 4 in apps/kuru-tui, 7 in kuru-connectors, 1 in kuru-runtime, 0 in kuru-platform. On the final tree, `mise run lint:windows` -> exit 0 (`green-lint-windows.log`). That run covered the tasks of all six packages, with `KURU_MBX=0` and a scratch `CARGO_TARGET_DIR`; it took 64 s with warm dependencies.
- [~] 3.2 @runtime (agent) on the PR run in CI (ubuntu-latest), `Lint (x86_64-pc-windows-msvc)` passes with offline bundle inputs and `ci-gate` requires it -> defer: this change has not yet run in CI; it will be observed on the final-head PR run before merge.

## 4. Root gates

- [x] 4.1 @integration (agent) Each of these -> exit 0: `format:check`, `lint`, `typecheck`, `lint:tooling` (including actionlint of the new job), `docs:check`, `cospec validate workspace-lint-rules-windows-target --strict` and `cospec:managed:check`. Package tests, each exit 0: `//packages/kuru-memory:test`: 318 passed.; `//packages/kuru-connectors:test`: 297 passed.; `//packages/kuru-runtime:test`: 220 passed.; `//apps/kuru-tui:test`: 245 passed, 4 ignored; these are the existing; ignored live tests. `//packages/kuru-delivery:test`: exit 0.; `mise.lock` was unchanged after each gate.
- [~] 4.2 @runtime (agent) Windows behavior of the three edited `cfg(windows)` bodies -> defer: this change has not yet run in CI; the native-tests Windows jobs will observe them on the final-head PR run before merge. They are the retry-deadline `take_if` in `provision.rs`, the tail block in `files.rs` `close`, and the fixture wait in `tools.rs`. The cross-target clippy above type-checks them.

## 5. Review round: tightened rules (2026-09-29)

Logs are in `flake/lint-rules/fix/`. The rules were tightened; no claim was
narrowed.

- [x] 5.1 @regression (agent) Each bypass was added to a copy of the pre-round tree (010eed5f) and checked with `kuru-delivery repo --root <copy>`, using the pre-round binary and then the new one (`run-cases.sh`). Cases: (a) `RUSTFLAGS=-Aclippy::disallowed_methods cargo clippy` inside a package `mise.toml` `run` string; (b) `packages/kuru-core/.cargo/config.toml` `[build] rustflags = ["-Aclippy::disallowed_methods"]`; (c) `apps/.cargo/config` with `rustflags = ["--allow", "clippy::style"]`; (d) the review's wrapper, `#[allow(clippy::disallowed_methods)]` on a `fn` that calls `with_default`; (e) the same wrapper under a reasoned `#[expect]`; (f) reasoned `expect`s on an `impl` and an `unsafe trait`; (g) a statement-level `#[allow(clippy::disallowed_methods, reason = "...")]`; (h) a statement-level `#[expect(clippy::disallowed_methods)]` with no reason -> the unmodified copy exits 0 with both binaries; every case exits 0 before and exits 1 after, naming its file, line and rule (`red-green.log`).
- [x] 5.2 @unit (agent) `mise run //packages/kuru-delivery:test` -> exit 0: lib 189 passed, including 6 `repo::lints` tests; `repo_validation` 28 passed, including the 2 new rule tests and the actual-repository test; 1 existing ignored test. `//packages/kuru-delivery:lint` and `:typecheck` -> exit 0.
- [x] 5.3 @integration (agent) Root `mise run lint` -> exit 0 (`lint.log`). Its first attempt failed on the cmux `NODE_OPTIONS` preload, and the second on two of this round's own `clippy::question_mark` findings, since fixed. Root `mise run lint:windows` -> exit 0 for all six package tasks (`lint-windows.log`); the stand-ins built `aws-lc-sys` and `libsqlite3-sys`. No existing call site needed a changed attribute: every guarded allowance was already a statement-level `expect` with a reason.
- [x] 5.4 @integration (agent) Each -> exit 0: `lint:tooling`, `format:check`, `docs:check`, `cospec validate workspace-lint-rules-windows-target --strict` and `cospec:managed:check` (`gate*.log`). `mise.lock` is unchanged versus `origin/main`.

## 6. First CI run (PR #131, 2026-09-29): two failing checks, both unrelated to this change

7 of 67 checks failed on the first run at head `7aa6cb9f8d14056cbfe342e96e4989437586e52a`
(before this rebase; confirmed via `gh api repos/replygirl/kuru/actions/jobs/<job-id>
--jq .head_sha` on both leaf jobs below). 5 were pure dependency/gate failures downstream of the
2 leaves below, with no work of their own (`Coverage merge (macos-latest)`, `Coverage merge
(windows-latest)`, `Require native coverage and installation checks` x2, `ci-gate`). Read-only
triage (session scratchpad `flake/ci/triage-0929/triage.md`) confirmed both leaves by diffing
this PR's changed files against each failing test's code path: the failing tests' own files
(`apps/kuru-tui/tests/embedded_runtime.rs`, and `kuru-memory`'s `facade.rs`/`store.rs`/dream-write
path) are absent from this PR's diff. This PR's only touches inside `packages/kuru-memory` are
cfg-attribute edits (`mise.toml`, `files.rs`, `provision.rs`, `provision/native_tests.rs`,
`server.rs`, `spawn_gate.rs`) plus one behavior-preserving `take_if` refactor of `provision.rs`'s
retry loop, none on either failing test's code path; the PR's own
`apps/kuru-tui/tests/cli.rs` hunk only moves an import behind `#[cfg(unix)]`, untouched at the
panic site. (The bulk of this PR's diff — `kuru-delivery/src/repo/lints.rs`, its registration in
`repo.rs`, `ci.yml`, `docs/development.md`, `AGENTS.md` — is unrelated to memory or install
paths entirely.)

- **Leaf 1**: `native-tests (macos-latest) / Coverage partition (macos-latest, 2)`, job
  109469266473. Test `cli_supports_all_modes_model_discovery_persistent_sessions_and_dreaming`
  (`apps/kuru-tui/tests/cli.rs:83`) panics with "memory service write outcome is uncertain;
  further client mutations are blocked". Not caused by this PR: the catalogued uncertain-write /
  Dolt lifecycle-ordering family (class B, F2), the same family `fix/memory-lifecycle-ordering`
  (PR #125) targets at its root cause.
- **Leaf 2**: `native-tests (windows-latest) / Coverage partition (windows-latest, 2)`, job
  109469267257. Test `packaged_install_and_update_preserve_complete_offline_memory`
  (`apps/kuru-tui/tests/embedded_runtime.rs:1526`) times out reading native stdout/stderr after
  100 s with no output. Not caused by this PR: `embedded_runtime.rs` is absent from this PR's
  diff; this is the catalogued "install timeout" / unmodelled-host-stall family (flaky-tests.md
  catalogue item 13, and the same shape recurring elsewhere in this triage round).

Full per-job evidence, log line citations and the file-by-file diff check are in
`flake/ci/triage-0929/triage.md`.

- [~] 6.1 @runtime (agent) -> defer: The rebased head (onto `origin/main` 8450c555, after merging main's
  release-workflow-tools check and this branch's lint rules) has not yet had its own CI run.
  Before merge, confirm on that run: no new leaf failures, and that only the two known-unrelated
  families above (or their tracked recurrences) may still appear as flakes.
