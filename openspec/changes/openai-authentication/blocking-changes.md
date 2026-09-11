# Dependencies

## Blocked by

- [x] `native-platform` — checked private credential filesystem primitives *(archived 2026-09-10)*
- [x] `native-windows` — native application and installation support *(archived 2026-09-11)*

## Soft-blocked by

None.

## Scope

The user explicitly authorized complete direct
OpenAI authentication as the remaining correction in the Dolt/portability PR.
Archived provider-tools and parallel-quality-gates provide the existing transport
and test graph. No new runtime evals or provider configuration redesign is needed.
The latest Windows closure CI memory connection failure still requires resolution
under the PR's existing native acceptance. The independent
`memory-connection-diagnostics` fix preserves the lost startup error context;
neither change consumes the other's implementation.
