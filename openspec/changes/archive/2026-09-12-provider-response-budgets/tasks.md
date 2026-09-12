## 1. Completion deadline

- [x] 1.1 Apply the private 600-second timeout to API-key Responses completion
  requests while retaining the 60-second catalog timeout and existing connect
  and idle bounds.
- [x] 1.2 Add a focused provider regression that observes the API-key completion
  request deadline without a real 600-second wait.

## 2. SSE retained and transport budgets

- [x] 2.1 Separate the retained completed-response limit from finite SSE wire,
  line, and event-payload limits, including final-envelope reconciliation.
- [x] 2.2 Add SSE regressions that fail before the repair: over-2-MiB discarded
  traffic with a small completion succeeds, a valid over-64-KiB single-line
  retained item succeeds, and retained, wire, and event excess fail boundedly.

## 3. Documentation and focused verification

- [x] 3.1 Describe the private completion and SSE budget distinction in the
  protocol documentation without adding configuration.
- [x] 3.2 Run focused connector tests through mise and record observed evidence;
  leave the coordinated combined coverage and full-repository checks pending.
