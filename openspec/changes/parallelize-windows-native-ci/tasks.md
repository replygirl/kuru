## 1. Coverage tooling

- [x] 1.1 Implement a delivery-owned helper that builds and validates the full
  Cargo artifact inventory, generates a task-private native runner, validates
  every full-inventory run/omit outcome, exact shard receipts and raw-profile
  inputs, and verify complete and fail-closed negative cases
- [x] 1.2 Add four explicit Windows package-shard tasks plus one aggregate
  workspace report task, and verify exact mise-owned cargo-llvm-cov 0.9.1
  resolution and unchanged 90% enforcement

## 2. Native workflow

- [x] 2.1 Parallelize Windows package coverage and the existing installation
  acceptance in the reusable native workflow while preserving the Linux and
  macOS monoliths, every existing behavior check and the single native gate
- [x] 2.2 Extend delivery workflow validation and contributor documentation,
  and verify exact package assignment, job dependencies, artifacts and failure
  propagation

## 3. Verification

- [x] 3.1 Run focused helper, workflow, formatting, lint and strict cospec checks
  and record observed local evidence
- [ ] 3.2 Run the exact pull-request head on native CI, record per-shard and
  aggregate timings plus every per-OS coverage/install result, and preserve any
  failure diagnostics
