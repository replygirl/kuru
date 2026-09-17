## 1. Windows typed transcript assertion [critical]

- [~] 1.1 @regression (agent) compile and run the package-owned delivery acceptance fixture that previously read `Message.content` on native Windows CI -> defer: native Windows runner must compile and execute `windows_mise`; local macOS cross-target attempt is blocked by absent Windows SDK headers.

## 2. Scope integrity

- [x] 2.1 @integration (agent) inspect `mise_acceptance.rs` typed-message accesses and run scoped static checks -> `rg` found no `message.content` access; `//packages/kuru-delivery:typecheck`, `//packages/kuru-delivery:lint`, `cargo fmt --check`, and `git diff --check` passed on macOS.
