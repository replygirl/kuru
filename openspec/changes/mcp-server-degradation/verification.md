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
- [ ] 3.3 @integration (agent) exercise timeout, natural root exit, caller loss and parent-runtime loss with a same-group descendant retaining stderr on native macOS and Linux -> worker confirms ordered cleanup or honestly retains the owner without a post-reap signal
- [ ] 3.4 @integration (agent) run equivalent caller/runtime-loss, stderr-flood and descendant-held-pipe fixtures with the Windows Job owner -> Job/tree and pipes become quiescent without inferring parity from Unix

## 4. Shutdown and presentation [critical]

- [x] 4.1 @integration (agent) race shutdown with starting, active and idle clients, then attempt discovery and invocation -> no new client is admitted after closure, every admitted client is attempted within one aggregate bound, and successful shutdown confirms its cleanup before return
- [x] 4.2 @e2e (agent) run real `kuru tools`, `kuru tool`, plain run and JSON run with a failed alias and fake stderr credential -> stdout remains shape-compatible, only direct human stderr receives the safe tail, and runtime JSON contains fixed existing-shape events
- [x] 4.3 @eval (agent) inspect captured provider requests, tool errors, event JSON and reopened memory after failure -> command/env/endpoint/raw stderr sentinels are absent and only fixed alias status is retained

## 5. Quality and documentation

- [x] 5.1 @unit (agent) exercise Scanner with a cleared reusable output buffer and long cumulative input -> destination capacity stays bounded by the current chunk while cumulative overflow/growth checks remain effective
- [ ] 5.2 @integration (agent) run focused platform, connector, runtime and TUI tests plus format, lint, typecheck and documentation checks; run workspace coverage once when coordinated -> all executed gates pass, the 90% floor remains intact, and unrun native evidence is explicitly deferred
- [x] 5.3 @integration (agent) build and inspect curated tools/protocol documentation -> public text matches alias degradation, explicit recovery, no replay, human-only stderr and finite-detector/process limits without publishing private notes

## Observed evidence

- macOS connector package: `RUST_TEST_THREADS=2 cargo test -p kuru-connectors --all-targets --all-features --locked --no-fail-fast` exited 0; 150 library tests passed, including 22 MCP and 4 RPC cases. The MCP cases include later-page atomicity, independent-alias progress, pre-ready cancellation, setup-failure retention and starting/active/idle shutdown.
- macOS focused acceptance additions: the exact `catalog_keeps_healthy_stdio_alias_when_http_alias_fails_in_either_order`, `explicit_catalog_recovers_a_failed_alias_without_replaying_its_call` and `received_stdio_mutation_disconnects_once_and_disables_later_calls` tests each exited 0. Together with the original healthy-HTTP/failed-stdio and cancellation cases, they observe both real transport combinations, preserve an independent alias across explicit recovery, and confirm that a real stdio peer receives one mutating call before disconnect while two later calls and shutdown add no requests. The full package suite has not been repeated after these proof-only additions.
- macOS Unix owner: `cargo test -p kuru-platform unix::tests -- --nocapture` exited 0; 7 tests passed. The runtime-loss RPC case used a real same-group descendant retaining inherited stderr and observed explicit cleanup confirmation after the parent Tokio runtime ended.
- Static checks: connector/platform and runtime/TUI `cargo clippy --all-targets --all-features -- -D warnings` exited 0; `cargo fmt --all -- --check` and `git diff --check` exited 0.
- Runtime isolation: `review_tests::unavailable_mcp_status_stays_out_of_provider_input_and_memory` exited 0 against actual durable Dolt, then closed and reopened that store. One failed and one healthy MCP alias produced fixed event metadata and only usable tools in captured provider requests; fake command, environment secret and HTTP endpoint were absent from provider requests and both live and reopened memory.
- Direct CLI: `direct_tools_keeps_stdout_json_and_reports_filtered_failed_stdio` exited 0. Real `kuru tools` and `kuru tool file_list` processes preserved JSON stdout and emitted fixed status plus a recognizable-secret-filtered, terminal-escaped diagnostic on human stderr. Real plain and JSON runs preserved their stdout shapes; JSON used the existing fixed MCP event, and neither run exposed the diagnostic, command or fake secret.
- Documentation: `mise run docs:check` exited 0 with pinned Node 26.8.2/npm 12.0.2, oxfmt, oxlint, VitePress build, and delivery content/link inspection.
- Exact-base push gate: all eight normal hook categories exited 0 at `4de6b2a19ea24e458867d765916d697120ed3a47`; combined LCOV covered 27,740 of 29,085 lines (95.3756%). This predates the three proof-only tests above, so their combined coverage remains pending.
- Strict cospec validation and the acknowledged apply gate exited 0 with only the declared `tool-result-redaction` and `owned-unix-shell-lifecycle` soft blockers.

Pending: one coordinated workspace coverage run including the proof-only additions; native Linux and Windows ownership/stderr execution; the unchecked compound rows above whose complete multi-surface fixture composition has not yet run. Cross-compilation is not recorded as native evidence.
