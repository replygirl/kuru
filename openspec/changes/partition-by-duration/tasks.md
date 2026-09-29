# Tasks

## 1. Timing table and placement

- [x] 1.1 Add the strict table parser, canonical digest and greedy placement with the SHA-256 fallback, and verify with the unit tests of verification 1.1, 1.2 and 1.4
- [x] 1.2 Bind the table digest into the partition scheme and route dispatch, ledger validation and the merge through the placement, and verify the existing merge, receipt and dispatch tests pass (verification 1.3)

## 2. Timing records and refresh

- [x] 2.1 Record per-test completion offsets in the runner ledger and verify with the live exact-selection test (verification 2.2)
- [x] 2.2 Add `coverage timings` and the `coverage:timings` task, and verify with its unit test (verification 2.1)
- [x] 2.3 Generate the first table as the rounded mean of main runs 36614320897 and 36592401648, rows from 36614320897, and verify the attribution replays each run's assignment (verification 2.3)

## 3. Reports

- [x] 3.1 Add the merge's prediction and staleness report with the job summary and warning, and `coverage balance`, and verify with verification 3.1 and 4.1

## 4. Documentation and verification

- [x] 4.1 Update `docs/development.md` and verify `docs:check`
- [x] 4.2 Run the repository gates of verification 6.1 and record observed evidence

## 5. Review follow-up

- [x] 5.1 Refuse partial or mixed evidence in `coverage timings` (one source and tree, one partition count and every partition index per measured label), and verify with the extended evidence test (verification 2.1)
- [x] 5.2 Bound each recorded completion by the invocation's recorded run time plus one second, and verify with the ledger tamper test (verification 1.3)
- [x] 5.3 Label the replayed "after" figures as a prediction that assumes additive attribution, naming its known limit (verification 3.1)
- [ ] 5.4 Run the branch on every hosted OS label and record the measured slowest partition jobs and merge results (verification 5.1)
