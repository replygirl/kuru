## Context

The trust store must retain a pinned private directory while it verifies the
workspace approval record. On Windows, a file opened through that directory
inherits pinned name retention and therefore omits `FILE_SHARE_DELETE`.
`publish_file` deliberately holds the written pending record until its
identity-checked `MoveFileExW` call, so the pending handle prevents that call
from moving its own name.

## Goals / Non-Goals

**Goals:**

- Publish and remove approval records through an identity-checked movable file
  operation handle while the original pinned storage authority remains live.
- Retain private ACL sealing, checked replacement, reconciliation, and
  fail-closed behavior.

**Non-Goals:**

- Change platform `Pinned` semantics, workspace-root retention, publication
  policy, or approval-record format.
- Add a generic platform abstraction or make Windows publication behavior
  available without native verification.

## Decisions

- Open the already-pinned owner-only workspace-storage path again with
  `NameRetention::Movable` only at the trust record publication/removal
  boundary. Revalidate the pinned authority and require matching full directory
  identity before using the movable capability.
- Create, read, remove, reconcile, and publish the pending/approval file using
  that movable capability. Keep the pinned directory and lock handle alive for
  the complete operation, and revalidate the pin before success is reported.
- Preserve `seal_private`: it sets the candidate's DACL after opening and does
  not change the creation-time sharing mode that causes the native failure.

## Risks / Trade-offs

- [A reopened path could name a replacement directory] → validate owner-only
  privacy and exact full identity against the retained pinned storage directory
  before every movable operation.
- [A movable record handle permits a concurrent name change] → retain the
  pinned storage and lock authority, use existing `verify`, checked
  publication/removal, reconciliation, and final revalidation.
- [macOS/Linux tests cannot establish Windows sharing behavior] → add
  filesystem regression coverage and leave required native Windows execution
  as an explicit verification row.

## Operational surface

This changes only local trust-record filesystem publication in the existing
`kuru trust approve` and `kuru trust revoke` commands. It adds no listener,
bind address, container or runner topology, credential, connection limit,
binary version, or architecture requirement; supported Windows CI remains the
required native execution environment.
