## Context

The connector shard passed every selected test but left one zero-byte raw
profile. The helper currently rejects any empty candidate while producing the
receipt. Pinned LLVM profile merging with failure mode `any` accepts empty input
as no contribution, but Kuru still needs at least one nonempty profile per shard
and must keep its stronger inventory, type, and size bounds.

## Goals / Non-Goals

**Goals:**

- Treat a validated zero-byte regular `.profraw` as no coverage contribution.
- Bound all candidates before filtering and retain at least one nonempty profile
  in every successful receipt.
- Preserve malformed nonempty input for rejection by the pinned reporter.

**Non-Goals:**

- Changing receipt schemas, aggregate identity, shard selection, execution
  ledgers, profile limits, coverage threshold, or workflow retries.

## Decisions

Enumerate and sort every `.profraw` candidate, then use symlink metadata to
require a regular file and enforce its per-file size limit. Apply the existing
candidate-count and checked cumulative-byte limits to that complete set before
filtering. Remove only entries whose validated metadata length is zero, and
require the retained set to contain at least one nonempty profile.

The producer copies and hashes only retained profiles, so zero-byte candidates
do not enter the receipt or uploaded evidence. Nonempty malformed bytes continue
through the receipt unchanged and remain subject to the existing pinned
`llvm-profdata`/reporter rejection. Aggregate receipt hashes, profile rechecks,
global limits, source/tool/attempt identity, and the 90% gate do not change.

## Operational surface

This affects only the existing Windows coverage helper running on GitHub's
Windows 2025 runner. It adds no listener, secret, binary, architecture, cache,
or workflow step; the pinned cargo-llvm-cov and LLVM tools remain authoritative.

## Risks / Trade-offs

An empty file no longer diagnoses which instrumented process produced it. The
receipt still proves at least one real profile, the full execution ledger proves
every selected binary's outcome, and all candidate bounds apply before filtering,
so accepting no-contribution entries cannot create an empty or unbounded shard.
