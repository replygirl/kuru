## Context

The managed provisioning path acquires `.install.lock` before it distinguishes a warm cache from an absent destination. Full hashing and the version probe therefore serialize every process even though only extraction and publication require exclusive authority. The existing 0755 diagnostic similarly uses legacy SQLite presence as a proxy for whether a rejected Unix directory can be safely identified as an owner-controlled mode problem.

## Goals / Non-Goals

**Goals:** Verify warm caches concurrently without weakening byte or identity checks; keep cold publication serialized and recoverable; and diagnose only current-user-owned real Unix directories whose access bits explain the refusal.

**Non-Goals:** Replace full digests with metadata, remove the version probe, add a cache daemon, repair permissions automatically, or relax link and foreign-owner rejection.

## Decisions

The warm path opens and retains the checked cache directory and its two payload handles, verifies size, privacy, executable mode and complete digest from those handles, and revalidates the directory and both names immediately before the path-based exact-version probe. Those handles remain alive until the owned probe is reaped.

The cold path observes absence, acquires the existing installation lock, then checks the destination again. A destination published by the winner is verified after releasing the contender's lock; a still-absent destination continues through unchanged owned extraction, probe, activation and Windows uncertainty recovery.

The Unix remedy inspects the rejected path itself without following links. It names mode 0700 only for a directory whose UID matches the effective user and whose group/other mode bits are nonzero. Every other refusal retains the underlying generic privacy error.

## Risks / Trade-offs

Concurrent warm opens read the same large payload and can contend for I/O bandwidth → retain the full integrity contract and document measured latency rather than adding a weaker cache marker.

A pathname probe necessarily follows the name after validation → revalidate the retained directory and exact payload identities immediately before launch and retain all handles until the process is reaped.

A cold contender can observe publication between its first check and lock acquisition → recheck under the lock and switch to ordinary warm verification without extracting or overwriting.

## Operational surface

The change adds no listener, container, secret, connection limit or deployment topology. It applies to the existing five target-specific embedded Dolt archives and their pinned native executable/license payloads; user-visible output is limited to the existing CLI error path for a rejected local data directory.
