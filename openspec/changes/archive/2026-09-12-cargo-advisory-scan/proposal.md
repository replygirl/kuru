## Why

Kuru's dependency pin review does not automatically detect RustSec vulnerability
advisories in the committed Cargo lockfile. A narrow, reproducible advisory scan
will make new known vulnerabilities fail a dedicated Ubuntu quality check without
expanding into license, source, or dependency-policy enforcement.

## What Changes

- Add a delivery-package-owned, exact-pinned `cargo-audit` 0.22.2 task pair: an
  explicit isolated RustSec database refresh and an offline root `Cargo.lock`
  advisory scan. The scan invokes the mise-selected `cargo-audit` executable
  directly, never `cargo audit` or a home Cargo alias.
- Add a separate Ubuntu advisory quality job that installs the delivery-owned
  tool, refreshes its temporary public database checkout, then runs the offline
  scan and joins the quality gate.
- Record an empty, explicit project-local audit configuration. Any future
  exception requires a separate reviewed change with its RustSec identifier and
  rationale.

## Impact

`packages/kuru-delivery/mise.toml`, `packages/kuru-delivery/mise.lock`, its
local audit configuration, bounded Rust wrapper and focused tests,
`docs/dependencies.md`, and the actual reusable workflow
`.github/workflows/quality.yml` gain CI/tooling behavior only. The delivery
package's working directory is part of the audit-config contract; the wrapper
requires an absolute `KURU_ADVISORY_DB`, validates the exact RustSec origin,
clean detached full-SHA `HEAD`, and a maximum 90-day commit age before and after
an offline `--no-fetch --no-yanked` scan.

A fresh owned `git init` checkout starts with no branch configuration, then
receives only the exact RustSec origin and fetch refspec before an explicit
`origin HEAD` fetch and detached checkout. Before an existing checkout is
refreshed or scanned, the wrapper clears inherited Git repository
selectors and every inherited Git-config injection variable, disables prompts,
and supplies an owned empty global configuration with system configuration
disabled. It then reads local configuration without following includes and
rejects any key outside the documented fresh detached-clone allowlist:
`core.repositoryformatversion`, `core.filemode`, `core.bare`,
`core.logallrefupdates`, optional boolean `core.ignorecase` /
`core.precomposeunicode`, `remote.origin.url`, and `remote.origin.fetch`; the
origin URL and fetch refspec must match the expected public RustSec checkout.
This rejects local URL rewrites, include directives, hook paths, credential
helpers, and transport/executable overrides before any network operation or
checkout mutation. It intentionally does not claim to isolate every host
executable lookup or system policy.

The job requires network access only for the pinned tool installation and its
explicit public database refresh; ordinary local scanning remains offline after
a deliberate refresh. It accepts no advisory ignores; `--no-yanked` deliberately
omits yanked-package index checking from this advisory-only scanner.
No application source, Cargo dependency graph, runtime behavior, secrets, or
native-platform job changes are included.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
