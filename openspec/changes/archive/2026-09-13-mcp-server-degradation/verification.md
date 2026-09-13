## 1. Alias-local catalog and recovery [critical]

- [x] 1.1 @integration (agent) discover real healthy stdio and HTTP fixtures beside failed aliases in both deterministic orders -> built-in and healthy tools remain usable, each failed alias has fixed status, and no partial failed-alias route is advertised or callable
- [x] 1.2 @integration (agent) return a valid first page followed by malformed, repeated and oversized catalog data -> the complete alias is unavailable while unrelated published routes remain exact
- [x] 1.3 @runtime (agent) run a provider turn with one failed and one healthy alias -> the provider receives only usable tools and fixed MCP activity reaches TurnOutput/TUI without entering provider input or memory
- [x] 1.4 @integration (agent) repair a failed alias and invoke explicit discovery -> its complete routes become usable without changing another alias or replaying an earlier call

## 2. Call serialization and no replay [critical]

- [x] 2.1 @integration (agent) overlap multipage discovery with a call on the same alias and calls on a different alias -> same-alias logical operations do not interleave while the independent alias proceeds
- [x] 2.2 @integration (agent) have a real fixture receive one mutating call and disconnect before replying -> exactly one request arrives, the alias becomes unavailable, and later calls write zero bytes until explicit successful discovery
- [x] 2.3 @integration (agent) cancel before dispatch and after observed dispatch -> pre-dispatch cancellation writes nothing, post-dispatch cancellation disables and cleans the session, and neither path automatically resends
- [x] 2.4 @regression (agent) return projected useful MCP `isError=true` content and then call again -> the application result remains distinct and the same healthy session stays available

## 3. Owned stdio and stderr [critical]

- [x] 3.1 @unit (agent) exercise Unix one-shot stdin/stdout/stderr extraction plus the existing signal-before-reap state transitions -> pipe access never exposes or transfers numeric process authority
- [x] 3.2 @integration (agent) emit a stderr flood containing fake credentials across scanner/read/tail boundaries, invalid UTF-8 and controls -> capture drains continuously and the bounded human tail is projected and escaped before exposure
- [x] 3.3 @integration (agent) exercise timeout, natural root exit, caller loss and parent-runtime loss with a same-group descendant retaining stderr on native macOS and Linux -> worker confirms ordered cleanup or honestly retains the owner without a post-reap signal
- [x] 3.4 @integration (agent) run equivalent caller/runtime-loss, stderr-flood and descendant-held-pipe fixtures with the Windows Job owner -> Job/tree and pipes become quiescent without inferring parity from Unix

## 4. Shutdown and presentation [critical]

- [x] 4.1 @integration (agent) race shutdown with starting, active and idle clients, then attempt discovery and invocation -> no new client is admitted after closure, every admitted client is attempted within one aggregate bound, and successful shutdown confirms its cleanup before return
- [x] 4.2 @e2e (agent) run real `kuru tools`, `kuru tool`, plain run and JSON run with a failed alias and fake stderr credential -> stdout remains shape-compatible, only direct human stderr receives the safe tail, and runtime JSON contains fixed existing-shape events
- [x] 4.3 @eval (agent) inspect captured provider requests, tool errors, event JSON and reopened memory after failure -> command/env/endpoint/raw stderr sentinels are absent and only fixed alias status is retained

## 5. Quality and documentation

- [x] 5.1 @unit (agent) exercise Scanner with a cleared reusable output buffer and long cumulative input -> destination capacity stays bounded by the current chunk while cumulative overflow/growth checks remain effective
- [x] 5.2 @integration (agent) run focused platform, connector, runtime and TUI tests plus format, lint, typecheck and documentation checks; run workspace coverage once when coordinated -> all executed gates pass, the 90% floor remains intact, and unrun native evidence is explicitly deferred
- [x] 5.3 @integration (agent) build and inspect curated tools/protocol documentation -> public text matches alias degradation, explicit recovery, no replay, human-only stderr and finite-detector/process limits without publishing private notes

## Observed evidence

- macOS connector package: `RUST_TEST_THREADS=2 cargo test -p kuru-connectors --all-targets --all-features --locked --no-fail-fast` exited 0; 150 library tests passed, including 22 MCP and 4 RPC cases. The MCP cases include later-page atomicity, independent-alias progress, pre-ready cancellation, setup-failure retention and starting/active/idle shutdown.
- macOS focused acceptance additions: the exact `catalog_keeps_healthy_stdio_alias_when_http_alias_fails_in_either_order`, `explicit_catalog_recovers_a_failed_alias_without_replaying_its_call` and `received_stdio_mutation_disconnects_once_and_disables_later_calls` tests each exited 0. Together with the original healthy-HTTP/failed-stdio and cancellation cases, they observe both real transport combinations, preserve an independent alias across explicit recovery, and confirm that a real stdio peer receives one mutating call before disconnect while two later calls and shutdown add no requests. The full package suite has not been repeated after these proof-only additions.
- macOS Unix owner: `cargo test -p kuru-platform unix::tests -- --nocapture` exited 0; 7 tests passed. The runtime-loss RPC case used a real same-group descendant retaining inherited stderr and observed explicit cleanup confirmation after the parent Tokio runtime ended.
- Static checks: connector/platform and runtime/TUI `cargo clippy --all-targets --all-features -- -D warnings` exited 0; `cargo fmt --all -- --check` and `git diff --check` exited 0.
- Runtime isolation: `review_tests::unavailable_mcp_status_stays_out_of_provider_input_and_memory` exited 0 against actual durable Dolt, then closed and reopened that store. One failed and one healthy MCP alias produced fixed event metadata and only usable tools in captured provider requests; fake command, environment secret and HTTP endpoint were absent from provider requests and both live and reopened memory.
- Direct CLI: `direct_tools_keeps_stdout_json_and_reports_filtered_failed_stdio` exited 0. Real `kuru tools` and `kuru tool file_list` processes preserved JSON stdout and emitted fixed status plus a recognizable-secret-filtered, terminal-escaped diagnostic on human stderr. Real plain and JSON runs preserved their stdout shapes; JSON used the existing fixed MCP event, and neither run exposed the diagnostic, command or fake secret.
- Documentation: `mise run docs:check` exited 0 with pinned Node 26.8.2/npm 12.0.2, oxfmt, oxlint, VitePress build, and delivery content/link inspection.
- Exact-base push gate: all eight normal hook categories exited 0 at `4de6b2a19ea24e458867d765916d697120ed3a47`; combined LCOV covered 27,740 of 29,085 lines (95.3756%). At that local checkpoint this predated the three proof-only tests above, so their combined coverage had not yet run.
- Strict cospec validation and the acknowledged apply gate exited 0 with only the declared `tool-result-redaction` and `owned-unix-shell-lifecycle` soft blockers.

Historical checkpoint: native Windows ownership/stderr execution and the unchecked compound rows above had not yet completed at that point. The later successful hosted run `34757216251` closes that gap with actual Windows execution; cross-compilation is not used as native evidence.

- Exact-base hook: `b8c186f` completed all eight normal hook categories; the
  authoritative LCOV total was 27,966/29,315 (95.3983%).
- Hosted macOS run `34742573359` failed only
  `toolhost_stdio_mcp_projects_application_success_and_protocol_results`: its
  fixture recorded initialize, initialized, list, and exactly three calls, then
  protocol-error cleanup ended the peer before its planned `Step::Eof`
  created `.done`. `.done` is not guaranteed during protocol-error cleanup; the
  correction asserts the complete ordered wire transcript after successful
  `ToolHost::shutdown` instead. The focused local test, connector typecheck, and
  connector lint each passed after the correction.

- Hosted CI `34743613521` observed hosted Linux fully green and instrumented
  macOS green for the b8 base. The exact base hook remains the authoritative
  27,966/29,315 (95.3983%) LCOV evidence for that historical head.
- Hosted CI `34757216251` completed successfully at exact head `7be6b46f8521c7784f4a0471054286de5fc3d2b9` on Linux, macOS and Windows. Linux and macOS each ran the real same-group descendant timeout, natural-exit, caller-loss and parent-runtime-loss fixtures: the connector suites passed 152 tests, including `worker_retains_cleanup_after_parent_runtime_loss`, bounded terminal-safe stderr and the retained Unix owner scenarios. Windows passed all 128 connector library tests plus six native command cases, including bounded stderr, parent-runtime cleanup retention, post-spawn owner retention, repeated cleanup, configured MCP `.cmd`/`npx` dispatch and owned descendant shutdown. The same Windows job passed the runtime isolation case, source installation, offline-input rejection and acceptance, installed cold offline runtime and PE-import inspection. No stack overflow occurred in the MCP suite or installed binary. Complete logs are `/private/tmp/kuru-pr15-linux-ci34757216251.log`, `/private/tmp/kuru-pr15-macos-ci34757216251.log` and `/private/tmp/kuru-pr15-windows-ci34757216251.log`.
- The normal pre-push hook at `7be6b46f8521c7784f4a0471054286de5fc3d2b9` passed all eight categories; combined LCOV covered 28,002 of 29,350 lines (95.4072%). The final archived descendant head still requires its own normal hook and exact hosted CI before merge.
- The first final-descendant hook exposed one compile-only ownership mismatch after `MemoryStore::close` became consuming in the parent: the MCP isolation fixture closed `harness.memory` and then explicitly dropped the whole harness. The fixture now closes a cloned memory handle before that explicit drop, preserving both the harness cleanup and immediate reopen observation. Runtime typecheck and the exact `review_tests::unavailable_mcp_status_stays_out_of_provider_input_and_memory` real-Dolt/loopback test passed; the first sandboxed test attempt was rejected at loopback bind with `EPERM`, and the authorized host rerun passed 1/1.
