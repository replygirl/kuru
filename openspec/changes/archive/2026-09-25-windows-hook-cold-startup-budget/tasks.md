## 1. Windows hook acceptance fixture

- [x] 1.1 In `packages/kuru-connectors/src/hooks.rs`, give the three successful stock PowerShell hook commands the documented bounded native startup allowance and an aggregate budget for three sequential calls; verify the deliberate ten-second timeout/owned cleanup case is unchanged.
- [x] 1.2 Run available static checks, record the historical first-hook timeout accurately, and keep exact-head native Windows execution as a required premerge gate.

Observed on the previous native Windows head: `windows_owned_hooks_rewrite_annotate_and_stop_after_timeout` failed in its first pre-turn command with `lifecycle hook timed out` under the fixture's five-second command cap; 251 other connector library tests passed. The source-backed cold-start mechanism remains an inference. On the corrected source, Rust format and diff checks pass, and independent review confirms 120,000 ms per successful command and 360,000 ms aggregate are within validated bounds; the separate deliberate ten-second timeout and owned cleanup proof are unchanged. Corrected-head native Windows execution remains unrun and required before merge.
