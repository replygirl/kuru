## 1. Configured lifecycle hook builds on Windows [critical]

- [~] 1.1 @regression (agent) compile the real Windows source installation before and after the connector boundary correction -> defer: the red half is observed in P14 PR #89 Windows installation job `108124216149` and delivery-archive coverage job `108124216116`, both E0308 at `process.rs:39`; the green half awaits corrected exact-head native Windows source installation.
- [~] 1.2 @integration (agent) exercise the configured finite hook launch on native Windows -> defer: exact-head application/runtime CI must pass the existing hook process and terminal cases without changing environment authority.

## 2. Repository checks

- [x] 2.1 @regression (agent) run affected format, lint, typecheck and strict Cospec/apply checks -> macOS connector all-target/all-feature typecheck and Clippy passed; Rust format and `git diff --check` passed; strict Cospec reported 0 errors/0 warnings and apply exited 0 with a clear gate.
- [~] 2.2 @integration (agent) run normal final-head push hooks and combined coverage at or above 90% -> defer: publication follows this fix's archive and commit; the previous green hook belongs to head `2aedf56e`.
- [~] 2.3 @integration (agent) observe final-head Windows, macOS and Linux CI before stack merge -> defer: hosted CI follows the corrected branch publication.

The installed Rust toolchain includes `x86_64-pc-windows-msvc`, but a local
connector cross-target `cargo check` on macOS stopped in transitive
`aws-lc-sys` C compilation because the Windows MSVC SDK headers were absent.
It did not reach the corrected Rust boundary and is not counted as a pass or a
new product failure. Native Windows CI is the decisive green regression proof.
