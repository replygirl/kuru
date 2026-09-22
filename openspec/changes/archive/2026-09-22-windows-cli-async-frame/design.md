## Context

The binary's Tokio entry currently awaits its helper and ordinary CLI branches inline. On native instrumented Windows builds, the main-thread stack has overflowed in ordinary CLI child processes on two distinct feature branches. The later diagnostic explicitly bound the CLI future before awaiting it and made even early commands fail before the first stage marker; it supports stack-layout sensitivity but does not prove a particular deep shell call was responsible. The current main source itself is green, so a Windows integration run on the formerly failing branches is needed to establish the correction there.

## Goals / Non-Goals

**Goals:** Put the entire unchanged dispatch future behind one heap-pinned boundary at the binary entry, preserving CLI/helper behavior and native test coverage.

**Non-Goals:** A new runtime task, larger thread stack, changed command route, or retained diagnostic marker.

## Decisions

Extract the existing async main body into `dispatch` and await `Box::pin(dispatch())` from the same `#[tokio::main]` entry. This keeps internal helper and ordinary CLI branches in one heap-resident future, so adding another internal branch does not enlarge the outer entry future. Boxing only the ordinary CLI arm was rejected because another internal arm can still dominate the outer async state. There is no spawn: the same task, cancellation, error propagation, and cleanup remain in sequence.

## Risks / Trade-offs

- One heap allocation per executable invocation and one indirection during polling are expected; no measurable provider, memory, or tool request overhead is claimed.
- Boxing does not guarantee that every transient poll-stack allocation disappears. Passing the existing failing native Windows cases on a dependent integrated head is the acceptance evidence; a passing macOS or current-main check alone is insufficient.

## Operational surface

This correction changes no bind address, service endpoint, secret, container, or runner policy. The ordinary `kuru` binary and its existing internal helper invocations remain the same executable on each supported target; the native Windows application matrix exercises the actual x86_64 Windows binary, while local macOS checks exercise the host binary only.
