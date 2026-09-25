## Why

The instrumented macOS Kuru executable is 171,024,280 bytes, above the embedded-runtime fixture's selected-input limit of 168,820,736 bytes. The fixture rejects it before making its private copy, so the required offline install and self-update acceptance never runs even though the same executable strips to 118,213,992 bytes, below the unchanged 128 MiB shipping limit.

## What Changes

- Bound selected debug and instrumented Cargo inputs independently at 192 MiB before copying and stripping them in the fixture.
- Retain exact source identity, digest and independent-copy checks, the 128 MiB prepared executable limit, profile collection, and actual offline install and self-update assertions.

## Capabilities

### Modified Capabilities

## Impact

Only `apps/kuru-tui/tests/embedded_runtime.rs` and its acceptance evidence change. No product API, archive limit, installed bytes, or runtime policy changes.

## Surfaces

- [x] deploy — native CI execution of the packaged runtime acceptance fixture
