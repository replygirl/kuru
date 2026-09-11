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
The independent `memory-connection-diagnostics` and `windows-update-acknowledgment`
fixes were archived after native acceptance passed. Browser handoff controls and
all five installed/update targets passed in CI 34612890798. The original plain
shell request passed in CI 34617836357 after removing its diagnostic observers.
That run exposed an independent delivery test's instantaneous lock-release
assumption, tracked by windows-command-lock-release. The corrected bounded
observation and original plain shell passed in final CI 34621868783, alongside
all five native application targets. This test correction does not change the
native OpenAI authentication scope.
