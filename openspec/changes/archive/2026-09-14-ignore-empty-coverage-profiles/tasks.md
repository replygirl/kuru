## 1. Profile selection

- [x] 1.1 Validate and bound all raw-profile candidates before filtering zero-byte regular files, require at least one retained nonempty profile, and preserve every existing receipt and aggregate gate
- [x] 1.2 Add focused regression cases for mixed empty/nonempty success, empty-only failure, pre-filter candidate bounds, nonregular rejection, and malformed nonempty reporter failure

## 2. Verification

- [x] 2.1 Pass focused coverage helper tests, delivery all-target/all-feature typecheck, Clippy with warnings denied, formatting, strict Cospec validation, and diff checks
- [x] 2.2 Record the pre-fix hosted connector receipt failure and preserve the post-push shard/aggregate checks as required merge gates
