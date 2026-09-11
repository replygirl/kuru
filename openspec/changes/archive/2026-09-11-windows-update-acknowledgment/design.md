## Context

The parent uses the same ten-second receive limit for helper configuration and publication acknowledgment. The latter includes full image verification, two durable executable copies and receipt publication. Windows CI observed correct publication at 11.689 seconds and failed acknowledgment at 11.707 seconds because the parent had closed its pipe. No corruption, missing acknowledgment validation or helper identity error was observed.

## Decisions

Allow up to 120 seconds for the first byte of verified publication acknowledgment, then retain the ten-second bound for the complete frame without extending the original 120-second deadline. Startup connection, configuration and outgoing frames keep their existing ten-second limits. The first-byte wait is finite and applies only to the authenticated helper's publication phase; malformed length, truncation, JSON and exact candidate validation remain unchanged.

Use the existing compiled helper observer to hold real publication beyond ten seconds, release it and inspect the actual acknowledged bytes and retained original process through the real private IPC. Exercise missing acknowledgment and incomplete frames through the same decoder and compiled native pipe fixtures with short injected test budgets. Preserve ordinary receipt, copy, cleanup and uncertain-publication behavior; this fix does not optimize or change filesystem publication.

## Operational surface

This affects the existing Windows x86-64 updater and its retained trusted helper on the native host. Private same-user named pipes still authenticate the peer; no listener, dependency, credential or setting is added. Only publication acknowledgment receives the separate bounded wait. Existing native CI and packaged offline acceptance exercise the ordinary executable.

## Risks / Trade-offs

A stalled trusted helper can now keep its parent waiting longer, bounded by 120 seconds overall and at most ten seconds after frame arrival. Existing failure handling retains recovery receipts and does not terminate a helper in its publication gap. Native Windows execution is required; macOS builds cannot establish this correction.
