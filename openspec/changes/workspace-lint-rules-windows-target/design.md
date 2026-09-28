# Design

## Context

- **Root cause 1, the observer.** tracing-core 0.1.36 caches each callsite's
  interest for the whole process.
  - While at most one dispatcher is registered, `Dispatchers::rebuilder`
    returns `JustOne`. `JustOne` asks `dispatcher::get_default` on the
    registering thread.
  - A thread-scoped subscriber set with `set_default` therefore cannot see a
    callsite that another test's thread, holding no subscriber, registered
    first. That thread cached `never`.
  - PR #126's first capture failed this way. Its fix is the process-wide
    recorder in `kuru-memory`'s `provision/native_tests.rs`.
  - Nothing prevents the pattern from being written again.
- **Root cause 2, the invisible code.**
  - `.github/workflows/quality.yml`'s Lint job runs only on `ubuntu-latest`,
    and rustc does not compile `cfg(windows)` items there.
  - `kuru-platform` has `lint:windows` tasks, but no workflow runs them.
  - A cross-target run of `origin/main` with the stand-in toolchain finds 25
    Windows-only warnings: 17 in the three named packages, 7 in
    `kuru-connectors` and 1 in `kuru-runtime`. `-D warnings` would reject
    each one.
- **Clippy 1.98.1 facts** (book `configuration.md` and `clippy_config`
  `lookup_conf_file` at tag 1.98.1; lint docs of `disallowed_methods` and
  `clippy_config/src/types.rs`):
  - Lookup starts at `CLIPPY_CONF_DIR`, else `CARGO_MANIFEST_DIR`, and walks up.
    The first directory holding `clippy.toml` or `.clippy.toml` wins, so a
    package-level file shadows the root file completely.
  - `disallowed_methods` is one lint in the `style` group. Any allowance of it,
    or of `style` or `all`, turns off every configured ban at once.
  - An entry whose path does not resolve warns, unless its crate is not loaded.
    `allow-invalid = true` silences that warning.

## Goals / Non-Goals

**Goals:**
- The ban lands with every existing site reviewed individually.
- The configuration cannot be shadowed or blanket-disabled without the
  repository check failing.
- Windows-only Rust is linted in CI in every package that has it.

**Non-Goals:**
- Further bans: tempfile roots, file locks, detached threads.
- `release.yml`.
- Windows on Arm.

## Decisions

1. **Statement-level `#[expect]` per call site.** A helper that wraps
   `with_default` was rejected: every future caller would inherit its
   exemption unreviewed. A site counts as exposed when its captured callsite
   can be registered first by a thread holding no subscriber. The
   `apps/kuru-tui` diagnostics tests emit only from callsites inside their own
   closure, on the capturing thread. Each new `Dispatch` also rebuilds every
   interest. So they are not exposed and keep scoped capture, with reasons.
   - `diagnostics.rs:65` is the application's one startup installation.
   - `auth.rs:275` propagates the caller's existing dispatch into its worker
     thread; it installs no new subscriber.
   - The memory recorder is the process-wide pattern itself.
2. **`SubscriberInitExt::try_init` and `init`**, not a nonexistent
   `SubscriberInitExt::set_global_default`, are the global installers in
   tracing-subscriber 0.3.23. `fmt` paths are omitted because the workspace
   does not enable that feature.
3. **Cross-target clippy on `ubuntu-latest` rather than a native Windows job.**
   Cross-target works for these crates:
   - With a stand-in C compiler and archiver for the MSVC target, every build
     script completes: `aws-lc-sys`, `ring`, `libsqlite3-sys`, and
     `kuru-memory`'s engine selection by `TARGET`.
   - Clippy never links, so no compiled C is consumed.
   - The warm local run took 25 s.

   Reasons to choose it:
   - AGENTS.md gives static categories separate Ubuntu jobs.
   - `kuru-platform`'s `lint:windows` sets the cross-target precedent.
   - Maintainers can reproduce the check on macOS and Linux.
   - On main run 36476184971, Ubuntu Lint took 2m01s. The Windows jobs took
     11m44s to 14m06s, so a Linux job stays off the critical path.

   A native job would compile the same crates a second time on a runner that
   costs twice as much per minute. It would find no additional Rust
   diagnostics: Rust lints depend only on `TARGET`.
4. **The stand-in toolchain is owned by `kuru-delivery/support`** and used only
   by `lint:windows` tasks on non-Windows hosts. A Windows host uses its native
   MSVC toolchain through `run_windows`.
5. **`kuru-connectors` and `kuru-runtime` are included.**
   - `cargo clippy -p kuru -- -D warnings` also applies `-D warnings` to the
     library targets of the TUI's path dependencies. That made connectors'
     five library findings blocking.
   - Owning their own tasks adds only the three test-target findings.
   - A repository test requires every package with `cfg(windows)` code to own
     `lint:windows`.
6. **The CI job lives in `ci.yml`, not `quality.yml`.** `release.yml` also calls
   `quality.yml`, and it has no bundle-inputs artifact. The job needs
   `bundle-inputs` and imports `ci-bundle-inputs-ubuntu-latest`, which already
   carries `x86_64-pc-windows-msvc.archive`. It uses `--archive --offline`
   under `KURU_DOLT_BUNDLE_OFFLINE`, as the partitions do.

## Risks / Trade-offs

- [A future build script consumes compiled C output, or runs a compiled probe]
  → The Windows lint job fails loudly with the build script's error. Nothing
  passes falsely. Replace the stand-in or move the job to a native runner then.
- [The stand-in reports every C flag as supported] → The only effect is on C
  flag probes. Rust `cfg`s come from rustc, not cc.
- [`release.yml` does not run the Windows lint] → The release workflow runs
  from `main`, where every merged commit already passed `ci-gate`.
- [The repository check reads text] → It cannot see an allowance produced by a
  macro, or a lint level passed through `RUSTFLAGS` from outside the
  repository.

## Operational surface

- Runner: `ubuntu-latest` (x64), job `windows-lint` named
  `Lint (x86_64-pc-windows-msvc)`, `needs: bundle-inputs`, required by
  `ci-gate`.
- Tooling: pinned Rust 1.98.1 through mise-action (`install_args: rust`),
  `rustup target add x86_64-pc-windows-msvc` and `rustup component add clippy`.
- Secrets: only `github.token` for mise-action.
- Network: none for engine inputs. `KURU_DOLT_BUNDLE_OFFLINE: "true"` is set,
  and the archive comes from the run's verified artifact.
- Cache: `Swatinem/rust-cache` with key `windows-lint`, saved only on `main`.
- No bind address, container or connection limits apply.
