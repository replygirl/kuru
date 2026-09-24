## 1. Ubuntu ARM release build [critical]

- [~] 1.1 @regression (agent) build the actual `kuru` release library on Ubuntu ARM at the corrected exact PR head -> defer: pre-fix job 107848821543 in run 36063834864 failed at Rust layout query depth 130; post-fix native ARM job can run only after the normal push and must pass before merge.

## 2. Bounded local source gate

- [x] 2.1 @integration (agent) run `mise run //apps/kuru-tui:build:release` and `cargo fmt --all --check` -> both passed on macOS after the 256 limit; the existing shutdown block was not edited.
- [x] 2.2 @equivalence (agent) review the bounded source diff against the failing head -> Toolcards review clear: only the library crate attribute and required Cospec record change; shutdown behavior stays byte-identical.
