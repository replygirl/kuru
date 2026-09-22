## 1. One real owner under cold competition [critical]

- [x] 1.1 @integration (agent) barrier-start two macOS client processes against a cold project with bundled Dolt -> both report one generation and both typed writes remain in one history window; `service::tests::separate_cold_starters_share_one_owner_and_preserve_both_writes` passed locally 2026-09-22
- [~] 1.2 @integration (agent) run the same native owner and cold-start fixtures on Windows and Linux -> defer: hosted native PR CI is required after push; cross-target compilation is not execution evidence

## 2. Authenticated and bounded storage transport [critical]

- [x] 2.1 @integration (agent) exercise native private socket and real Dolt typed append/history exchange -> 13 `service::tests` passed locally on macOS 2026-09-22, including wrong authority fields, oversized-frame rejection, cancelled-call invalidation, pre-open scope validation, failed-publication cleanup, and surviving repeated accept deadlines; the separate `service::rpc::tests::partial_request_header_expires_without_waiting_for_peer_eof` passed
- [~] 2.2 @integration (agent) exercise Windows native pipe ACL, handshake and service requests -> defer: hosted native PR CI is required after push

## 3. Surviving attachment and idle cleanup [critical]

- [x] 3.1 @integration (agent) attach two clients, drop one, read through the survivor, then release it and wait for idle cleanup -> `service::tests::independent_clients_elect_one_real_process_and_keep_it_warm` passed locally in 32.2 seconds; endpoint and owner lock retired after Dolt close
- [x] 3.2 @integration (agent) open a real owner and close it explicitly -> `service::tests::owner_reaps_real_dolt_before_retiring_endpoint_and_lock` passed locally; second owner rejected before engine open

## 4. Foundation checks

- [x] 4.1 @regression (agent) run package tests, lint and typecheck after internal entry integration -> `mise run //packages/kuru-memory:test` passed the full native package suite (188 lib tests plus integration targets) in 230.90s with the owning two-thread fixture policy and verified offline bundle on macOS 2026-09-22; `mise run //packages/kuru-memory:lint`, `mise run //packages/kuru-platform:lint`, `mise exec -- cargo check -p kuru-memory -p kuru --all-targets --all-features --locked`, `mise run format:rust` and `git diff --check` all exited 0 after final source edits
- [~] 4.2 @regression (agent) run normal full pre-push coverage, docs and managed checks -> defer: run once through the ordinary hook after archive/commit, coordinated with other branch writers
