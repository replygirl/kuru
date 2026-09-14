## 1. LLVM-compatible raw-profile selection [critical]

- [x] 1.1 @regression (agent) produce one zero-byte regular `.profraw` beside a valid nonempty profile through the receipt copy/hash seam -> run 34849384516 shows the prior helper rejected the connector shard's empty profile after all tests passed; the corrected focused case bounded both candidates, omitted only the empty file, copied and hashed the 13-byte nonempty profile, and passed 1/1
- [x] 1.2 @equivalence (agent) exercise empty-only, excess-candidate, oversized, and nonregular profile inputs -> focused cases rejected empty-only, 4,097 zero candidates, a sparse oversized file, a directory, and a Unix symlink before receipt, while retaining malformed nonempty data; pinned llvm-profdata 1.98.1 accepted empty-only and empty-plus-valid input but rejected malformed nonempty input as truncated with no mergeable profile

## 2. Preserved coverage gate

- [x] 2.1 @integration (agent) run focused coverage helper tests plus delivery typecheck, Clippy, formatting, and diff checks -> the mixed-input and Unix symlink cases each passed 1/1; delivery all-target/all-feature typecheck, Clippy with warnings denied, formatting, strict Cospec validation, and diff checks passed, with no receipt, aggregate, ledger, or threshold source change
- [~] 2.2 @runtime (agent) run the corrected connector shard and aggregate on required GitHub CI -> defer: required checks run only after the completed fix record is archived and pushed and remain the merge gate
