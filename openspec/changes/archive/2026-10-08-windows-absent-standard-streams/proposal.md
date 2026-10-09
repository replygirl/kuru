# Proposal

## Why

All142 native Windows platform tests pass at77e081f2, but the unchanged95% gate fails. Saved coverage leaves the public absent-standard-stream policy untested: null child stdio, NotConnected console admission and refused ANSI output.

## What Changes

- Extend the existing isolated `verify_private_console_fixture` in `packages/kuru-platform/src/windows/console.rs` with real absent Input/Output/Error slots. Retain original handles, observe every public outcome, restore each slot and original console modes before assertions or output, and prove absent output refuses ANSI even when the retained console had VT enabled.

## Impact

One existing native console fixture covers real public admission failures; no production policy, new fault seam, timeout or coverage exclusion changes. Native Windows x64 and ARM acceptance remains required.
