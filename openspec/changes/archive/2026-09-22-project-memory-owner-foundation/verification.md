## 1. One real owner under cold competition [critical]

- [x] 1.1 @integration (agent) barrier-start two macOS client processes against a cold project with bundled Dolt -> both report one generation and both typed writes remain in one history window; `service::tests::separate_cold_starters_share_one_owner_and_preserve_both_writes` passed locally 2026-09-22
- [~] 1.2 @integration (agent) run the same native owner and cold-start fixtures on Windows and Linux -> Windows hosted CI on PR #56, head `ead383686aa31f7e20cc5c4934914e9daf2a008b`, failed the memory runtime shard: the coverage runner placed the memory libtest in its default deny-breakaway owned Job, so an independent owner correctly rejected launch before publication. A checked runner correction now selects the existing breakaway-permitting fixture Job only for the verified memory library artifact; native rerun is pending. Linux native evidence remains pending.

## 2. Authenticated and bounded storage transport [critical]

- [x] 2.1 @integration (agent) exercise native private socket and real Dolt typed append/history exchange -> 13 `service::tests` passed locally on macOS 2026-09-22, including wrong authority fields, oversized-frame rejection, cancelled-call invalidation, pre-open scope validation, failed-publication cleanup, and surviving repeated accept deadlines; the separate `service::rpc::tests::partial_request_header_expires_without_waiting_for_peer_eof` passed
- [~] 2.2 @integration (agent) exercise Windows native pipe ACL, handshake and service requests -> Windows platform primitives and installation jobs passed on PR #56, but four memory owner fixtures failed endpoint retirement with Windows sharing violation 32: retirement's pinned read handle denied its own checked delete. The checked retirement directory now uses movable name retention while keeping the owner lock, exact generation/secret and file identity checks. The starter-survivor fixture also failed to observe endpoint retirement, plausibly the same deletion failure; native rerun must establish this rather than inference.

## 3. Surviving attachment and idle cleanup [critical]

- [x] 3.1 @integration (agent) attach two clients, drop one, read through the survivor, then release it and wait for idle cleanup -> `service::tests::independent_clients_elect_one_real_process_and_keep_it_warm` passed locally in 32.2 seconds; endpoint and owner lock retired after Dolt close
- [x] 3.2 @integration (agent) open a real owner and close it explicitly -> `service::tests::owner_reaps_real_dolt_before_retiring_endpoint_and_lock` passed locally; second owner rejected before engine open

## 4. Foundation checks

- [x] 4.1 @regression (agent) run package tests, lint and typecheck after internal entry integration -> `mise run //packages/kuru-memory:test` passed the full native package suite (188 lib tests plus integration targets) in 230.90s with the owning two-thread fixture policy and verified offline bundle on macOS 2026-09-22; `mise run //packages/kuru-memory:lint`, `mise run //packages/kuru-platform:lint`, `mise exec -- cargo check -p kuru-memory -p kuru --all-targets --all-features --locked`, `mise run format:rust` and `git diff --check` all exited 0 after final source edits
- [x] 4.2 @regression (agent) run normal full pre-push coverage, docs and managed checks -> normal hook passed on pushed head `ead383686aa31f7e20cc5c4934914e9daf2a008b`; it predates the Windows native corrections above, so its result does not validate them. The next hook and hosted CI are pending.

The four-file Windows correction passed independent source review and local `//packages/kuru-delivery:typecheck`, `//packages/kuru-memory:typecheck`, `format:rust`, and `git diff --check` on macOS. These checks do not execute Windows behavior. PR #56 run `35699375176` also had two Windows application shell tests exit with main-thread stack overflow after their pre-existing script stage markers completed; those failures are outside this owner correction and remain tracked by the Windows shell diagnostic lane.
