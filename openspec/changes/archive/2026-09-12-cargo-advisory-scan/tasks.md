## 1. Delivery-owned advisory scanner

- [x] 1.1 Add the exact package-scoped `cargo:cargo-audit` 0.22.2 mise tool
  with `locked = true` and `default-features = false`, plus delivery-local
  audit configuration with no ignored advisory IDs; verify no root tool,
  application dependency, or Cargo lockfile selection changes. Refresh only
  `packages/kuru-delivery/mise.lock` for the package-owned tool, and update
  `docs/dependencies.md` with the scanner's bounded advisory-only scope.
- [x] 1.2 Add the delivery Rust wrapper and `audit:advisories:refresh` / offline
  `audit:advisories` tasks. Verify direct mise-selected `cargo-audit` execution
  scans only `../../Cargo.lock`, requires an absolute isolated database path,
  rejects wrong-origin, dirty, non-detached, missing, stale, or changed-HEAD
  databases, logs the full pre/post SHA and timestamp, and passes `--no-fetch`
  and `--no-yanked`; the latter intentionally omits yanked-package index checks
  and is not an advisory waiver.
- [x] 1.3 Verify the explicit refresh uses the exact public RustSec origin and
  rejects an unexpected existing checkout before mutation. A fresh owned
  `git init` receives only the exact origin/refspec, fetches `origin HEAD`,
  and detaches that explicit snapshot; before any existing checkout network action or
  mutation, reject local config keys outside the documented detached-clone
  allowlist (`core.repositoryformatversion`, `core.filemode`, `core.bare`,
  `core.logallrefupdates`, optional boolean `core.ignorecase` /
  `core.precomposeunicode`, `remote.origin.url`, `remote.origin.fetch`) and
  reject an unexpected exact origin/refspec. The bounded Git child clears
  inherited repository selectors and `GIT_CONFIG*` injection variables, disables
  prompts and system/global config, and reads local config without includes;
  document the remaining host process-policy boundary without claiming total
  home isolation.

## 2. Quality workflow

- [x] 2.1 Add one Ubuntu advisory job to the reusable quality workflow. Verify
  it prepares the package-owned exact tool, refreshes a `$RUNNER_TEMP` advisory
  checkout once, runs the ordinary offline task, and joins `quality-gate`
  without adding fetches to native, build, test, or coverage jobs.
- [x] 2.2 Add focused delivery fixture coverage for acceptance and rejection
  paths, then run the package lint/typecheck/tooling checks and actionlint.
  Verify documented remediation names the explicit refresh task and that each
  newly reported RustSec vulnerability fails unless a later reviewed change
  records its exact advisory ID and rationale.
