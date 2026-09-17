## 1. Windows typed transcript assertion [critical]

- [~] 1.1 @regression (agent) compile and run the package-owned delivery acceptance fixture that previously read `Message.content` on native Windows CI -> defer: corrected-head Windows CI run 35178200480 compiled past the obsolete field but failed E0658 at `marker.as_str()` in this assertion because the literal is already `&str`; changed to `Some(marker)`. Native Windows compile and execution on this final source remain pending. Local macOS cross-target compilation is blocked by absent Windows SDK headers.

## 2. Scope integrity

- [x] 2.1 @integration (agent) inspect `mise_acceptance.rs` typed-message accesses and run scoped static checks -> `rg` found no `message.content` access; `//packages/kuru-delivery:typecheck`, `//packages/kuru-delivery:lint`, `cargo fmt --check`, and `git diff --check` passed on macOS.
