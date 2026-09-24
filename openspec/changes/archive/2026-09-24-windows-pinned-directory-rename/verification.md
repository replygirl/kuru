## 1. Pinned directory name stability [critical]

- [~] 1.1 @regression (agent) open a real Windows directory through `Directory::open(..., Pinned)`, attempt DELETE-open and pathname rename while held, then release it -> defer: diagnostic head `421c7ef5` proved the metadata-only pre-fix operations succeed and narrower traversal refuses; the corrected production-path fixture must run on final-head native Windows CI before merge.
- [~] 1.2 @integration (agent) run the existing prepared `file_list` parallel-read fixture on native Windows -> defer: job 107807657370 still failed on diagnostic-only head `421c7ef5` with a successful rename; corrected-head native connector CI must prove refusal and no replacement-data exposure before merge.
- [~] 1.3 @regression (agent) hold the checked-removal `pin_directory` on a real Windows directory and attempt to rename it -> defer: source reviewed and Windows cross-target checks passed, but the new direct pin and nested-removal fixtures must run on corrected-head native Windows CI before merge.

## 2. Scope and portability

- [x] 2.1 @integration (agent) run owning Windows platform typecheck/lint and relevant host runtime fixtures -> Windows target check/Clippy passed; host platform package tests passed after sandbox-only IPC refusal rerun, and the three focused real-Dolt runtime fixtures passed 1/1 each.
- [~] 2.2 @integration (agent) require final-head supported-platform CI before merge -> defer: this exact head does not exist until the reviewed source and archive are published; leave the delivery gate open until hosted CI succeeds.
