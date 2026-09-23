## 1. Discovery, issuer and registration binding [critical]

- [x] 1.1 @integration (agent) drive loopback protected-resource and authorization servers through challenge, path/root metadata fallbacks, OAuth and OIDC discovery, configured registration, advertised CIMD and advertised compatibility DCR with `application_type=native` -> each supported path selects the exact issuer/client/resource, uses the required priority and bounded request sequence, and invalid/mismatched/unadvertised paths send no authorization or registration request
- [x] 1.2 @regression (agent) serve hostile metadata with wrong issuer/resource, non-HTTPS external endpoints, redirect escape, oversized bodies, multiple authorization servers and unsupported PKCE -> the selected alias fails closed with bounded diagnostics and no credential, browser, route or other-alias effect

Observed on macOS: `mcp_oauth::tests` passed 10/10 in 10.02s with native loopback permission. The module covers exact discovery fallback order, no forwarded secrets, URL/redirect/body bounds, issuer/resource mismatches, PKCE and capability gating.

## 2. Browser, headless and device authorization [critical]

- [ ] 2.1 @e2e (agent) run CLI login against a fake authorization server and the real owned loopback callback across the RFC 9207 advertised/present issuer matrix -> exact redirect/state/PKCE S256/resource/issuer/code exchange succeeds once, wrong, required-missing or repeated callbacks fail before redemption, and cancellation/expiry releases the listener without publishing a credential
- [x] 2.2 @integration (agent) exercise advertised RFC 8628 success, pending, slow-down, denial, expiry and cancellation -> verification details are bounded, polling honors server pacing, only success publishes, and no request occurs when the endpoint/grant was not advertised
- [ ] 2.3 @manual (human) inspect browser and no-browser output in a practical local and remote/headless terminal -> the desktop handoff does not own browser lifetime, and printed guidance accurately states same-host/forwarded-loopback reachability and the advertised device alternative

Observed on macOS: the connector device matrix passed all pending/slow-down/success/denial/expiry/cancellation/absent-capability branches within the 10/10 module result. Manual remote/headless inspection remains pending.

## 3. Native credential storage and lifecycle [critical]

- [ ] 3.1 @runtime (agent) expected-absence-create, race a second stale create, reopen, generation-replace and generation-delete an isolated synthetic Kuru MCP record through the real macOS, Windows and Linux native-store implementations on supported native jobs -> only the exact service/account binding is visible, restart retains it, stale create/replace/delete lose, and cleanup leaves no test record
- [ ] 3.2 @integration (agent) run OAuth with the native facility missing, locked and denied -> login/refresh/status report distinct actionable unavailable states, no plaintext/session fallback appears, and a configured static-header alias continues to work
- [ ] 3.3 @regression (agent) place similarly named synthetic records for another application beside the Kuru fixture -> Kuru does not enumerate, read, migrate, report or delete them

Observed on macOS: `native_store_reopens_replaces_deletes_and_isolates_adjacent_records` passed 1/1 in 0.50s. Linux Secret Service and Windows Credential Manager native execution remain required hosted evidence.

## 4. Refresh, scope step-up and cancellation settlement [critical]

- [x] 4.1 @integration (agent) store a logical credential spanning multiple native items; assert the exact 67,899-byte envelope, 27-chunk maximum, 124-byte two-descriptor manifest and 160-byte complete native item; interrupt before and after prepared, committed-new cleanup, stable-new and committed-deletion publication; then expire a token, return a rotating refresh token, cancel after the accepted response and race a second process/login/logout -> recovery exposes either the complete old or complete new digest-bound generation, retains enough descriptor state to remove every retired secret without enumeration, authoritative deletion cannot be resurrected, one refresh request settles the exact native generation, stale writers lose, and cancellation returns only after safe publication/reconciliation
- [x] 4.2 @integration (agent) vary initial challenge scopes against protected-resource metadata and return invalid-token plus insufficient-scope challenges before an MCP body is accepted -> Kuru performs at most one eligible refresh or explicit step-up, uses the exact allowed initial authority or prior-grant/challenge union, refuses scopes outside the configured ceiling, and recreates only the selected alias session
- [ ] 4.3 @integration (agent) lose the response to a rotating refresh and separately lose/abort a bearer tool-call response after dispatch -> neither request is blindly replayed; refresh requires stored-generation recovery or relogin and the tool call retains the connector's ambiguous-effect result

Observed on macOS: `mcp_credentials::tests` passed 4/4 in 0.09s, including every publication phase, exact envelope/manifest bounds and prepared-collision rollback. `browser_login_settles_native_publication_then_refreshes_and_logs_out` passed 1/1 in 1.03s and proves accepted callback settlement, native rotation, scope step-up and serialized logout. Explicit lost-response execution remains pending in 4.3.

## 5. Alias, resource, header and catalog isolation [critical]

- [ ] 5.1 @integration (agent) configure two OAuth resources on one issuer with non-authorization resource headers plus one static-authorization alias and capture every request -> each bearer/resource/client/scope binding reaches only its exact alias, non-authorization headers appear only on the selected protected resource, no static header enters OAuth authority requests, and OAuth headers never enter the static alias
- [ ] 5.2 @integration (agent) change endpoint, issuer, client identity, reviewed authority, scopes and local credential generation across cached catalog records -> mismatched OAuth metadata is ineligible, logout does not reveal stale authorized tools, and only current live discovery creates a route
- [ ] 5.3 @e2e (agent) inspect a mixed disabled/live/stale/degraded/login-required/native-store-unavailable catalog through CLI and `/tools` -> both surfaces show the same bounded availability plus authorization projection, disabled aliases perform no discovery/store access, and no state grants permission or dispatch

## 6. Login, status and logout command parity [critical]

- [ ] 6.1 @e2e (agent) exercise `kuru mcp login|status|logout` and `/mcp ...` through fake servers and a real PTY synchronized to completed frames -> help/completion/argument parsing and ordered final output agree, commands make no provider request, and the user can continue the same TUI session
- [ ] 6.2 @regression (agent) request unknown, disabled, STDIO and unapproved automatically configured aliases -> every surface refuses before discovery, credential access or process startup and leaves the prior credential/catalog unchanged

Observed on macOS: the typed command parser and CLI authority projection each passed 1/1; `oauth_commands_refuse_unknown_disabled_and_stdio_aliases_before_activation` passed 1/1 in 0.38s with zero HTTP requests and zero STDIO conversations. The completed-frame PTY status/quit fixture passed 1/1 in 4.43s. Fake-server login/logout parity remains pending before this section closes.

## 7. Local deletion and advertised remote revocation [critical]

- [x] 7.1 @integration (agent) advertise a valid RFC 7009 endpoint and accept access/refresh revocation -> logout serializes with refresh/login, sends only the bound credential to the exact endpoint, reports remote success and removes the local record
- [ ] 7.2 @integration (agent) omit the endpoint, refuse it, lose its response and return a hostile body -> each outcome is reported distinctly and safely, local deletion still completes, and Kuru never claims remote revocation without evidence
- [ ] 7.3 @regression (agent) cancel logout before and after remote dispatch while another process refreshes -> the operation retains one coherent local generation outcome and never restores deleted credentials or repeats an uncertain revocation

Observed on macOS: advertised empty-body revocation reports `revoked` and leaves no local record in the integrated browser test. The pure revocation classification fixture passed 1/1 for absent, lost and hostile outcomes; the native implementation unconditionally generation-deletes after each bounded remote outcome. Explicit cancellation/race execution for 7.2/7.3 remains pending.

## 8. Configuration, authority, bounds and secret exclusion [critical]

- [x] 8.1 @integration (agent) validate accepted and hostile OAuth TOML against native parsing and `configuration.v1.schema.json` -> supported configured/CIMD/DCR fields, restrictive scope defaults and non-authorization headers match, while literal secrets, STDIO OAuth, static `Authorization`/`Proxy-Authorization` conflicts, unknown fields, incompatible choices and bound violations fail before activation
- [ ] 8.2 @integration (agent) resolve automatic project OAuth configuration before and after exact-root trust -> no secret environment/native-store/discovery access occurs before approval, changed final-leaf authority invalidates it, and command-specific inspection activates only applicable claims
- [ ] 8.3 @regression (agent) inject recognizable synthetic secrets into every token/client/error field and inspect stdout, stderr, `/tools`, `/mcp status`, config, cache and debug/error projections -> no secret, callback code, raw authorization body or credential-derived identifier is emitted or persisted outside the native record

Observed on macOS: native and JSON-schema parity each passed 1/1, including explicit non-HTTPS resource refusal and static authorization conflicts. Protocol response/debug tests redact recognizable token, client-secret, device-code and hostile error fields, and connector cache identity contains only binding digests/generation. Full stdout/stderr/`/tools`/status scan remains pending.

## 9. Documentation and protocol provenance

- [x] 9.1 @integration (agent) build and check curated docs and configuration examples -> published guidance covers MCP 2026-07-28 authorization discovery/resource/PKCE/issuer/scope rules applied to Kuru's existing negotiated transport, optional advertised device/revocation behavior, native-store setup, callback reachability and static/OpenAI separation without claiming full 2026 wire support or exposing repository evidence or secrets
- [ ] 9.2 @manual (human) compare the implementation constants and fixtures with the pinned official MCP 2026-07-28 authorization page plus RFC 7009, RFC 8252, RFC 8628, RFC 8707, RFC 8414, RFC 9207 and RFC 9728 -> every mandatory or optional behavior is attributed accurately, DCR is identified as compatibility fallback, no optional extension is presented as core MCP, and no whole-protocol 2026 conformance is claimed

Observed on macOS: `mise run //apps/kuru-docs:check` passed formatting, lint, build, content and local-link checks. The artifact comparison cites the official MCP 2026-07-28 authorization page and primary RFCs; final human inspection remains pending.

## 10. Owning checks and supported hosts [critical]

- [x] 10.1 @integration (agent) run granular core/platform/connectors/TUI format, lint, typecheck and deterministic OAuth/command suites through package-owned mise tasks -> all affected crates and feature combinations pass without contacting a paid/live provider
- [ ] 10.2 @runtime (agent) run native macOS, Linux and Windows credential/browser/CLI fixtures plus combined workspace coverage -> supported hosts pass real native behavior, subprocess/listener cleanup is bounded, and workspace line coverage remains at least 90%
- [ ] 10.3 @integration (agent) run strict Cospec validation, apply gate, docs checks and commit hooks -> artifacts remain coherent, dependencies are archived, generated schema is current and no required gate is skipped

Observed locally: affected core/platform/connectors/runtime/TUI all-target/all-feature Clippy passed with `-D warnings`; `cargo fmt --all -- --check` and `git diff --check` passed. Deterministic focused results are recorded above and used no live or paid provider. Linux/Windows native behavior, combined coverage, hooks and archive remain Delivery gates.
