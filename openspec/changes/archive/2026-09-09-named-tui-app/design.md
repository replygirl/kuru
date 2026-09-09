## Context

The single terminal app occupies apps/tui. Name it apps/kuru-tui to make ownership
explicit and leave apps/ available for future branded or internal applications.

## Goals / Non-Goals

Goal: consistent app-directory naming with no runtime changes.
Non-goals: renaming the installed kuru executable or its Cargo package.

## Decisions

Move the directory and replace active references atomically. Retain historical
cospec evidence as written; rewriting it would obscure what was actually tested.

## Risks / Trade-offs

Stale install or tooling paths could break delivery. Search active files and
verify workspace tests, metadata and source installation through the new path.

## Seam ownership

apps/kuru-tui continues to own CLI and rendering. Packages retain their current
runtime, connector and core responsibilities; no state boundary changes.
