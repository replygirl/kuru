## Context

The cache stage is owned by a temporary directory whose destructor discards removal errors. After a verified move, candidate and probe handles are dropped only by the activation structure's destructor, and a stage cleanup failure cannot affect the returned success.

## Goals / Non-Goals

**Goals:** Return a checked result for successful publication cleanup while retaining the cache lock and closing candidate/probe handles first.

**Non-Goals:** No change to publication retry, failed-stage retention, cancellation, or Dolt process lifecycle.

## Decisions

Consume the activation structure after a successful checked move. Drop source and probe handles, call the existing temporary-directory owner's one-shot checked close, then release the cache lock. A close failure reports that engine publication succeeded and cleanup failed; it does not misclassify the move as failed.

## Risks / Trade-offs

The caller can receive an error even though the engine is published. The contextual error makes that state explicit; the next checked open still verifies the published identity and strict cache inventory.

## Operational surface

The change affects local native Dolt cache staging on supported hosts, especially Windows file-handle cleanup. It changes no bind address, secret, binary version, architecture, or CI topology.
