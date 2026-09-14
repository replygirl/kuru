## 1. Wrapped validation diagnostic

- [x] 1.1 Match the stable unique validation prefix in `packages/kuru-delivery/tests/powershell_diagnostics.rs`, preserve every other native assertion and invocation detail, and verify the focused portable diagnostics test, formatting, strict Cospec validation, and diff checks. *(Observed: the native log contains the prefix across PowerShell's wrapped diagnostic; portable diagnostics passed 4/4, Cargo formatting, package Taplo validation, and diff checks passed. Hosted Windows remains the required final proof.)*
