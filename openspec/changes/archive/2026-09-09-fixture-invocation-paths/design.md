## Context

The pre-push coverage run lost two peers, but the normal suite and preceding full
gate passed. The logs do not show fixture stderr or the actual Codex error.
Executable-path resolution is a hypothesis, not a demonstrated cause of those
failures. Absolute invocation identity is nonetheless the intended contract for
per-fixture plans and can be exercised independently of platform path resolution.

## Decisions

Locate fixture plans and transcripts from absolute argv[0], never by resolving
the shared executable inode. Preserve immutable hard links and Arc ownership.
Launch the shared artifact with an explicit fixture alias as argv[0] in a bounded
real-process regression. Verify exact response, transcript and completion marker.
Keep normal hard-link execution and concurrent isolation regressions as well.
Capture useful test-owned startup/error diagnostics and display actual assertion
errors. Investigate any reproduced failures; do not conceal them with retries,
long sleeps, suite serialization or weaker protocol/coverage checks.

## Risks / Trade-offs

Invocation identity alone may not explain the intermittent failure. Record that
limit and require the full gate, pre-push hook and hosted Linux/macOS checks.
Test paths are absolute and supplied by our harness; this change is not a
production executable-discovery policy.

## Operational surface

Native Rust fixture processes run directly on macOS and Linux, compiled by the
repository's pinned rustc. No new dependency, secret, port or production process
topology is introduced. Existing HTTP integration tests use ephemeral loopback.

