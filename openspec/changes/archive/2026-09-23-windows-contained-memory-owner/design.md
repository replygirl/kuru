## Context

`NativeSpawnSpec::IndependentService` always adds `CREATE_BREAKAWAY_FROM_JOB`. That is correct when no Job contains the starter or every applicable Job permits breakaway. Under a legitimate nonpermitting outer Job, Windows returns `ERROR_ACCESS_DENIED` before creating the child. P27 application and packaged-offline fixtures therefore fail before the memory service can publish an endpoint.

Windows nested-Job semantics distinguish starter independence from host containment. A child created without breakaway inherits the current Job, but the starter process exiting does not itself close an outer Job whose handle is retained by the host. Closing the outer kill-on-close Job terminates the complete contained tree.

## Goals / Non-Goals

**Goals:** Preserve permitted breakaway, allow service startup under legitimate nonpermitting host containment, retain private IPC and authoritative owner/Dolt cleanup, and prove normal recovery after the outer Job terminates the contained tree.

**Non-Goals:** Escape every host Job, identify which ancestor denied breakaway, weaken owner election, add global `BREAKAWAY_OK`, retry arbitrary launch failures, or change Unix/process/domain protocols.

## Decisions

Keep the existing `IndependentService` create attempt first. If it fails with exactly `ERROR_ACCESS_DENIED`, call `IsProcessInJob(GetCurrentProcess(), NULL, ...)`. Retry only when that query succeeds and confirms membership. Rebuild the command-line buffer and zeroed `PROCESS_INFORMATION`, preserve the same executable, environment, current directory, startup attributes, explicit inherited-handle list, console flags and retained handles, and remove only `CREATE_BREAKAWAY_FROM_JOB`. A failed membership query returns its own error; a confirmed nonmember returns the original access-denied error.

The retry creates no Kuru-owned Job. The returned `NativeChild` retains the exact process handle for status, termination, and reap observation as before. The service owns its Dolt supervisor, endpoint and owner/lifecycle locks as before. The fallback changes only whether the service inherits a pre-existing outer Job.

Do not broaden the coverage runner's breakaway allowlist. That would hide direct launches under nonpermitting Jobs and would change runner topology rather than make the platform API honor its declared lifetime contract.

## Operational surface

There is no new setting, secret, address, binary, architecture, or user action. On supported Windows hosts, a permitting Job still allows the service to escape the starter's Job. A nonpermitting outer host or runner contains the owner and Dolt tree; Kuru does not claim survival after that host containment closes.

## Integration contract

`kuru-platform` alone classifies the exact failed native create and reconstructs mutable launch state. `kuru-memory` continues selecting `IndependentService` and authenticating the same private endpoint. `OwnedJob`, `TrustedSupervisor`, fixture handle allowlists, election locks, receipt reconciliation, and all non-Windows call sites remain unchanged.

## Risks / Trade-offs

- **A live owner could be replaced after containment failure.** → The fallback changes only child creation; existing start, owner and lifecycle locks still gate election and recovery.
- **Access denied could mean something other than Job breakaway.** → Retry requires both the exact first error and confirmed current Job membership; every other create or membership-query error remains fatal.
- **CreateProcess mutates caller buffers.** → Reconstruct the command line and output structure for the one retry while retaining all immutable inputs, attributes and handles.
- **Outer Job closure kills the service.** → This is the host's legitimate lifetime boundary; native acceptance proves complete-tree termination and ordinary authoritative recovery without stale lease damage.
