## 1. Connection-safe transport

- [x] 1.1 Add a connector-owned HTTP(S) fetch transport with explicit URL parsing, connection-time resolver/address validation, redirect revalidation, and fixed redirect/deadline policy; verify local DNS-rebind and redirect fixtures reject every prohibited address class before dispatch.
- [x] 1.2 Enforce separate compressed-wire, decompressed-body, and model-facing retained-content limits using streaming reads and bounded cleanup; verify hostile compression, oversized bodies, slow responses, and cancellation have bounded truthful outcomes.

## 2. Tool authority and result projection

- [x] 2.1 Register `web_fetch` in the native tool schema and route it through the existing centralized permission evaluator and receipt path; verify allow/ask/deny boundaries with a normal ToolHost proposal fixture.
- [x] 2.2 Keep fetch transport credentials and headers independent of provider and MCP clients, and project returned text as untrusted tool data; verify fake credentials never reach the origin and instruction-like documents cannot alter authority.
- [x] 2.3 Preserve redacted, bounded diagnostics and exact cancellation settlement for fetch failures; verify malformed URLs, resolver failures, cancellation, and response faults expose no remote secrets or unbounded content.

## 3. Contract and documentation

- [x] 3.1 Add any required configuration/schema/permission selector support and update user tool documentation with limits, public-destination policy, and untrusted-content semantics; verify schema tests and docs examples agree with the runtime contract.
- [x] 3.2 Run the connector-focused, integration fixture, docs, and normal delivery checks; record observed results in verification.md, archive only after every critical acceptance row has evidence.
