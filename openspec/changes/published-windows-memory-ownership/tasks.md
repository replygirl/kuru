## 1. Reproduce and correct native process ownership

- [ ] 1.1 Add a native Windows regression that fails on the original published verifier Job policy and passes when only IndependentService children may break away.
- [x] 1.2 Apply the existing breakaway-permitting Job to published verifier commands and add fixed command-phase diagnostics.

## 2. Retire the isolated memory service

- [x] 2.1 Use installed Kuru's existing `memory purge --yes` on the exact verifier-created project and data root after all evidence assertions, with no memory dependency in delivery tooling.
- [ ] 2.2 On verification errors, attempt bounded authenticated cleanup; retain the isolated root and both errors if retirement is uncertain. Add normal and error-path native fixture checks.

## 3. Validate and deliver

- [ ] 3.1 Run focused owning checks, strict Cospec validation/apply and normal hooked push; record observed results without calling unrun checks green.
- [ ] 3.2 Obtain native Windows CI proof, archive this change, and merge only after exact-head gates pass. Preserve v0.8.0 assets and original run.
