# Tasks

## 1. Use the official HTTPS Ubuntu archive

- [x] 1.1 Update the native-test and release Ubuntu bootstrap steps to replace only the canonical mirror-list URI with the official HTTPS archive while preserving deb822 metadata and apt bounds; update the existing `FIXED_APT` fixture to match.
- [x] 1.2 Run the existing delivery repository-validation tests for Ubuntu apt isolation/rejection and the repository tooling checks; inspect that no apt sourceparts, retry, timeout, release-dispatch, or unrelated workflow behavior changed.
