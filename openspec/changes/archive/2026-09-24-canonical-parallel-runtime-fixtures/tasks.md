## 1. Runtime fixture roots

- [x] 1.1 The three parallel-tool fixtures in `packages/kuru-runtime/src/tests.rs` now pass one canonical tempdir path to both retained `Directory::open` and `Harness`; their effect, receipt and identity assertions are unchanged (independent source review clear).
- [x] 1.2 Owning runtime all-target typecheck passed and the three exact real-Dolt filters passed 1/1 each on macOS after a sandbox-only loopback denial. Corrected-source Windows execution is unrun and remains a mandatory final-head CI merge gate; no Windows pass is claimed by this local evidence.
