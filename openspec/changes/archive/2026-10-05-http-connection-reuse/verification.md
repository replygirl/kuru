# Verification

## 1. Reuse a completed HTTP connection without sharing actor request state [critical]

- [x] 1.1 @integration (agent) send two completed requests through one provider instance to a loopback peer and count accepted TCP sockets -> both complete over one accepted HTTP/1.1 connection and their actor-specific request bodies remain distinct; connector test `completed_requests_reuse_keepalive_connection_and_isolate_actor_inputs` passed 1/1 and again in the final 5/5 selection
- [x] 1.2 @benchmark (agent) compare accepted TCP sockets per two completed requests for one reused provider and fresh providers -> reused provider 1 socket, two fresh providers 2 sockets (50% fewer accepted sockets in this controlled fixture); actual counts are recorded in `proposal.md`, with no latency threshold or claim
- [x] 1.3 @equivalence (agent) run the existing Responses provider and SSE characterization selections -> `subscription_completion_and_catalog_retry_explicit_rejections`, `repeated_401_and_partial_stream_errors_do_not_retry_or_leak_tokens`, `cancelled_partial_stream_closes_socket_and_releases_same_actor`, and `missing_content_type_accepts_fragmented_subscription_sse` passed with the new reuse fixture in the final 5/5 selection

## 2. Keep memory service effects distinct from HTTP pooling

- [x] 2.1 @manual (agent) review the scoped protocol/development documentation against the integrated service lifecycle evidence -> HTTP socket reuse is limited to the measured completed HTTP/1.1 fixture; N4's `two_fresh_terminals_share_owner_and_keep_private_sessions_through_eof` and `independent_clients_elect_one_real_process_and_retire_after_both_detach` evidence is cited for overlapping attachments, one owner generation, and final-client retirement/reap, with no warm-idle retention claim

## 3. Scoped validation

- [x] 3.1 @manual (agent) run the owning connector typecheck, host lint, Windows-target lint, docs check, and Cospec strict/apply gates -> all commands exited 0; the actual socket counts and named provider/SSE tests are recorded above
