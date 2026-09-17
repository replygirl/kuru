## Why

A native Windows runtime shell fixture timed out, but its first assertion hid whether the command performed its file write before the timeout. The existing typed receipts can provide two bounded side-effect facts when that assertion fails.

## What Changes

- In `packages/kuru-runtime/src/windows_tool_tests.rs`, add two Boolean file-read and file-delete outcomes to the existing shell-receipt parse-error context.

## Impact

Only failure diagnostics in an existing Windows runtime test change. The command, assertions, deadline, production behavior, and CI test count remain unchanged.
