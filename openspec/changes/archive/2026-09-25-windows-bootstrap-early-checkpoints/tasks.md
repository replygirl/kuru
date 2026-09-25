## 1. Localize early stock PowerShell startup

- [x] 1.1 Add flushed verbose-only checkpoints after strict-mode setup and the first verbose phase in `packages/kuru-delivery/support/install.ps1`; verify the install authority and timeout paths are unchanged in the diff.
- [x] 1.2 Require both checkpoints in order in `apps/kuru-tui/tests/embedded_runtime.rs`; verify the existing packaged-install success fixture still requires every earlier and later checkpoint.
- [x] 1.3 Run strict Cospec validation, the apply gate, source diff check, and independent review of the bounded two-file patch.
