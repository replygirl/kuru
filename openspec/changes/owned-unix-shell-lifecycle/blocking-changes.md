# Dependencies

## Blocked by

- [x] `workspace-trust` — retained exact workspace `Directory` propagation and
  revalidation before built-in shell launch *(archived 2026-09-12)*

## Soft-blocked by

None.

## Siblings

`shell-environment-minimization` shares the built-in shell launch and tools
documentation, while `bounded-provider-retries` shares connector checks and
`docs/protocols.md`; coordinate source and documentation ownership without
treating either as a semantic prerequisite. `bounded-process-group-observation`
provides the existing delivery bounded runner reused only by the outer CLI test
process through a TUI dev-dependency; it is already implemented in this checkout
and adds no connector production dependency. `async-terminal-events`
and `dream-part-instruction-preamble` neither provide nor consume shell process
ownership.
