# Tasks

## 1. Refresh exact dependencies

- [x] 1.1 Audit every Cargo.toml and mise.toml against primary release metadata; preserve mise itself and record unchanged latest dependencies.
- [x] 1.2 Update exact Cargo/tool pins and synchronized Rust selectors, regenerate Cargo.lock and all four platform-complete mise lockfiles with the existing CI-pinned mise, and verify package-owned pin/provenance checks.
- [ ] 1.3 Rebuild the Windows ARM64 engine with the updated Go toolchain on its authoritative Linux host, record actual archive/payload checksums, and verify native preparation and offline acceptance.
- [x] 1.4 Verify cospec's standalone single-document/gate contract without its old preload, regenerate managed integrations through cospec update, and refresh dependency/development/release documentation.

## 2. Verify and deliver

- [x] 2.1 Run relevant formatting, host/Windows lint, type checks, repository/tooling checks, docs build/content checks and strict Cospec/managed checks; record observed results and any compatibility corrections.
- [ ] 2.2 Verify behavioral and 90% coverage acceptance without duplicate competing suites, and require all native CI installation/update/terminal/memory checks to pass on the final PR head.
- [ ] 2.3 Archive completed Cospec records through the actual archive gate, commit with normal hooks, publish the dependency PR, and verify that mise version requirements and the original checkout's dirty lockfile remain unchanged.
