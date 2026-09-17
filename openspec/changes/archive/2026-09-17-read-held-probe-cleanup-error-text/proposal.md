## Why

Windows native CI confirmed that checked temporary-stage cleanup reports OS error 32 as formatted error text, while the held-probe fixture expects a downcastable `std::io::Error` in the anyhow chain. The assertion fails after correctly verified publication and cleanup refusal.

## What Changes

- Update the Windows held-probe assertion in `packages/kuru-memory/src/provision/native_tests.rs` to check the observed OS error 32 text while retaining the independent raw native-code assertion on the preactivation rename.

## Impact

Test-only assertion correction; no production behavior, workflow, timeout, or test workload change.
