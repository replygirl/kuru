## Why

The Intel macOS release build cannot install hk 1.58.1 because that release has
no Intel macOS asset. Building Kuru archives only needs Rust, and existing PR
checks did not exercise the two additional release platforms.

## What Changes

- `.github/workflows/release.yml`: install only locked Rust in native archive
  jobs and skip their mise postinstall repository-hook setup; retain git hooks
  and every validation job.
- `.github/workflows/ci.yml`: exercise Rust-only native builds and packaging on
  Intel macOS and Linux arm64 before merge, alongside existing full gates.
- `docs/release.md` and `docs/development.md`: describe native build tooling and
  the distinction between repository setup and archive construction.
- `communique.toml`: tighten generated-note scope after the first real artifact
  invented a future version promise and unsupported tool/provider details.

## Impact

Native build job tool installation and required CI aggregation. No application
source, dependency version, archive format, release permissions or publication
order changes. Locked installation and the Intel release target stay enabled.

## Surfaces

- [ ] interactive — no product interface change
- [x] deploy — CI and native release build tooling
- [ ] integration — no new external API contract
- [x] agent-behavior — release-note generation instructions, not runtime peers
