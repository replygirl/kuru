# Tasks

## 1. Characterize HTTP reuse

- [x] 1.1 Add a bounded loopback fixture that counts accepted sockets and completes two requests over a reused provider instance; assert isolated actor request bodies and owned server cleanup
- [x] 1.2 Measure the fresh-provider-per-request baseline and the reused-provider result with the same fixture, then record the observed counts in `proposal.md`
- [x] 1.3 Run the focused reuse fixture and existing provider/SSE equivalence selections and record actual outcomes

## 2. Integrate service evidence and document scope

- [x] 2.1 Normally integrate the clean N4 service-lifecycle commit once available; preserve its accepted overlap, generation, and immediate zero-client retirement behavior
- [x] 2.2 Update the relevant development documentation to distinguish HTTP socket reuse from per-project memory-service attachment and record the bounded evidence

## 3. Validate and archive

- [x] 3.1 Run applicable connector/docs checks, Cospec strict validation, and the actual apply gate; review the final scoped diff
- [x] 3.2 Review the final fixture, measured result, documentation, and N4 evidence against the bounded O3 scope before archiving
