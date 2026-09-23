## 1. Exact Windows owner and DACL handoff [critical]

- [~] 1.1 @regression (agent) on native Windows, create an ordinary source whose `TokenOwner` differs from `TokenUser`, stage behind a checked private directory, and copy/publish its access policy -> defer: source and regression compile for `x86_64-pc-windows-msvc`, but this macOS host cannot execute the native owner/DACL behavior; hosted Windows CI must prove the exact owner, DACL, inheritance state and OWNER RIGHTS behavior after the pre-fix `staged file cannot preserve source owner access`
- [~] 1.2 @regression (agent) attempt the handoff from a source owner that cannot become a validated `TokenUser` or exact `TokenOwner` stage owner -> defer: the native regression compiles and asserts unchanged staged security after rejection, but execution requires hosted Windows
- [~] 1.3 @regression (agent) mutate the test source DACL through an exact-handle `WRITE_DAC` reopen -> defer: the corrected exact-handle fixture compiles, but hosted Windows must show it reaches the captured-policy mismatch instead of failing at fixture setup

## 2. Integrated checked file edits

- [~] 2.1 @integration (agent) run the Windows platform coverage job and connectors/core/platform coverage shard -> defer: the failing hosted jobs were captured as exact pre-fix evidence; corrected native reruns and the 90% coverage result require Delivery's pushed commit
- [x] 2.2 @equivalence (agent) run scoped format, lint/typecheck and non-Windows platform checks -> `cargo fmt --all -- --check` and `git diff --check` passed; Windows all-target `cargo check` passed; the macOS `kuru-platform` suite passed 41/41 after rerunning socket cases with native permissions
