## 1. Integrated MCP catalog acceptance

- [x] 1.1 Extend `apps/kuru-tui/tests/terminal.rs` with a bounded test-owned stdio MCP peer and verify real `kuru tools` and registered `/tools` expose the same live, stale, degraded, and disabled aliases without secret values.
- [x] 1.2 In the same deterministic fixture, return one fake-provider response containing exact live, stale, and denied MCP calls and verify provider-order results, exactly one live effect, and zero stale, denied, or disabled effects.

## 2. Closure

- [x] 2.1 Run strict Cospec validation, formatting and diff checks, then execute the exact Unix PTY filter with synchronized completed frames and bounded process cleanup. *(Observed 2026-09-22: strict validation, formatting, diff check and no-run compile passed; the final authorized native filter passed 1/1 in 6.31s.)*
- [x] 2.2 Record Unix-only local evidence honestly and prepare the completed test change for archive and a normal-hook commit without claiming new native cross-platform behavior. *(The initial sandbox run was blocked by loopback `EPERM`; native iterations exposed and corrected a missing fake model-catalog route and the expected stale-route projection before the final pass.)*
