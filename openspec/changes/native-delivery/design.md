## Context

The TUI and peer runtime are Rust, but delivery helpers and real terminal/protocol
tests currently launch Python. Root package manifests also made unrelated tools
look like language-owned projects. Correct ownership before publishing the new
docs and release flow.

## Goals / Non-Goals

Goals: no Python or Bun requirement; package-local mise tasks; equivalent real
process/HTTP/PTY coverage and safe installation; a useful docs site with its
Node dependency isolated to that app.
Non-goals: rewrite VitePress, remove Cargo's necessary workspace resolution,
change user CLI names, weaken coverage, or alter cognitive/runtime behavior.

## Decisions

packages/kuru-delivery exposes an installer/archive library plus internal CLI
entry points. The TUI uses the library for updates; maintainer tasks use the CLI.
Use existing Unix APIs plus an incremental terminal parser for Rust PTY fixtures.
Keep compiled fixture subprocesses under the package that tests their contracts.
Move task definitions to app/package mise.toml files; root aliases orchestrate
them and retain one workspace-wide Rust coverage pass. Use npm's local lockfile
only inside the VitePress app, without a root JS workspace.

Cospec 0.7.0's vendored OpenSpec bundle calls its entrypoint twice after bundling
collapses two module identities. A package-owned compatibility preload applies
only to that known content hash in cospec's child runtime. Task-local NODE_PATH
and BUN_OPTIONS load it without an external Bun installation. It suppresses the
redundant self-main guard, leaving the original command and all cospec gates
intact. Actual standalone tests verify one JSON response plus hard, soft and
missing-artifact blockers. Remove the shim after an upstream release fixes the
bundle and those tests pass without it.

## Risks / Trade-offs

Porting test drivers can lose behavioral coverage. Preserve delayed scheduling,
output backpressure, cancellation, restart persistence and negative protocol cases.
Archive parsing is security-sensitive: reject traversal, links, duplicates,
oversized expansion and checksum ambiguity before atomic replacement. Verify
all native tools and application code together under the unchanged 90% gate.

## Seam ownership

The delivery package owns archive/installation and maintainer automation.
The TUI owns terminal integration support; connectors own their subprocess
fixtures. Docs owns its npm manifest/lockfile/theme. Root owns pins, Cargo
dependency resolution, mise aggregation and hosted settings.

## Operational surface

Existing macOS/Linux runners and four release targets. No new service, bind
address or credential. Release secrets remain the cospec-scoped app credentials
and ANTHROPIC_API_KEY_COMMUNIQUE; the separate release change owns publication.
