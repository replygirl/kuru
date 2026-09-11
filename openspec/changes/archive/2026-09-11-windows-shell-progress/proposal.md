## Why

The native Windows shell fixture timed out with a still-running PowerShell process and no output in CI 34602482369. Its first output occurs after file hashing, so the failure cannot distinguish interpreter startup from module import or command execution.

## What Changes

Add private fixed-stage markers to the existing apps/kuru-tui/tests/windows_cli.rs control and Kuru shell invocations. Keep distinct marker files and assert the observed sequence while preserving real hashing, environment retention, file identity, negative module-import control and timeout/cleanup behavior.

After an original timeout only, run one separate bounded Core debugger trace of
the same fixture source; retain the original failure regardless of that diagnostic
result. Keep the original unobserved source authoritative for both the poisoned
control and the Kuru request, preserving cold command discovery.

CI 34612890798 passed its instrumented Windows suite with temporary lookup
observers and no production shell change. Remove that wrapper rather than use
its pass as evidence that the original cold command works. Its observed result
remains in the verification history; the retained Core trace runs only after an
original failure and does not substitute a successful retry.

## Impact

Only the existing Windows shell acceptance fixture gains bounded marker files and a timeout-only diagnostic child. The diagnostic uses stock PowerShell's `-Command` transport and labels that difference from Kuru's authoritative `-EncodedCommand` call. No product behavior, timeouts, dependencies, CI topology or runtime evaluation changes.
