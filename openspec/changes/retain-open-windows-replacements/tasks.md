## 1. Windows replacement primitive

- [x] 1.1 Implement exact-handle `FileRenameInformationEx` regular replacement with bounded relative-name encoding, pre-DACL-copy DELETE/SYNCHRONIZE staged authority and unchanged uncertain error classification
- [x] 1.2 Preserve the existing MoveFileEx new-only path, common source/target validation, source flush and same-volume checks

## 2. Regression coverage

- [ ] 2.1 Update the native retained-target replacement regression to prove old-handle identity/bytes and new-path candidate identity/bytes in one successful operation
- [ ] 2.2 Extend the ordinary inheritance fixture through retained-target replacement and prove later parent DACL changes reach the published file
- [ ] 2.3 Exercise a target DACL without file DELETE under a replacement-authorizing parent and prove exact staged authority plus copied final access
- [ ] 2.4 Verify occupied New refusal, mismatch rejection, all platform targets and the native Windows platform suite
