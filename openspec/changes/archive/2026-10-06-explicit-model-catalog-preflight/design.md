# Design

## Context

Models dispatch currently follows shared memory activation. A global legacy
sentinel therefore requests migration before an explicitly selected catalog can
run. Saved preferences contain framework and per-provider model/effort choices;
the provider itself comes from configuration or invocation overrides.

## Decisions

Use one predicate for Models with both explicit provider and model flags. After
the existing provider-route trust preflight and workspace revalidation, finalize
the immutable snapshot with default preferences and reuse provider construction
and catalog serialization. Keep all other invocations on their current path.

## Risks / Trade-offs

Skipping preferences also skips a remembered framework or effort. Those values
do not drive catalog retrieval. Default invocations retain their existing saved
selection loading and validation behavior.

## Operational surface

This CLI inspection uses the existing executable and native credential routes.
It introduces no listener, container, required secret, connection limit, binary
version or architecture change. Tests use numeric loopback HTTP with synthetic
credentials and must assert that no inference or memory runtime starts.
