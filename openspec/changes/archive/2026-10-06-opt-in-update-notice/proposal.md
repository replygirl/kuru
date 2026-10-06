# Proposal

## Why

Kuru has no opt-in update availability notice despite the canonical D2 outcome. A personal preference should permit a bounded advisory check without slowing startup or shutdown, granting installation authority or allowing repository text to cause network traffic.

## What Changes

- Add `[update] notice = false` to the immutable configuration and public schema, with repository-origin enablement rejected even after workspace trust and existing managed constraints retained.
- Add a bounded fixed-HTTPS manifest check and private advisory cache, with sanitized stable-version and package-manager hints.
- Activate only for interactive TUI with terminal stderr after reviewed configuration; display at most one advisory after terminal restoration and abort unfinished work without waiting at exit.
- Prove cache/failure/cancellation bounds, preference provenance, excluded headless commands and actual restored-terminal output using isolated fake HTTPS and real terminal fixtures.

## Capabilities

### New Capabilities

- `update-notices`: Personal opt-in, bounded advisory availability checks and post-restoration notices.

### Modified Capabilities

- `configuration-schema`: Personal update preference and repository-origin enablement refusal.

## Impact

Own core configuration/parser tests, docs schema, delivery notice/parser/cache and narrow existing test-support feature, CLI interactive-only lifecycle wiring, feature docs and fixtures. Reuse existing platform checked files, HTTP dependencies and ownership hint APIs; no memory/runtime/terminal-render changes, provider calls, install effects, new pins or release workflow.

## Surfaces

- [x] interactive — opt-in advisory after restored terminal
- [ ] deploy — no installation or release topology change
- [x] integration — fixed HTTPS release manifest and configuration schema
- [ ] agent-behavior — no actor/provider policy changes
