## Context

The existing tool host provides checked filesystem operations, owned shell execution, MCP, centralized permission decisions, bounded receipts, and provider redaction. P05 adds an external HTTP boundary where hostnames may resolve differently between parsing and connection and where response decompression can create a second size boundary.

## Goals / Non-Goals

**Goals:**

- Use a connector-owned client path that can enforce address policy at each actual connection and redirect.
- Keep HTTP limits, cancellation, redaction, and tool receipts observable through existing runtime boundaries.
- Make untrusted-content and credential-isolation behavior testable with local fixtures.

**Non-Goals:**

- Browser automation, web search, image extraction, a general proxy, or an internal-network escape hatch.
- Changing provider authentication, MCP transport behavior, workspace trust semantics, or adding a new process sandbox claim.

## Decisions

### Enforce address policy in the connection path

The fetch client will resolve and validate addresses for the endpoint it is about to connect to, and will repeat that work for every redirect. A URL-preflight DNS check alone is rejected because it cannot prevent DNS rebinding between validation and the socket connection.

### Disable automatic cross-origin credential inheritance

The fetch transport starts from an empty credential/header authority and accepts no provider or MCP headers. Reusing an existing provider client was rejected because its authentication and endpoint configuration would be the wrong authority domain.

### Stream through distinct wire and decoded bounds

The implementation will account separately for compressed bytes accepted from the network and decoded bytes retained or parsed. Buffering the full body before validating text was rejected because it permits decompression and memory amplification before the limits apply.

### Deliver fetched material as tool data

The tool returns a bounded, marked result through the existing receipt projection. Treating page text as a configuration, hook, or prompt-authority source was rejected because it would allow a remote response to expand Kuru's permissions.

## Integration contract

`kuru-connectors` owns the fetch client, resolver policy, HTTP fixture seam, and native-tool registration. The tool request carries one URL and the bounded response projects through the existing tool-result envelope with explicit truncation/omission metadata. Fixture DNS and HTTP servers remain local test dependencies; no provider, MCP, or deployment credential is part of the transport contract.

## Risks / Trade-offs

- [DNS API does not expose a safe connection-time hook] → keep the work behind a small connector transport boundary and add rebind fixtures before choosing a dependency or adapter.
- [Public sites use unexpected content encodings] → support only deliberately bounded, verified decoders and give fixed diagnostics for unsupported or malformed bodies.
- [Useful documents exceed retained limits] → surface truthful truncation metadata while draining/cancelling within the fixed operation budget.
