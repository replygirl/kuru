## Context

The merged-main Ubuntu ARM release build passed, while P29's exact head fails when Rust computes an existing `kuru` library async-block layout at query depth 130. The failing block is byte-identical to main; changing its runtime flow would add risk unrelated to the observed compiler bound.

## Goals / Non-Goals

**Goals:** Permit the observed finite layout query and prove the actual Ubuntu ARM release build succeeds.

**Non-Goals:** Change the TUI event loop, box its futures, or raise limits in other crates.

## Decisions

Set `#![recursion_limit = "256"]` only in `apps/kuru-tui/src/lib.rs`, the crate named by the compiler error. If the same query still overflows, investigate actual type growth rather than repeatedly increasing the limit.

## Risks / Trade-offs

A higher compiler query limit can consume more build resources. The finite 256 limit is scoped to one library; the exact Ubuntu ARM release build and full CI must establish that it compiles within the runner budget.

## Operational surface

This changes only the Rust compiler setting for the `kuru` library during native release builds, especially Ubuntu 24.04 ARM (`aarch64-unknown-linux-gnu`). It changes no bind address, runtime connection limit, container/runner topology, required secret, or deployed binary behavior.
