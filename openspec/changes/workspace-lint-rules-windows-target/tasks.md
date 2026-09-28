# Tasks

## 1. Shared lint configuration and the tracing ban

- [x] 1.1 Add root `clippy.toml` with `disallowed-methods` entries for `tracing::subscriber::{set_default, with_default, set_global_default}`, `tracing::dispatcher::{set_default, with_default, set_global_default}` and `tracing_subscriber::util::SubscriberInitExt::{set_default, try_init, init}`, each with a reason naming the safe alternative, and verify that no crate reports an unreachable path
- [x] 1.2 Give every existing call site its own statement-level `#[expect(clippy::disallowed_methods, reason = "...")]`, and verify that `mise run lint` passes with no crate- or module-level allowance
- [x] 1.3 Regression: reintroduce a thread-scoped `set_default` in a scratch copy, and verify that the package lint command (`cargo clippy -p kuru-memory -p kuru --all-targets --all-features --locked -- -D warnings`) fails with `disallowed_methods` while it is present and passes once it is reverted

## 2. Repository check

- [x] 2.1 Add the `kuru-delivery` repository rules and tests. They reject a package-level `clippy.toml`, a crate- or module-level allowance of the lint or of its groups, a `[lints]` table that lowers it, `CLIPPY_CONF_DIR` or `-A` in mise or workflow configuration, and a root entry without a reason. Verify with `mise run //packages/kuru-delivery:test`
- [x] 2.2 Regression: add a package-level `clippy.toml` in a scratch copy, and verify that `mise run //packages/kuru-delivery:check:repo` (part of `lint:tooling`) fails before removal and passes after

## 3. Windows-target lint

- [x] 3.1 Add package-owned `lint:windows` tasks for `kuru-memory`, `kuru-delivery` and `apps/kuru-tui`, the delivery-owned stand-in C toolchain, `lint:windows` tasks for `kuru-connectors` and `kuru-runtime` (design decision 5), and a root `lint:windows` aggregate that includes `kuru-platform`
- [x] 3.2 Regression: record the `origin/main` Windows-target findings, fix each individually, and verify that root `mise run lint:windows` exits 0
- [x] 3.3 Add the `ci.yml` job `Lint (x86_64-pc-windows-msvc)` with offline bundle inputs and require it in `ci-gate`. Verify with `lint:tooling` and the release workflow tests

## 4. Instructions, docs and gates

- [x] 4.1 Amend AGENTS.md (root workspace role, Windows-target lint) and `docs/development.md`, and verify with `docs:check`
- [x] 4.2 Run the root gates: `lint`, `typecheck`, `format:check`, `lint:tooling`, `docs:check`, `cospec validate --strict` and `cospec:managed:check`. Also run the package tests whose test code changed. Record the results in verification.md
