## Why

CI repeats static checks on three operating systems and runs ordinary tests before rerunning them for coverage. Independent quality checks should start concurrently in CI and hk, with native jobs focused on observable platform behavior.

## What Changes

- `.github/workflows/ci.yml` and reusable quality/coverage workflows: separate format, lint, typecheck, tooling, cospec and documentation jobs from native coverage, installation and update checks; retain an always-run, fail-closed `ci-gate`.
- `.github/workflows/release.yml`: reuse granular validation at the immutable dispatch and version commit SHAs, preserving version/retry behavior, five shipping builds and final release-only Pages publication.
- Root and package `mise.toml` files and `hk.pkl`: expose independently schedulable checks, remove ordinary-test prerequisites from coverage, and retain package-owned bundle/fixture preparation and instrumented supervisor coverage.
- `AGENTS.md` and contributor/release documentation: describe granular commands and checks accurately. Keep the optional local `check` aggregate convenient without using it as a CI or hook scheduling unit.

## Impact

Static analysis runs once on Ubuntu; native tests still compile and exercise OS-specific branches. This intentionally does not claim Linux Clippy checks Windows/macOS conditional code. The three full native coverage legs, two additional architecture shipping legs, Windows primitive coverage and 90% thresholds remain required. Hook checks run concurrently subject to real Cargo/fixture resource constraints. No secrets, runtime application behavior, release inputs, publication authorization or branch-protection check name change.

## Surfaces

- [ ] interactive — no product interface changes
- [x] deploy — CI execution and hook/task scheduling topology
- [ ] integration — existing tools and action contracts retained
- [ ] agent-behavior — no provider or prompt changes
