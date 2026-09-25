## Context

P12 leaves each configured MCP alias behind one connector-owned client, filters discovery before route construction, and separates live routes from owner-private stale catalog metadata. HTTP aliases can apply static headers resolved from trusted environment references, but there is no OAuth state or credential store. CLI `tools` and TUI `/tools` already render one typed alias projection through a shared command registry.

The stable MCP 2026-07-28 authorization specification applies to HTTP transports and requires RFC 9728 Protected Resource Metadata, OAuth/OIDC authorization-server discovery, resource indicators, PKCE, RFC 9207 authorization-response issuer validation and audience-bound bearer use. It prefers configured registration, then advertised Client ID Metadata Documents, with advertised DCR retained as a deprecated compatibility fallback. RFC 8628 device authorization and RFC 7009 revocation are optional OAuth capabilities rather than core MCP requirements, so Kuru can use them only when metadata advertises usable endpoints. This change applies the current authorization requirements to Kuru's existing negotiated MCP transport; it does not implement or claim the stateless 2026-07-28 JSON-RPC transport. The product roadmap separately requires supported-host OS keychain storage and truthful headless behavior.

Kuru's OpenAI authentication manager uses a provider-specific private file and fixed upstream protocol. MCP authentication must not read, reuse, migrate or delete that store. Shell remains process authority; native secret storage reduces ordinary file/config exposure but is not a same-user process sandbox.

## Goals / Non-Goals

**Goals:**

- Keep OAuth protocol and tokens inside connectors while `kuru-platform` owns only a small native secret-store and desktop browser-handoff boundary.
- Preserve P12 alias isolation, workspace trust, cache/route separation, bounded HTTP behavior and static-header operation.
- Make login, refresh, step-up, cancellation, local deletion and advertised remote revocation deterministic enough for fake-server and native-store fixtures.
- Support desktop, no-browser loopback and advertised device authorization without making a false callback-reachability claim.

**Non-Goals:**

- OAuth for STDIO servers, MCP resources/prompts/sampling, client-credentials automation, enterprise authorization extensions, token passthrough, a generic identity service, or a generic secret-management CLI.
- Importing another application's credentials, moving OpenAI credentials, accepting literal tokens, adding a plaintext/session fallback, or claiming containment from same-user shell processes.
- Automatic replay of tool calls, authorization requests with accepted ambiguous outcomes, or rotating refresh requests whose response was lost.

## Decisions

### 1. Add one alias-scoped connector OAuth state machine

`kuru-connectors` will own bounded types for protected-resource metadata, authorization-server metadata, registration, grants and redacted status. Each enabled HTTP alias may use the existing static-header context or an OAuth manager whose immutable binding includes the configured alias, reviewed authority digest, normalized endpoint/canonical resource, selected issuer, client identity and granted scopes. An OAuth alias rejects static `Authorization` and `Proxy-Authorization` header references; other already-valid non-authorization headers remain available only for protected MCP resource requests. The HTTP request builder receives only a short-lived opaque credential snapshot for that exact binding and attaches it only to that alias.

Discovery first consumes a valid 401 challenge, then uses the specification's ordered well-known fallback. Each request has a bounded body, redirect count, connect/total deadline and HTTPS target validation; local fake fixtures use explicit loopback allowances. Protected-resource discovery, authorization-server discovery, registration, authorization, device, token and revocation requests do not inherit MCP static headers or an earlier bearer token. A successful live list is still the only operation that installs routes.

Rejected alternative: a process-global OAuth client or generic header injector. Either would make cross-alias token reuse easy and blur P12's route/authentication boundary.

### 2. Select registration and user flow from explicit configuration plus discovered capability

Registration order is configured preregistration, a configured HTTPS client-ID metadata identity when the issuer advertises CIMD, then deprecated compatibility DCR when a registration endpoint is advertised. DCR sends `application_type=native` together with the exact owned loopback redirect and binds the result to the issuer. A client secret for preregistration is read only from its configured environment reference after workspace trust; returned DCR secrets enter the native store. Kuru will not manufacture a public client ID or post to an inferred registration endpoint.

Initial scope authority follows MCP 2026-07-28 exactly. The initial `WWW-Authenticate` `scope` set is authoritative when present. Otherwise the protected-resource metadata `scopes_supported` set is the baseline, and absence of both omits `scope`. A nonempty configured scope list is a restrictive allowlist; its default empty value adds no configuration ceiling. A configured allowlist never adds to or replaces a challenge, every authoritative challenge scope must be allowed, and without a challenge Kuru requests only the intersection with `scopes_supported`. A scope outside that configured ceiling fails before browser/device authorization. Step-up requests the bounded union of previously granted scopes and the new authoritative challenge, subject to the same configured ceiling; it never drops an existing grant or substitutes metadata scopes for the challenge.

For authorization code, Kuru binds an IPv4 loopback listener before fixing the redirect URI, then constructs the registration/authorization request with exact redirect, state, PKCE S256 and resource. Before redeeming any returned code or acting on an authorization error, it applies the RFC 9207 matrix to the metadata's `authorization_response_iss_parameter_supported` value and the callback's literal `iss`: advertised support requires presence and exact simple-string equality, an unadvertised present issuer must still match, and only unadvertised absence may proceed. No-browser changes only desktop URL handoff: the command explains that the callback must reach the Kuru host, including through deliberate forwarding. Explicit device mode is available only with an advertised RFC 8628 endpoint and grant; its polling follows expiry, interval and `slow_down` under one deadline.

Rejected alternative: treat a printed loopback URL as a headless flow or use device authorization whenever a token endpoint exists. The first is unreachable from many remote browsers; the second assumes an extension the issuer did not advertise.

### 3. Use native secret facilities behind one platform interface

`kuru-platform` will expose exact get plus generation-checked publish and delete for one bounded opaque service/account item. Publishing with expected absence creates only when no item exists; an existing item makes the create stale. Publishing with an expected generation replaces only that exact generation, and delete likewise requires the expected generation. Implementations use macOS Keychain Services, Windows Credential Manager and Linux Secret Service. Each item fits Windows Credential Manager's documented 2,560-byte credential-blob limit, including Kuru's item envelope. Unsupported, locked, denied and missing-store outcomes remain typed and distinct from not-found. The interface neither enumerates arbitrary records nor receives workspace paths.

The connector stores one bounded logical credential containing token material, returned DCR secret when any, and its binding metadata. Its compact binary envelope has an exact 67,899-byte maximum: fixed version, authority/project/alias/resource/issuer binding digests, registration kind and expiry; one length-prefixed client ID at the accepted 2,048-byte maximum; 64 length-prefixed scopes at 256 bytes each; and length-prefixed client-secret, access-token and refresh-token fields at the accepted 16,384-byte maximum each. This avoids JSON escaping growth and admits every value accepted by the protocol validators. At the cross-platform 2,524-byte native payload limit it uses at most 27 chunks.

The logical credential may span generation-named native items; a small versioned native manifest is the sole publication point and binds the complete byte length, digest, generation and chunk count. A descriptor is exactly 53 bytes: a nonzero 16-byte logical generation, a `u32` complete length, a 32-byte digest and a `u8` chunk count bounded to 1–27. Chunk accounts are exact 64-character lower-hex digests derived from the fixed manifest account, logical generation and chunk index, so recovery never enumerates native records. The largest manifest is 124 bytes: a 16-byte magic, one-byte version, one-byte state and two descriptors. With the platform's 36-byte envelope this is 160 bytes, below the exact 2,560-byte native-item maximum; tests assert both this maximum and every accepted item/account bound.

Replace publication has three explicit states under the connector lock. `prepared(old,new)` retains the old readable descriptor while the complete new chunks are written and verified; recovery may complete or roll back. `committed-new(new,retired-old)` makes only the complete digest-verified new descriptor readable and forbids rollback while retaining the exact old cleanup descriptor. Only after every old chunk is confirmed absent does recovery publish `stable(new)`. Create uses `prepared(no-old,new)` after proving manifest absence, then stable-new. Delete publishes `committed-deletion(retired-old)`, making absence authoritative, deletes and confirms every exact old chunk, then generation-checks removal of the manifest itself. Every reader and expected-absence create acquires the same checked owner-private lock, recovers a non-stable manifest first, and then applies manifest-generation compare-and-swap semantics. Recovery therefore always preserves the old usable generation, completes a fully verified new generation, or completes authoritative deletion; it cannot expose a partial/mixed credential, lose the descriptor needed to retire secrets, or let a stale refresh resurrect logout. Blocking native calls run through bounded platform operations and, after a rotating response is accepted, replacement drains independently of caller cancellation. If native publication fails, the login/refresh fails without an in-memory credential fallback and reports whether relogin is required.

Rejected alternative: reuse the existing OpenAI credential file or add an encrypted file fallback. The former mixes routes that canon keeps separate; the latter creates key-management and plaintext-recovery promises outside this scope.

### 4. Bind catalog persistence to nonsecret OAuth context

P12's catalog cache fingerprint will include OAuth mode, configured registration fields, canonical resource, selected issuer, client identity, granted-scope digest and a nonsecret credential generation. It will never include raw tokens, codes or client secrets. Refresh within the same binding advances the generation only after a live authorized session; logout makes old cached OAuth metadata ineligible for projection. Static-header fingerprints and session-fixed header maps remain unchanged.

Rejected alternative: hash access/refresh token values into the cache. That unnecessarily invalidates catalog metadata on ordinary rotation and expands the secret-derived persistence surface.

### 5. Refresh and authorization recovery obey the existing no-replay boundary

Before an MCP operation body is accepted, an expired token or one invalid-token 401 can trigger one serialized refresh and a new HTTP attachment. A 403 insufficient-scope challenge can start one explicit step-up whose requested set is the bounded union of the authoritative challenge and previously granted scopes, constrained by the configured allowlist, under the normal interaction/cancellation boundary. Once a tool call may have been accepted, Kuru returns the existing ambiguous MCP outcome and never refreshes then replays it. A refresh request that might have consumed rotating material is not repeated; reconciliation uses the native record generation or requires relogin.

Rejected alternative: hide auth recovery in a general HTTP retry layer. That cannot distinguish safe pre-body discovery from possibly effective tool calls or rotating token POSTs.

### 6. Commands call the same connector control plane

Add `kuru mcp login|status|logout <alias>` and a `/mcp ...` registry entry backed by the same typed operation. Commands resolve the immutable configuration snapshot, pass exact-root trust preflight, and then avoid memory/provider construction. Status combines P12 availability with redacted auth state. Logout serializes with refresh/login, attempts RFC 7009 only for the exact bound advertised endpoint, then deletes the local record and reports local and remote outcomes separately.

Rejected alternative: independent CLI and TUI auth clients. Separate clients would drift on discovery, redaction, cancellation and revocation semantics.

## Risks / Trade-offs

- [Native credential facilities behave differently and may be absent on headless Linux] → keep a typed platform contract, verify real supported-host behavior, make unavailable actionable, and retain static headers without a fallback credential file.
- [Discovery metadata can redirect or point at an attacker] → validate bounded HTTPS URLs, issuer/resource relationships and every redirect; never forward bearer/static credentials during discovery.
- [Multiple aliases share one issuer] → retain alias/resource/client binding in both native records and request snapshots; never look up by issuer alone.
- [A cancelled rotating refresh can consume the old token] → allow cancellation before dispatch, then drain and generation-check native publication after an accepted response; never blindly retry an uncertain request.
- [DCR or callback redirect registration may not support the selected loopback URI] → bind before presenting the request and fail with mechanism-specific setup guidance; do not fall through to a weaker registration or unowned listener.
- [Remote revocation may be unsupported or fail after local deletion] → report its exact advertised outcome separately and never promise server-side revocation.

## Operational surface

Browser authorization owns one IPv4 loopback listener for a single bounded login and releases it on success, refusal, expiry or cancellation. It opens a URL through the existing desktop platform handoff without owning the browser process. No-browser mode opens no desktop process. Device authorization owns no listener and polls only the exact advertised endpoint within the returned expiry and server pacing.

The only required secret facilities are the host's canonical user credential service and, for configured confidential preregistration, an explicitly named environment variable. No token or client secret enters a runner environment by default. Native CI uses isolated Kuru service/account names with synthetic credentials and deletes them; HTTP fixtures use loopback authorization/resource servers. Supported release architectures remain the repository's current macOS/Linux arm64/x86-64 and Windows x86-64 set. Metadata, callback, registration, token, refresh and revocation bodies and deadlines are finite and connector owned.

## Integration contract

`kuru-core` owns the typed OAuth configuration, semantic validation, JSON-schema parity and authority claims. `kuru-platform` owns native credential CRUD and browser handoff without OAuth or MCP types. `kuru-connectors` owns discovery, registration, OAuth state, token binding, refresh/revocation, secret-record codec and integration with the existing `McpClient`/`McpCatalog`. `kuru-tui` owns argument parsing and presentation only.

Local fake servers exercise exact HTTP challenge, metadata, registration, authorization, device, token, refresh, revocation and protected MCP requests; their synthetic secrets are asserted absent from stdout/stderr/status/cache/config. Platform fixtures exercise real store create/replace/reopen/delete/unavailable behavior under isolated names. CLI and real-PTY fixtures prove the same command registry and completed-frame status. No MCP protocol JSON-RPC type or Dolt schema is changed.
