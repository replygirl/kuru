## Why

The packaged Windows bootstrap fixture can time out with open pipes and no captured output, leaving its earliest script boundary unknown. Its existing verbose phase diagnostics need a direct, flushed stderr signal before native bridge compilation.

## What Changes

- Add three fixed, `-Verbose`-gated early phase markers to the shipped PowerShell bootstrap, written and flushed directly to stderr.
- Extend the existing bootstrap fixture to assert these markers when the genuine script completes.

## Impact

Touches `packages/kuru-delivery/support/install.ps1` and focused Windows bootstrap acceptance assertions. The command and its 100-second fixture deadline stay unchanged; native Windows confirmation remains required.
