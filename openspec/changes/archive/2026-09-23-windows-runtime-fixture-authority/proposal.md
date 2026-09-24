## Why

Two runtime fixtures fail on native Windows before exercising their intended behavior because one compares a noncanonical retained tool root with the canonical workspace and the other performs accepted file mutations without the required private checkpoint store.

## What Changes

- Build the cancellation fixture's retained tool root and harness from one canonical project path.
- Give the native Windows tool replay fixture the same retained-root, private-checkpoint authority required by accepted file mutations.

## Impact

Only `kuru-runtime` test setup changes. Production workspace validation and checkpoint requirements remain unchanged; native runtime coverage exercises the intended cancellation and authority assertions.
