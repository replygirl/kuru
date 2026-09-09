## 1. Focus-loss observation [critical]

- [x] 1.1 @regression (agent) Split terminal frame delivery deterministically -> the text-only implementation returned Ok while the producer withheld the cursor trailer, causing the regression to fail with exit 101; the corrected test passes, rejecting text-only and partial escape stages, accepting the complete Show/MoveTo trailer and still rejecting a later acknowledged visual update with the exact byte-count assertion
- [x] 1.2 @integration (agent) Run real PTY tests in normal and reduced-motion modes -> mise run //apps/kuru-tui:test -- --test terminal exited 0, including 5 terminal tests with one subprocess fixture entry ignored; normal/reduced focus suspension, cancellation, persistent selections and real app terminal restoration pass
- [x] 1.3 @integration (agent) Run full mise check -> exit 0 on 2026-09-09 with 214 passing Rust tests and unchanged 97.54% line coverage (8681/8900), above the 90% gate; formatting, Clippy, tooling lint, docs and cospec checks pass

## 2. Hosted evidence

- [x] 2.1 @runtime (agent) Inspect Linux CI run 34401929985 -> coverage failed real_pty_accepts_chat_navigation_commands_and_restores_terminal at terminal.rs:179 with 66641 versus 66633 output bytes; macOS passed the full check stage
- [~] 2.2 @runtime (agent) Confirm hosted checks for the corrected commit -> defer: run after this local evidence record is archived and committed, with the result recorded in GitHub and reported separately
