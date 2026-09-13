## Context

`McpHosts::specs` currently builds one all-or-nothing route map. `McpClient::list` releases its transport mutex between pages, stdio RPC discards stderr, and cancellation after a request write can leave the same transport reusable. Unix RPC separately retains a numeric group and a Tokio child, permitting root reap before a later numeric group signal. Existing source already supplies a finite streaming secret scanner, a typed ToolHost projection, a non-clone Unix process-group owner, a Windows Job owner, and generic runtime/TUI events.

The retained-root contract requires revalidation immediately before any stdio process creation using the workspace pathname. Moving launch to a worker therefore moves that exact `Directory` and the final revalidation into the worker; catalog entry, HTTP operations, and calls on an existing stdio session acquire no new cwd authority.

## Goals / Non-Goals

**Goals:** Keep healthy aliases usable, make an alias's catalog and call state coherent, prevent ambiguous replay, retain process authority through cleanup, and provide bounded human-only stderr diagnostics.

**Non-Goals:** Catalog caching, health polling, configurable timeouts, retry, status persistence, tool generations, a general worker framework, or stronger filesystem/process containment.

## Decisions

### One existing logical lock per alias

Replace the transport mutex value with client state and hold it for a complete logical operation plus short route publication. An async cancellation guard also has a synchronous availability bit so caller loss immediately prevents later dispatch. A global discovery lock and generation/CAS scheme were rejected because independent aliases need no shared network critical section and a catalog is already a point-in-time result.

### Complete alias candidates and inactive prior routes

Validate all pages and generated tools before replacing one alias's routes. Failure preserves prior names only as inactive entries so stale callers receive a deterministic unavailable result without dispatch; successful explicit discovery replaces them. Separate tombstone and transition stores were rejected because the route's alias plus current client availability carries the needed authority.

### One minimal catalog/status API

`ToolCatalog` returns usable tools and current statuses. Status exposes only validated alias, availability, fixed summary, and an optional already-safe human diagnostic. `specs()` remains a wrapper. Recovery-transition queues and public transport/failure-stage types were rejected because Phase 0 needs current visibility, not a health history.

### Private independent stdio worker

One named OS thread and current-thread Tokio runtime per stdio session owns the platform process value and all pipes. Commands and one-shot replies serialize protocol operations. Reply loss before a write cancels safely; after a write starts it poisons and cleans the session. This is smaller than extracting the shell registry and, unlike a caller-owned task, survives parent-runtime destruction.

### Existing scanner with bounded projected chunks

The stderr reader feeds the existing scanner before putting whole projected chunks into a bounded tail. Scanner reservation uses current destination length while its cumulative counter retains relative-growth checks. A new filter or generic streaming sink was rejected because the existing scanner already holds cross-chunk recognition state.

### Closed admission, existing authority checks

A single closed bit is set before shutdown and rechecked after alias admission and immediately before native spawn. A handle for admitted startup is registered before readiness, so successful shutdown confirms its cleanup even when the process finishes starting after closure. Clients close concurrently within one aggregate bound. No sticky host-fatal latch is added. Construction keeps its retained-root validation and the worker performs the required immediate pre-spawn revalidation; no additional checks are added where no pathname-based authority is acquired.

## Operational surface

The existing `kuru tools`, `kuru tool`, plain run, JSON run and TUI entrypoints
keep their command and stdout contracts. Direct commands may add fixed status
and a bounded human diagnostic on stderr; runtime output uses the existing event
array and generic activity renderer. This change adds no bind address, container,
runner, secret, binary-version, architecture, or connection-limit setting. MCP
keeps the existing 64-server configuration bound and protocol I/O limits.

## Integration contract

The configured alias remains routing authority and the generated UUIDv5 tool
name remains the provider-facing identifier. Stdio fixtures continue to speak
newline-delimited JSON-RPC over owned stdin/stdout and now exercise independent
stderr; HTTP fixtures retain Streamable HTTP request/session behavior. No MCP
schema, ID type, endpoint route, SDK, mount, authentication, or configuration
shape changes. Catalog output adds connector-owned current status beside the
existing tool vector; the compatibility `specs()` projection remains available.

## Risks / Trade-offs

- [A status can become stale after its alias lock is released] → Define it as the result of that explicit deterministic sweep; every call rechecks current availability after admission.
- [A bounded tail may omit early stderr] → Retain byte count and truncation metadata, keep the most recent whole projected chunks, and never stop draining at the cap.
- [Cleanup cannot always be confirmed within the caller bound] → Return fixed unconfirmed context while the worker keeps the native owner and continues confirmation.
- [A concurrent shutdown can race admission] → Check the closed bit before and after acquiring client state and immediately before native spawn; register the pending transport before awaiting readiness so successful shutdown observes and cleans every admitted owner.
