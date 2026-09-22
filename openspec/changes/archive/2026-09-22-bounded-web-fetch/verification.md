## 1. Public-destination authority [critical]

- [x] 1.1 @integration (agent) resolve `public.fixture` through the local resolver/HTTP fixture, then change its next answer to a prohibited address -> `redirect_to_each_prohibited_fixture_destination_stops_before_dispatch` rejects the changed destination before a second dispatch.
- [x] 1.2 @integration (agent) redirect a permitted fixture origin to loopback, private, link-local, multicast, unspecified, and IPv4-mapped IPv6 addresses -> the same fixture rejects all six hops before dispatch and returns no fetched content.
- [x] 1.3 @integration (agent) submit malformed, relative, non-HTTP(S), and credential-bearing URLs -> `url_validation_rejects_unsupported_and_credential_bearing_values` fails before DNS or socket creation.

## 2. Bounded response lifecycle [critical]

- [x] 2.1 @integration (agent) serve redirect loops, slow streams, compressed expansion payloads, and oversized bodies from the bounded local fixture -> `redirect_loop_stops_at_the_fixed_hop_limit` and `fixture_enforces_content_encoding_size_and_deadline_bounds` report fixed limit outcomes within their deadlines.
- [x] 2.2 @integration (agent) cancel an admitted request to a stalled fixture -> `cancelling_a_stalled_fetch_cannot_publish_a_later_body` joins cleanup and observes no later body.
- [x] 2.3 @regression (agent) run the owning connector tool and receipt suite plus the current transport fixtures -> `mise run //packages/kuru-connectors:test` passed 214 tests before the fixture-only additions, and the current ten transport fixtures plus ToolHost boundary fixture pass.

## 3. Authority and untrusted-content boundaries [critical]

- [x] 3.1 @integration (agent) fetch an instruction-like document from the local pinned fixture -> `local_transport_fixture_pins_the_checked_origin_and_drops_credentials` observes only `Accept-Encoding: identity`, no `Authorization` or `Cookie`, and an `untrusted: true` result.
- [x] 3.2 @e2e (agent) invoke `web_fetch` through a real ToolHost proposal under allow, deny, and ask decisions -> `web_fetch_uses_the_normal_allow_ask_and_deny_receipt_boundaries` reaches destination policy for allow, blocks before fetch for deny, and records the native-web-fetch ask denial receipt.
- [x] 3.3 @eval (agent) project instruction-like fixture text through the existing `ToolExecution::Json` path -> the typed `untrusted: true` content has no route to permission evaluation or further dispatch.

## 4. Documentation and schema [critical]

- [x] 4.1 @integration (agent) exercise permission schema and parser coverage for the selector and independent fallback -> `packages/kuru-core/tests/permissions.rs` passes and the published schema, tools, configuration, and TUI labels include `web_fetch`.
- [x] 4.2 @integration (agent) run the owning documentation check after formatting the public schema -> `mise run docs:check` passes VitePress build, local links, anchors, and content checks.

## 5. Delivery verification [critical]

- [x] 5.1 @regression (agent) run the sole normal workspace coverage writer on the final source -> the 702.55-second writer succeeds and saves `target/coverage.lcov`.
