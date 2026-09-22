## 1. Disabled and filtered aliases never gain transport or routes [critical]

- [x] 1.1 @integration (agent) configure disabled stdio and HTTP aliases with launch/connect witnesses, then inspect both catalog surfaces -> observed: `disabled_and_filtered_aliases_never_publish_routes_or_omitted_metadata` reported both disabled aliases without touching either witness and published only the admitted live tool
- [x] 1.2 @integration (agent) discover overlapping allow/deny and unmatched original names, then attempt their projected and guessed routes -> observed: deny-first filtering retained only `read_public`; denied, unmatched, and guessed names had no route or transport effect
- [x] 1.3 @regression (agent) invoke one admitted live MCP tool through `ToolHost` with allow, ask/deny, and a valid grant -> observed: focused ToolHost deny-before-dispatch, live HTTP selector/result, and foreground Once approval fixtures passed 3/3 with one transport call

## 2. Persistent catalogs are bounded metadata, not authority [critical]

- [x] 2.1 @integration (agent) discover a real fixture catalog, close it, reopen from the same private data/project context while the fixture is offline, and inspect then call -> observed: offline restart projected one stale tool, installed no selector, and both selector lookup and execution refused without transport
- [~] 2.2 @integration (agent) vary endpoint, workspace/root identity, reviewed manifest, filters, header reference, and resolved header value across isolated reopen cases -> defer: context/root mismatch and substitution were observed, and a resolved-header change closed the old captured session before rediscovery with the new value; the remaining full one-variable-at-a-time cache matrix is assigned to final integration coverage
- [x] 2.3 @integration (agent) inject wrong-version, truncated, malformed, over-limit, and hostile-schema private records while a fixture server is healthy -> observed: cache boundary group passed 5/5 and healthy corrupt-cache rediscovery replaced the invalid record while retaining a live route
- [x] 2.4 @regression (agent) cancel or fail discovery and cache publication at each owned boundary -> observed: unpublished staging stayed invisible, failed later pages published no partial route, cache-write failure retained the live route, and explicit rediscovery recovered

## 3. Static HTTP headers remain referenced, redacted, and session-complete [critical]

- [x] 3.1 @integration (agent) run a bounded local Streamable HTTP MCP fixture through initialize, list, call, protocol-version request, and DELETE close -> observed: all five captured requests carried the referenced header; only post-initialize requests carried the negotiated protocol header; a second fixture changed the resolved headers, observed DELETE under the old fixed header set, and observed the complete successor session under the new fixed set
- [x] 3.2 @integration (agent) exercise missing, non-Unicode, oversized, invalid-value, reserved-name, and stdio-header configurations -> observed: native/schema validation and header-boundary fixtures refused invalid declarations or degraded before dispatch with value-free diagnostics
- [~] 3.3 @regression (agent) inspect effective config, CLI/TUI status, diagnostics, tool errors, and private cache bytes with a recognizable fixture secret -> defer: errors and private cache bytes were inspected and value-free, but one composite recognizable-secret projection probe across every CLI/TUI surface remains assigned to final integration coverage

## 4. CLI and TUI share truthful MCP inspection [critical]

- [~] 4.1 @e2e (agent) run `kuru tools` and registered `/tools` against mixed live, stale, degraded, and disabled fixture aliases -> defer: connector mixed-state projection and both shared empty-catalog surfaces passed; `kuru tools` also succeeded with a hostile memory path and left it byte-identical, proving inspection does not open project memory; the app harness still lacks one mixed-state CLI/TUI fixture, assigned to final integration coverage
- [~] 4.2 @e2e (agent) drive a real PTY through `/help`, completion, and `/tools` -> defer: deterministic real PTY completed `/help` and `/tools` and rendered the shared JSON frame, but its fixture has no stale/live MCP pair; state distinction remains assigned to final integration coverage

## 5. Schema, documentation, and compatibility remain exact

- [x] 5.1 @regression (agent) validate accepted and rejected TOML against the native parser and published JSON schema -> observed: focused native and schema parity cases passed 2/2 with default compatibility and matching control rejection
- [x] 5.2 @regression (agent) build and check the public documentation -> observed: app-owned format, lint, VitePress build, generated artifacts, local links, and anchors passed
- [~] 5.3 @regression (agent) run connector, core, runtime, and TUI all-target typecheck/lint plus combined coverage on the final tree -> defer: all four affected all-target typecheck and lint tasks passed before closeout review, and the final header/memory/cache corrections passed the combined connector/TUI all-target typecheck; post-correction lint and combined final-tree coverage are owned by Delivery's normal push hook after the integration restack

## 6. Alias-local degradation and recovery remain isolated [critical]

- [x] 6.1 @integration (agent) combine healthy live, valid offline-cache, corrupt-cache, failed, and disabled aliases in one catalog -> observed: the mixed-alias fixture independently reported live, stale, degraded corrupt/failed, and disabled states while retaining only live and valid stale tools
- [x] 6.2 @integration (agent) restore a previously failed or corrupt-cached server and request explicit discovery again -> observed: failed-alias explicit recovery and corrupt-cache repair both became live without replay, and independent healthy aliases stayed usable
- [~] 6.3 @eval (agent) give the deterministic provider fixture one live, one stale, and one denied MCP proposal in a single response -> defer: exact call IDs and individual live/stale/denied behavior are covered separately, but the runtime harness has no single-response mixed MCP fixture; assigned to final integration coverage
