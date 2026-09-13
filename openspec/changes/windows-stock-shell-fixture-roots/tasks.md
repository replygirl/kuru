## 1. Native Windows shell fixture

- [x] 1.1 Restore the product-preserved Windows machine roots in `apps/kuru-tui/tests/windows_cli.rs` and verify the cold stock-shell test retains hostile `PSModulePath`, isolated cache roots, unchanged deadlines and bounded stage evidence.
- [ ] 1.2 Run granular formatting and static checks, then verify the exact integrated head with the native Windows shell acceptance and source-install steps in hosted CI.
