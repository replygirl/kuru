## ADDED Requirements

### Requirement: HTTP MCP authorization follows discovered OAuth authority

For an OAuth-enabled HTTP MCP alias, Kuru SHALL discover Protected Resource Metadata from a valid `WWW-Authenticate` challenge or the ordered RFC 9728 well-known locations, select an advertised authorization server, and obtain OAuth or OpenID authorization-server metadata through the current stable MCP 2026-07-28 discovery order. Kuru MUST validate bounded HTTPS metadata and endpoint URLs, require an exact issuer match, keep discovery requests free of MCP bearer and static-header credentials, and refuse metadata redirects or documents that escape the validated authority rules. These authorization requirements apply compatibly to Kuru's existing negotiated MCP transport and do not assert full 2026-07-28 wire-protocol support. STDIO aliases MUST NOT use this flow.

#### Scenario: Protected server supplies a valid challenge
- **WHEN** an OAuth-enabled HTTP alias returns 401 with a bounded valid `resource_metadata` challenge and its protected-resource and authorization-server documents agree
- **THEN** Kuru selects the advertised issuer and endpoints for only that alias without sending an existing token or configured static header to a discovery host

#### Scenario: Discovery authority is inconsistent
- **WHEN** the challenge, protected resource, issuer metadata, endpoint scheme, redirect target or canonical resource is malformed, over-limit or inconsistent
- **THEN** Kuru refuses authorization before opening a browser, registering a client or storing a credential and reports a bounded alias-local diagnostic

### Requirement: Client registration uses supported mechanisms in stable order

Kuru SHALL prefer configured preregistered client information for an alias, otherwise use a configured HTTPS Client ID Metadata Document only when the authorization server advertises `client_id_metadata_document_supported`, and otherwise use deprecated compatibility Dynamic Client Registration only when the server advertises a registration endpoint. DCR requests MUST declare `application_type` as `native`. Kuru MUST bind registered client information to the exact alias, canonical resource and issuer, store returned client secrets only through the native secret store, and MUST NOT invent a universal client identifier or submit registration to an unadvertised endpoint.

#### Scenario: Advertised registration choices overlap
- **WHEN** an alias has valid preregistered client information and its issuer also advertises CIMD and DCR
- **THEN** Kuru uses the configured registration and performs neither metadata-document fallback nor dynamic registration

#### Scenario: No registration mechanism is usable
- **WHEN** no configured registration is valid and the issuer advertises neither a usable configured CIMD nor DCR
- **THEN** login stops with setup guidance before authorization and stores no client or token material

### Requirement: Interactive authorization binds state code and resource

Kuru SHALL implement authorization-code login with cryptographically random single-use state and PKCE S256, require advertised S256 support, bind an owned loopback listener before presenting its exact redirect URI, validate the callback method, host, path, state, RFC 9207 issuer response and unique code, and include the same canonical RFC 8707 `resource` in authorization and token requests. Before code redemption or acting on an authorization error, advertised issuer-response support MUST require a present exact issuer, an unadvertised but present issuer MUST still match exactly, and only an unadvertised absent issuer MAY proceed. Authorization endpoints and redirect URIs MUST satisfy the stable MCP transport security rules.

#### Scenario: Browser login succeeds
- **WHEN** the user completes a valid authorization response for the owned callback, exact state, issuer, alias and resource
- **THEN** Kuru exchanges the code once with its original redirect URI and verifier, validates the token response, stores it in the native secret store and retries only the authorization-gated discovery operation

#### Scenario: Callback is forged or duplicated
- **WHEN** a callback has the wrong host, path, state or issuer, repeats a code, exceeds a bound, or arrives after cancellation or expiry
- **THEN** Kuru rejects it without token exchange or credential publication and releases the listener at the bounded end of the login

### Requirement: Initial and step-up scopes preserve authoritative least privilege

For initial authorization, Kuru SHALL treat the initial `WWW-Authenticate` `scope` set as authoritative when present, otherwise use protected-resource `scopes_supported`, and otherwise omit `scope`. A nonempty configured scope list SHALL be a restrictive allowlist: it MUST NOT add to or override an authoritative challenge, MUST constrain metadata fallback by intersection, and MUST refuse a required challenge scope outside the configured ceiling. The default empty list SHALL add no configuration ceiling. A step-up request SHALL use the bounded union of previously granted scopes and the new authoritative challenge, subject to that same configured ceiling when configured.

#### Scenario: Challenge and metadata scopes differ
- **WHEN** the initial challenge supplies a bounded scope set that differs from protected-resource `scopes_supported`
- **THEN** Kuru requests the exact challenge set if every member is allowed and does not substitute or add metadata scopes

#### Scenario: Step-up preserves prior grants
- **WHEN** an authorized alias receives an insufficient-scope challenge within its configured ceiling
- **THEN** Kuru requests the exact bounded union of prior grants and challenged scopes without dropping prior grants or adding unrelated configured scopes

### Requirement: Headless and device login report reachable behavior

Kuru SHALL support RFC 8628 device authorization only when the authorization-server metadata advertises a valid device authorization endpoint and the device-code grant. It SHALL display the verification URI and user code, honor the server's bounded expiry and polling interval including `slow_down`, and stop on denial, expiry or cancellation. A no-browser authorization-code flow SHALL state that its loopback callback must be reachable on the host running Kuru and MUST NOT describe a printed loopback URL as remotely reachable; when device flow is advertised the user can explicitly select it.

#### Scenario: Advertised device flow succeeds
- **WHEN** a headless user selects device login and the issuer advertises RFC 8628 support
- **THEN** Kuru shows the bounded verification details, polls no faster than allowed, binds the token to the same issuer/resource/alias and stores it only after success

#### Scenario: Remote browser cannot reach loopback
- **WHEN** the user requests no-browser authorization-code login
- **THEN** Kuru prints the URL together with accurate same-host or forwarded-loopback reachability guidance and cancellation instructions rather than claiming that another machine can complete the callback directly

### Requirement: Tokens refresh and step up without crossing aliases

Kuru SHALL attach a bearer token only to the exact HTTP MCP alias, canonical resource, issuer and client binding that produced it and SHALL include it on every request in that alias's current HTTP session. Kuru SHALL refresh before expiry or after one pre-body invalid-token response when a refresh grant is available, preserve rotated refresh material through safe settlement, and handle an authoritative insufficient-scope challenge through one bounded interactive step-up. It MUST NOT move tokens, scopes or authorization headers between aliases or resources, replay an accepted or ambiguously dispatched MCP call, or blindly repeat an uncertain rotating refresh request.

#### Scenario: Expired token refreshes
- **WHEN** one alias has an expired access token and a valid refresh token bound to its unchanged issuer/resource/client context
- **THEN** Kuru sends one bounded refresh request containing that resource, atomically replaces the stored generation after a valid response and resumes discovery without exposing either token

#### Scenario: Resource or issuer changes
- **WHEN** an alias endpoint, protected resource, authorization issuer or client identity differs from the stored binding
- **THEN** Kuru withholds the credential, reports login required for that alias and leaves other aliases and the old credential record isolated

#### Scenario: Tool call authorization becomes ambiguous
- **WHEN** a bearer-authenticated tool call may have been accepted before transport failure or cancellation
- **THEN** Kuru does not replay it automatically and reports the same bounded ambiguous-call outcome used by the MCP connector

### Requirement: Logout deletes locally and reports remote revocation exactly

Kuru SHALL serialize logout with refresh/login publication for the selected alias, attempt RFC 7009 remote revocation only when the bound authorization metadata advertises a valid revocation endpoint, and always remove the selected Kuru-owned local MCP credential after the bounded remote attempt settles. Status and logout output MUST distinguish remote success, refusal/failure, unavailable native storage, no advertised endpoint and absence of a local credential without displaying secret material.

#### Scenario: Advertised revocation succeeds
- **WHEN** a selected alias has a stored refreshable credential and its bound issuer advertises a revocation endpoint that accepts the request
- **THEN** Kuru reports remote revocation success, deletes the local record and sends the credential nowhere else

#### Scenario: Remote revocation is absent or fails
- **WHEN** the issuer did not advertise revocation or its bounded revocation request refuses or fails
- **THEN** Kuru reports that exact remote outcome, deletes the local record, and does not claim that the server-side grant was revoked
