## Why

The staged Windows release gate proves current installation, but it does not run a previously published `kuru update` executable. The existing three-member core compatibility contract needs a real old-client check before public promotion.

## What Changes

- Extend the existing staged Windows acceptance test and its package-owned helper to authenticate the official v0.4.2 Windows executable, run its old updater against the exact staged candidate, and verify replacement and explicit shell-support repair.
- Record the new prepromotion proof and its separate postpublication public-download limit in release operations documentation.

## Impact

The existing `verify-staged-windows` job runs one additional isolated native Windows acceptance path before the existing `publish` dependency. No release workflow graph, product code, updater policy, or public asset changes are introduced.
