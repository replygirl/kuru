## 1. Final selected request fits without claiming exact tokens [critical]

- [x] 1.1 @integration (agent) drive local HTTP fixtures through native Responses and Codex routes with multilingual, code, tool-heavy and encrypted-continuation requests at small budgets -> the package's Responses/custom-wire and native subscription fixtures passed with small-window refusal before HTTP, exact-boundary dispatch, both encrypted output hops and receipts intact; the final-body 2 MiB transport check remained independent. Runtime whole-row optional-history and mandatory-receipt tests passed. Real native subscription multilingual/code/tool and one-hop continuation requests also completed; no direct API-key live inference was run.
- [x] 1.2 @unit (agent) compare pinned `o200k_base` text vectors and route/model eligibility with unknown, custom-base and opaque cases -> official Cookbook six/seven-token text examples and multilingual/code/JSON-shaped cases passed; route-specific GPT-5.6 mapping, unmapped Astra and custom-base historical bytes/2 plus reserve, and mixed native output labels passed. Unknown models remained selectable.
- [x] 1.3 @integration (agent) capture outgoing fixture requests and measured estimates while transport bytes are independently bounded -> captured custom Responses JSON matched measured final bytes, stable tool order and labelled fallback; subscription one- and two-hop fixtures proved the exact mixed estimate at the boundary and refusal one token lower without changing sent output items. No inference count endpoint was called.

## 2. Shared prompt material and actor isolation [critical]

- [x] 2.1 @integration (agent) capture two actors and repeated turns under one project with fake provider responses -> `different_actors_share_leading_instructions_before_private_identity` passed with reviewed synthetic project rules: leading bytes and tool projections matched, identities followed them, and the existing private-memory isolation test passed in the runtime package suite.
- [x] 2.2 @unit (agent) vary phase, topology and public transcript independently -> the same fixture compared deliberate/speak phases and later public turns for the first actor, then changed only the in-memory roster name and made a test-only direct ask; the common prefix stayed byte-identical while the actor/roster suffix changed. Focused test passed after this extension.

## 3. Calibration and cache claims [critical]

- [x] 3.1 @eval (agent) compare multilingual, code and tool-heavy sizing against real provider-reported usage or an authorized offline official input-token count for supported models -> the table and held-out results below record genuine reported input totals, per-class signed/absolute errors, model, native subscription route, date and sample counts. The initial class pass informed the positive allowance; the final mixed estimator was checked in a separate held-out real continuation. API-key, long-context and cache-hit accuracy remain unmeasured.
- [x] 3.2 @integration (agent) deliver fake provider usage with absent, zero and positive cached tokens through the actual HTTP/ledger path -> connector HTTP and runtime accounting tests passed: `None`, `Some(0)` and `Some(8)` remain distinct, while the real subscription runs reported zero cached input on all observed requests. Matching prefix alone is not recorded as a hit, and fixture usage is not calibration evidence.
- [x] 3.3 @manual (agent) inspect `/cost` and context status for API and subscription fixture routes -> real PTY known-price and unknown-price `/cost` tests and visual context-status tests passed at practical widths; core catalog test confirmed the subscription basis remains API-equivalent, not a bill. Source/docs inspection confirmed incomplete cache-write terms and labelled sizing. A live subscription TUI display and direct API-key bill were not observed; the live route was headless and its reported usage was inspected separately.

## 4. Dependency, documentation and repository gates

- [x] 4.1 @regression (agent) verify pinned tokenizer asset/version and license with package ownership, build/docs checks, strict Cospec validation and applicable package tests -> `tiktoken-rs = =0.12.0` and its embedded asset SHA-256 matched the upstream published value, with MIT notices in the connector package. Offline binary build, core/connector/runtime/TUI package suites, root typecheck, affected-crate lint, root format check, docs build/link/content check and strict Cospec validation passed on macOS. Native Windows execution and normal pre-push coverage remain delivery/CI gates, not local evidence.

## Observed evidence (2026-09-22)

The first calibration pass used Kuru's already-authenticated native ChatGPT route
from four separate temporary projects under `/private/tmp/kuru-p07-calibration.*`,
with fresh sessions, `max_rounds = 1`, `max_tool_calls = 1`, `max_parallel = 1`,
dreaming disabled, and artificial prompts. Kuru's bounded `--debug` diagnostic
ring paired a final-body estimate with the terminal provider usage within each
actor request span. No credential file, prompt body, tool arguments or response
text was copied into this ledger. Every run completed, with four reported
invocations and `cached_input_tokens = 0` for each invocation. The first
default-data-directory attempt failed private-directory validation; the
approved preview data directory's existing Dolt cache then failed validation,
so only this temporary project's configured engine cache was provisioned.

| Class | Model | Requests | Reported input | Initial local estimate | Signed/absolute error | Earlier bytes/2 estimate | Local BPE before allowance |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Multilingual translation | gpt-5.6-luna | 4 | 3,199 | 5,298 | +2,099 / 2,099 (+66%) | 8,935 (+179%) | not logged in first run |
| Rust code | gpt-5.6-luna | 4 | 3,391 | 5,490 | +2,099 / 2,099 (+62%) | 9,132 (+169%) | 4,210 (+24%) |
| Mock tool schema | gpt-5.6-luna | 4 | 3,477 | 5,770 | +2,293 / 2,293 (+66%) | 9,761 (+181%) | 4,490 (+29%) |
| Mixed held-out prompt | gpt-5.6-terra | 4 | 3,541 | 5,671 | +2,130 / 2,130 (+60%) | 9,538 (+169%) | 4,391 (+24%) |

These are short subscription-route requests, not a broad accuracy guarantee.
They show that the original large structural allowance overcounted on top of
serialized BPE overcount. Although their common prefix matched, the native
provider reported zero cached input for all 16 invocations; there is no
observed savings claim. Synthetic HTTP usage fixtures validate plumbing only
and are never counted as calibration.

After reducing the allowance to a still-positive 64 tokens plus 8 per input
item and tool, a fresh held-out `gpt-5.6-terra` subscription session used an
artificial Japanese/Rust/JSON file. The real `file_read` call succeeded and
the subsequent native continuation remained intact. Its first four requests
used the sourced `o200k_base` estimate: 4,569 estimated versus 3,277 reported
input tokens, a +1,292 signed and absolute error (+39%). Each request
overcounted; none undercounted. The original whole-body one-token-per-byte
fallback made the fifth, opaque continuation 5,405 estimated versus 795
reported (+4,610, +580%); this exposed a material usable-context regression.
For the whole five-request session, that first revision estimated 9,974
versus 4,072 reported (+5,902, +145%), while historical bytes/2 would have
been 11,741 (+7,669, +188%). It is calibration of a superseded revision,
not evidence that its continuation fallback should ship.

The final mixed estimate was then checked in a **new** isolated project with
the same artificial file and prompt. Kuru again made a real `file_read` call;
the pending native output and tool receipt were present in the next request.
The first four visible requests estimated 4,575 versus 3,284 reported
(+1,291 signed/absolute, +39%); the native continuation estimated 1,811
versus 799 reported (+1,012, +127%). Its final body was 5,305 bytes, so the
prior bytes/2 heuristic alone would have estimated 2,653 (+1,854, +232%)
for that request. Across all five requests, the final estimator totalled
6,386 versus 4,083 reported (+2,303, +56%); the prior bytes/2 total was
11,763 (+7,680, +188%). All five requests overcounted; none undercounted.
The unknown/custom route still uses historical bytes/2 plus the small
structural reserve, and no unknown-route live accuracy inference is drawn
from this mapped-route sample. Both held-out sessions reported zero cached
input in every request. A restricted-network attempt failed at catalog
transport before inference; the approved live route succeeded. API-key route,
long contexts and actual cache hits remain unmeasured, so no universal
accuracy or savings claim follows from these samples.

Local verification: core, connector, runtime and TUI package suites passed
(211 connector and 142 runtime tests, plus all TUI targets); root typecheck,
affected-crate lint, root format check, docs build/link/content check and
offline TUI binary build passed. After the narrow two-hop fix and test-only
topology extension, the focused connector two-hop, mapped-route and eligibility
tests and the focused runtime prefix test passed again. The test-only runtime
fixture required the owning prepared Dolt snapshot; its first unprepared attempt
failed, then the prepared run passed. A TUI rebuild briefly exhausted disk;
removing only this worktree's generated incremental artifacts allowed the
subsequent build to pass. Strict Cospec validation and apply gate passed after
the estimator policy revision. Normal pre-push coverage and native Windows
execution are pending the delivery and CI gates.

The pinned tokenizer vocabulary is constructed once by
`o200k_base_singleton()` and shared across requests. Mapped estimates tokenize
the already serialized final JSON once. Mixed estimates clone the bounded
final body, remove only saved opaque output ranges in the local sizing copy,
serialize that visible copy once and tokenize once. This adds per-request CPU
and allocation relative to bytes/2, bounded by the 2 MiB request-body check;
cold and warm latency were not separately timed.
