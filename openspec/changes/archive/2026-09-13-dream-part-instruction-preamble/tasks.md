## 1. Canonical core construction

- [x] 1.1 Add one core-owned equal-peer instruction constructor/validator that enforces the incoming pre-normalization 8,192-byte bound, preserves the current builtin IDs and exact instruction bytes, removes only repeated exact leading canonical prefixes, preserves the tail bytes, and rejects blank/prefix-only remainders.
- [x] 1.2 Retain runtime `Add` validation for name, role, capacity, and topology, and invoke the core constructor only for a newly accepted candidate addition; do not duplicate its byte-limit check or alter load or historical-part paths.
- [x] 1.3 Add focused core regressions for exact builtin preservation, repeated-prefix normalization, prefix-like/non-leading prose, UTF-8 boundary handling, and bounded-domain idempotence; verify the raw legacy behavior fails the missing-prefix regression before the correction and passes afterward.

## 2. Candidate persistence regression

- [x] 2.1 Extend the real dream candidate/promotion fixture with mixed valid and invalid additions; verify individual rejection, exact promoted and reloaded instruction bytes, stopped reload, and reversal while retaining its existing topology/history assertions.
- [x] 2.2 Run the focused core and runtime package-owned mise checks plus scoped format/lint/typecheck; record the native Windows real-memory fixture as unrun locally rather than fabricating that evidence.
