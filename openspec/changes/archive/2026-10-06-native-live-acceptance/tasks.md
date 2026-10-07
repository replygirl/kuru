# Tasks

## 1. Opt-in native acceptance

- [x] 1.1 Add ignored `apps/kuru-tui/tests/native_live_acceptance.rs` using the existing factory, isolated temporary memory and synthetic project; verify native model metadata is retained and wrong model, unsupported effort, excessive requests, input and streamed output are rejected before further dispatch.
- [x] 1.2 Exercise two existing actors through bounded real turns and the public live renderer, private saved-summary identity and fresh-context exclusion, normal public replay and existing rewrite/annotation hooks; verify public and private projections separately and preserve terminal and memory cleanup.
- [x] 1.3 Run only the selected owning-app fixture once after source review, record actual usage, cache presence, reasoning and tool observations with exact terminal result, and distinguish live evidence from existing serializer proof and unobserved behavior.

Observed: owning task 56022 exited 0; the single selected ignored fixture passed in 18.45 seconds, and every other case was filtered. Its PTY child exited 0 with restored terminal state and ordinary public replay. Four real native `gpt-5.6-luna` completions used advertised `low` effort; the empty advertised capability map remained intact. Exact optional usage observations were:

| Completion | Input | Output | Cached input | Reasoning output |
| --- | --- | --- | --- | --- |
| 0 | 4576 | 104 | 0 | 88 |
| 1 | 5174 | 53 | 0 | 37 |
| 2 | 4628 | 26 | 0 | 0 |
| 3 | 5242 | 56 | 0 | 26 |

All usage values were present, including observed cached zero. No cache savings were observed. Both wrapper estimates and native measured estimates totaled 21916; actual reported input totaled 19620. The local payload bound observed 397 bytes, charging each request's larger stream or reconciled terminal payload without duplicating the two projections. This is not a provider output-token cap. The fixture permits six delegated streams, 16000 estimated input tokens per request, 64000 cumulatively, 16384 observed payload bytes and a 180-second child deadline. Rejection guards were source-reviewed; no extra paid rejection cases were dispatched.

There were 31 bounded facing-preview frames, two rendered settled frames after the backend and View preview cleared, a real pre-turn rewrite and two post-turn annotations. The neutral shared instruction prefix remained identical across the two actors and phases. Cognitive and facing tool inventories differed as designed; their whole inventories are not a shared final-wire prefix. Existing serializer fixtures provide separate serialization evidence; this wrapper does not prove final wire bytes.

No reasoning summary deltas, settled private summary sidecars, persisted private summary rows, thinking label, tool deltas or file-read calls were emitted. The conditional private retention/identity/exclusion, summary-facing display and tool-hook assertions are implemented but their positive live branches remain unobserved. Positive internal reasoning usage does not establish emitted reasoning summaries. No additional paid retry, canary, login or default-memory open occurred. Source review cleared both isolation and final failure-path scalar retention.

## 2. Verification and completion

- [x] 2.1 Run affected format, lint, type and managed checks, confirm ordinary CI remains offline through the ignored marker, and record any unrun checks and reasons.
- [x] 2.2 Complete this record with observed limits and evidence and verify strict validation before the required actual archive and normal branch commit for the existing PR delivery workflow.

Observed: final owning app host lint, all-target/all-feature typecheck and Rust format task 46429 exited 0; earlier affected static graph 52457 also exited 0 and included managed drift verification. Owning preparation/no-run task 15846 exited 0 before the reviewed corrections; task 56022 subsequently compiled and executed the final frozen source. Initial typecheck 29618 failed a private-module import and 18331 failed a mutex Send lifetime; both were corrected before inference, and 22444 compiled successfully. Ordinary CI remains offline because the fixture is ignored. No broad behavioral/coverage suite, native Windows live evaluation or paid retry was run for this Unix-only evaluation fixture; normal PR CI remains the delivery gate.
