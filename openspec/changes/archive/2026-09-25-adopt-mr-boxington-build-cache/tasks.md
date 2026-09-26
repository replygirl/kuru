## 1. Pin and enable the shared cache

- [x] 1.1 Pin `mr-boxington` exactly in root `mise.toml` for Linux, Apple
  silicon macOS and Windows, enable the templated Rust `mr_boxington` option,
  raise `min_version` to 2026.9.4, and regenerate `mise.lock` with `mise lock`;
  verify the lock keeps format version 1 and adds mbx entries for every locked
  platform except Intel macOS, and that `KURU_MBX=0` resolves plain Cargo.
  Evidence is in verification rows 1.2 and 1.3; `mise exec -- which cargo`
  printed mise's wrapper by default and rustup's Cargo with `KURU_MBX=0`.
- [x] 1.2 Add a scheduler-only root `.mbx.toml` and keep `target/` a real
  checkout directory with `MBX_TARGET_VIEWS=0`; verify with `taplo format
  --check` and the memory package's engine preparation and tests under mbx.
  taplo passed; see verification row 3.2.
- [x] 1.3 Accept the table form of the root Rust pin in the repository check,
  keep its equality with `rust-toolchain.toml`, and require exact root tool
  pins; verify with new `repo_validation` cases and `mise run lint:tooling`.
  `repo_validation` passed 11 tests and `lint:tooling` passed.

## 2. Keep CI and user builds outside the cache

- [x] 2.1 Set `KURU_MBX=0` in the ci, native-tests, quality and release
  workflow environments; verify with actionlint, `release_workflow` tests and
  verification row 1.1. actionlint passed and `release_workflow` passed 20
  tests with its task tools.
- [x] 2.2 Set `KURU_MBX=0` for the Unix and Windows source installers and the
  Windows source update command, with scoped PowerShell restoration and
  fixture assertions; verify with shellcheck, macOS Clippy and verification
  row 2.1. All passed; Windows-only compilation is deferred in row 2.2.
- [x] 2.3 Confirm instrumented coverage bypasses mbx caching; verify with
  verification row 3.3. No coverage task change was needed.

## 3. Documentation and evidence

- [x] 3.1 Document the shared cache, that `KURU_MBX` must come from the process
  environment (checked: `[env] KURU_MBX = "0"` in `mise.local.toml` still
  selected the wrapper), the hard-link fallback without cloning, the Intel
  macOS gap, removal of per-user `CARGO_TARGET_DIR` overrides and the mise
  floor; verify with `docs:check` and `format:check`.
- [x] 3.2 Pilot cold and warm builds across two worktrees, record timings, hits
  and disk use, then run formatting, tooling lint and affected delivery tests
  before archiving. See verification rows 3.1 to 3.3; the store held 3.6 GiB
  logical after all pilot, lint and test builds.
