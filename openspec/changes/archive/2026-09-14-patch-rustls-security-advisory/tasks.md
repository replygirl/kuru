## 1. Patched TLS dependency

- [x] 1.1 Resolve `rustls` 0.23.45 in `Cargo.lock` and verify the lockfile changes only the compatible package version and checksum. *(Observed: the final lockfile diff contains only the `rustls` 0.23.44 to 0.23.45 version and checksum update.)*
- [x] 1.2 Verify the advisory is absent with the repository audit task, then pass the affected connector transport tests, connector all-target/all-feature typecheck, strict Cospec validation, and diff checks. *(Observed: the package-owned advisory refresh and scan passed against database `e2e640471715167f73e22eaf761f2e547adafeec`; connector HTTP tests passed 3/3, A2A tests passed 3/3, connector all-target/all-feature typecheck passed with Rustls 0.23.45, and diff checks passed.)*
