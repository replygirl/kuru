## 1. Configuration and protocol foundations

- [x] 1.1 Add bounded MCP OAuth configuration, semantic validation, final-leaf authority claims and `configuration.v1.schema.json` parity, and verify accepted preregistered/CIMD/DCR examples plus authoritative scope restriction, non-authorization static headers, authorization-header conflict, literal-secret, STDIO, disabled-activation, unknown-field and bound failures on both paths.
- [ ] 1.2 Pin only the required current OAuth/native-store dependencies with matching workspace and lockfile changes, and verify supported target feature selection plus license/provenance checks through owning package tasks.
- [x] 1.3 Implement bounded protected-resource, authorization-server, registration, token, device and revocation types with redacted Debug/errors, and verify hostile JSON, URL, redirect, RFC 9207 issuer-response matrix, resource, authoritative initial/step-up scope and body-limit cases as pure protocol tests.

## 2. Native secret storage

- [x] 2.1 Add the small `kuru-platform` exact-record get/expected-absence-create/generation-replace/generation-delete interface and typed not-found/stale/denied/unavailable/corrupt outcomes, and verify the platform-neutral contract with an isolated synthetic backend.
- [ ] 2.2 Implement macOS Keychain Services and Windows Credential Manager records under Kuru's exact service/account namespace, and verify native create/reopen/stale-replace/delete/cleanup plus adjacent foreign-record isolation on their supported CI hosts.
- [ ] 2.3 Implement Linux Secret Service records with an actionable unavailable path when no usable collection/session exists, and verify native persistence on a supported fixture plus locked/missing-service behavior without a plaintext or session fallback.
- [x] 2.4 Add connector-owned nonsecret cross-process serialization plus the bounded versioned prepared/committed-cleanup/stable manifest and generation-named chunk credential format with exact binding, generation, complete-length and digest validation, and verify interrupted publication and retired-secret cleanup recovery, login/refresh/logout races, stale publication refusal and cancellation after accepted rotation.

## 3. OAuth discovery and grants

- [x] 3.1 Implement RFC 9728 challenge/well-known resource discovery and ordered RFC 8414/OIDC issuer discovery with bounded validated redirects, and verify exact request sequences plus mismatch/no-secret-forwarding refusals against local HTTP fixtures.
- [x] 3.2 Implement configured preregistration, advertised CIMD and advertised compatibility DCR in priority order, sending DCR `application_type=native` and storing returned secrets natively, and verify overlap, unavailable mechanism, callback registration and changed-issuer/client cases.
- [x] 3.3 Implement owned loopback authorization code with single-use state, PKCE S256, exact callback validation and RFC 8707 resource on authorization/token requests, and verify success, denial, spoofed/duplicate callback, listener conflict, expiry and cancellation.
- [x] 3.4 Implement explicitly selected advertised RFC 8628 device authorization with bounded server-paced polling, and verify pending/slow-down/success/denial/expiry/cancellation plus no request when capability is absent.
- [x] 3.5 Implement bound access snapshots, expiry/invalid-token refresh and one explicit insufficient-scope step-up whose scope union preserves prior grants under the configured ceiling, and verify rotated-token settlement, exact alias/resource/scopes and no replay after ambiguous refresh or MCP tool dispatch.
- [x] 3.6 Implement advertised RFC 7009 revocation followed by unconditional exact local deletion with separate outcomes, and verify success, no endpoint, refusal, lost reply, hostile body and concurrent refresh/login.

## 4. MCP catalog and transport integration

- [x] 4.1 Integrate OAuth into `McpClient` session initialization, discovery, calls and close while keeping static-authorization and STDIO paths separate, permitting validated non-authorization headers only on protected resource requests, and verify every captured request has only its alias's authorized headers and a valid live list remains the sole route publication event.
- [x] 4.2 Extend cache context and alias status with nonsecret OAuth binding/generation/scope facts, and verify changed authority/login/logout makes old metadata ineligible without treating authorization or stale metadata as execution permission.
- [ ] 4.3 Exercise mixed disabled, live, stale, degraded, login-required, refresh-required and native-store-unavailable aliases through `ToolCatalog`, and verify one failing/authenticating alias never hides, authorizes or mutates another.

## 5. Commands and documentation

- [x] 5.1 Add one typed `mcp login|status|logout` CLI family and shared `/mcp` TUI registry handler with browser/no-browser/device options, and verify help/completion/parsing/dispatch parity, exact-root trust preflight, no provider turn and completed-frame real-PTY output.
- [x] 5.2 Route desktop URLs through the existing platform handoff and print truthful no-browser callback reachability/device guidance, and verify opener failure leaves the bounded URL usable while cancellation releases listener/polling resources.
- [x] 5.3 Update curated configuration, tools, authentication and protocol documentation with official stable MCP/RFC provenance, native-store setup, headless behavior, route separation, refresh/no-replay and local-versus-remote revoke semantics, and verify docs format/build/content/link checks.

## 6. Integrated verification and delivery

- [ ] 6.1 Run the deterministic fake authorization/resource matrix and secret-exclusion scan across config, cache, status, stdout, stderr and errors, and record exact observed results in `verification.md` without live or paid provider calls.
- [ ] 6.2 Run granular format, lint, typecheck and affected core/platform/connectors/TUI suites plus real native keychain and PTY cases on macOS, Linux and Windows, and record each executed count/result separately from unavailable hosted evidence.
- [ ] 6.3 Run combined workspace coverage with its package-owned instrumented fixtures, retain the 90% gate, inspect practical login/status/logout terminal output, and record the current official-protocol comparison.
- [ ] 6.4 Complete every verification row, strict-validate, confirm the apply gate, archive the change through Cospec, run normal hooks and hand Delivery the clean scoped commit with exact native/CI evidence.
