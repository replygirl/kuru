# Proposal

## Why

The docs app's transitive development dependency `source-map-js` is pinned to 1.2.1, within the published high-severity vulnerable range. Update the lockfile to the first patched version, 1.2.2.

## What Changes

- Update only `apps/kuru-docs/package-lock.json` to resolve `source-map-js` 1.2.2.

## Impact

The docs app's development dependency graph receives the security fix; runtime behavior and dependency declarations remain unchanged.
