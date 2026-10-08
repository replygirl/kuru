# Proposal

## Why

The native Windows suite passes 136 tests but still fails its unchanged 95% gate. Its remaining report exposes useful publication-authority, ACL-policy and bounded current-image name defenses.

## What Changes

- Exercise missing native publication filenames in `src/fs/windows.rs`; preserve exact held objects and adjacent payloads. Retained-parent mismatch already has native passing coverage.
- Exercise existing fail-closed unfamiliar ACL and bounded policy-token contracts in `src/windows/security.rs`, using genuine ACL installation and pure codec inputs without simulating kernel faults.
- Unit-test `src/windows/process/image.rs` exact UTF-16 spelling and rejection of truncated, unterminated, interior-NUL and wrong-namespace names; retain independent native mapping acceptance.

## Impact

Only platform tests change. No production policy, unsafe boundary, dependency pin, coverage exclusion or threshold changes. Native Windows CI remains the behavior and 95% acceptance gate.
