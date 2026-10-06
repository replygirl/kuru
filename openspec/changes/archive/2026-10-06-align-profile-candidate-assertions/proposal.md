# Proposal

## Why

PR239's Ubuntu partition 7 exposed two diagnostic tests still expecting the superseded positive writer claim. An exact search found one additional coupled late-profile assertion with the same obsolete wording.

## What Changes

- `packages/kuru-delivery/src/coverage/orchestrate.rs`: align those three existing test assertions and the nearby fixture comment with candidate/unknown attribution; retain every profile prefix, identity, PID, error and receipt assertion.

## Impact

Test assertions only, with one focused pure selection covering the three coupled cases and five existing attribution cases. No production, process, profile parsing, strict export, threshold, pin or workflow change; prior archives remain immutable.
