## 1. Windows fixture lifetime

- [ ] 1.1 @regression (agent) run the application Windows shard that previously returned successful output followed by process-tree quiescence timeouts -> each checked command returns after its root and ordinary descendants exit, while managed-service cleanup succeeds
- [ ] 1.2 @integration (agent) run the sequential CLI warm-owner fixture -> two commands reuse the exact endpoint generation, release attachments, then authenticated retirement removes the endpoint and lifecycle lock
- [ ] 1.3 @integration (agent) run the packaged offline and ConPTY fixtures -> their managed owner escapes only the immediate fixture Job, command/terminal completion is observed, and exact fixture cleanup retires the owner before root deletion
- [ ] 1.4 @equivalence (agent) run delivery/platform/application all-target checks and the existing command-tree timeout regression -> ordinary descendants remain owned and a genuine surviving non-breakaway child still fails quiescence

## Observed evidence

- `cargo fmt --all -- --check` and `git diff --check` passed on 2026-09-23.
- `cargo check -p kuru-delivery -p kuru --all-targets --all-features` passed on the scoped source tree.
- The macOS sequential CLI control did not reach its lifecycle assertions: both the initial isolated-target run and the run after the package prefetch completed exited in 2.87 seconds at the first managed-owner start with `memory service exited before readiness: exit status: 1`. The package prefetch itself completed and verified the bundled Dolt path. The failure does not establish a bundle, prepared-supervisor, resource, or product cause; the changed launch selector is Windows-only and hosted Windows verification remains required.
