# Tasks

## 1. Preserve owner-fixture startup diagnostics

- [x] 1.1 Annotate the direct `ServiceOwner::open` call in `packages/kuru-memory/src/service.rs` with the existing bounded fixture startup-log helper, retaining its original error chain and options until annotation completes.
- [x] 1.2 Add a controlled service-owner startup-path regression in `packages/kuru-memory/src/service.rs`: delegate version verification to the actual Dolt binary, then use a fake executable startup failure to verify the raw error lacks the fixture log tail and the helper-annotated error preserves the original startup cause and includes the bounded tail.
- [x] 1.3 After coordinating the Unix behavioral-test slot, run the new regression and focused `service::tests::idle_accept_deadlines_do_not_close_live_attachment`; record observed results without inferring the hosted failure's cause. Evidence: offline package prefetch passed from the checksum-pinned local macOS archive; both focused tests passed 1/1 when rerun outside the sandbox because loopback binding is denied inside it; `kuru-memory:lint`, `format:check` (outside the sandbox for Taplo), and normal commit hooks passed. The hosted Dolt exit cause remains unknown.
