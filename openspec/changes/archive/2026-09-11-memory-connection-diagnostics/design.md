## Context

SQLx 0.9 acquisition retries can discard `after_connect` errors and end with PoolTimedOut. The existing two-second budget includes authentication and directory/project verification. CI establishes lost context, not a particular timing or transport cause.

## Decisions

Track static phases and a bounded, sanitized callback error per initial pool acquisition. Preserve the original SQLx error and explain when transport/authentication never reached the callback. Add call-site staging, active, and post-ready context. Append observed cleanup failure without weakening owned lifetime behavior.

## Risks / Trade-offs

SQL errors can contain sensitive values: retain only safe classifications or explicitly authored identity-check messages, never credentials, arbitrary SQL payloads or connection options. Keep diagnostics bounded and preserve the existing two-second acquisition and startup deadlines. This improves diagnosis and does not claim to fix an unestablished Windows timing cause.
