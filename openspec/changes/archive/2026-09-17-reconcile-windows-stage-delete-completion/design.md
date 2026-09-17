## Context

The checked platform remover intentionally returns `PublicationPhase::Uncertain` after a native deletion may have changed the tree. Windows native CI showed a private stage in that state through a delete-pending name or an OS145 nonempty outer directory. The previous P5 cleanup retried only rejected raw OS32 and therefore could not reconcile the exact object after these typed outcomes.

## Goals / Non-Goals

**Goals:** Reconcile only the retained private-child and outer identities within the existing two-second cleanup bound; keep the first causal error for bounded exhaustion; and observe name disappearance before success.

**Non-Goals:** No platform API, public setting, generic deletion policy, publication retry, process supervision, or unbounded wait.

## Decisions

`RemovalError.phase == Uncertain` and rejected raw OS32 begin one shared bounded recovery window. A child must re-open as the same identity before a checked retry, be absent before advancing, or be delete-pending and observed without mutation. Changed identities and nonrecoverable rejected errors fail immediately. The outer directory is retried only for raw OS32 or OS145 while the child remains absent or pending; a successful raw removal still requires observed absence.

The pending state uses platform-typed native errors only. The fixture releases a nested retained directory after the first recoverable removal result, exercising reconciliation without a blind delay. The invalid outer-handle premise is removed.

## Risks / Trade-offs

A private path that remains delete-pending can occupy the full two-second cleanup budget. The cache lease remains owned and the first typed removal cause is returned on exhaustion, so the implementation neither releases authority mid-cleanup nor deletes a replacement.

## Operational surface

This affects only Windows native private-stage cleanup and its fixtures. It changes no bundled asset, secret, binary version, network route, or CI topology.
