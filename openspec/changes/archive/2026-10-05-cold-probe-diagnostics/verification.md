# Verification

## 1. Genuine cold-copy progress [critical]

- [x] 1.1 @regression (agent) prepare a checked private executable copy with an existing per-open counter -> pre-fix owning test exited101, count0 expected2; corrected test passed, final shared cross-host fixture passed in owning20/20 probe selection, exact2 before observer/3 after hash and identical bytes
- [x] 1.2 @integration (agent) hold the checked-copy observer while sampling the counter -> real checked-copy barrier kept counter unchanged while blocked; destination hashing alone added final unit after release, fixture passed
- [x] 1.3 @integration (agent) run existing corrupt-source/corrupt-copy fixtures -> both checksum refusals passed in corrected2/2 and final20/20 owning selections

## 2. Diagnostic boundaries [critical]

- [x] 2.1 @integration (agent) drain a partial/EOF/oversized pipe under the existing output bound -> pending drain reported4 bytes/noEOF, actual close4/EOF, oversized refusal4097/noEOF; owning20/20 passed
- [x] 2.2 @integration (agent) exercise observed probe refusal and diagnostic gate -> isolated real child gate1 emitted creation/refusal/cleanup-reaped, exact20-byte pipes/EOF; gate0/unexpected inert, no body/private path/stdout diagnostics, LLVM_PROFILE_FILE preserved; owning20/20 passed
- [~] 2.3 @runtime (agent) run native Windows cold-memory fixture in CI -> defer: native Windows and hosted coverage require the resulting PR run; original Windows stall remains unproved

## 3. Repository checks

- [x] 3.1 @integration (agent) run owning host and Windows lint/type checks, formatting and documentation checks -> memory host and Windows-target lint checked all targets/features with warnings denied, root format and docs build/content checks exit0; Windows-specific unused wrapper correction passed rerun
- [x] 3.2 @manual (agent) independent review of source and local acceptance -> reviewer accepted final production/fixture/docs changes and reported20/20 focused plus static/docs outcomes; final strict validation exit0, actual archive --skip-specs exit0 and complete moved archive confirmed before final commit
- [x] 3.3 @e2e (agent) run the original exact cold CLI JSON fixture locally -> owning kuru-tui:test filtered to cli_memory_progress_is_bounded_and_keeps_json_on_stdout exited0 with fd4096 and isolated verified cache on macOS; real bundled cold preparation preserves JSON/progress assertions, no Windows claim
