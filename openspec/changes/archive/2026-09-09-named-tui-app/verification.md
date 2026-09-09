## 1. Application path [critical]

- [x] 1.1 @equivalence (agent) Run the existing workspace gate at the new path -> full mise run check passes: 148 Rust tests and 97.64% coverage; independent byte comparison confirms all moved application source/manifests unchanged.
- [x] 1.2 @integration (agent) Install through apps/kuru-tui and inspect metadata -> cargo install --path apps/kuru-tui --locked --root /tmp/kuru-named-app-install succeeds; installed binary reports kuru 0.1.0. Active docs/tasks/workspace have no remaining old path.
