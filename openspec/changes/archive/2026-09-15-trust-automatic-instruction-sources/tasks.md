## 1. Snapshot and authority claim

- [x] 1.1 Capture bounded typed instruction sources, preserve outer-to-local order including the project root, and bind ordered source identities and exact bytes into the complete manifest.
- [x] 1.2 Return formatted prompt instructions only from the immutable snapshot and remove the production runtime pathname reread.

## 2. Command preflight and review

- [x] 2.1 Apply instruction authority to runtime-producing commands while preserving authentication, inspection, memory, catalog and direct-tool command subsets.
- [x] 2.2 Render a fixed instruction claim with bounded ordered source labels and no instruction contents.

## 3. Regression coverage and documentation

- [x] 3.1 Add regressions proving add/change/remove/replacement staleness, root inclusion, source ordering, snapshot byte consistency and provider-free refusal before activation.
- [x] 3.2 Document instruction-source trust and the command matrix, then run core and TUI trust checks plus formatting and lint appropriate to the change.
