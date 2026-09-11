## 1. Package and runtime

- [x] 1.1 Add package/config/API boundaries and exact driver dependency; prove SQLx against the verified full engine.
- [x] 1.2 Implement verified provisioning/cache and actual/adversarial installer tests.
- [x] 1.3 Implement authenticated owned supervisor, branch connection setup and lifecycle/parent-crash tests.

## 2. Versioned persistence

- [x] 2.1 Implement async typed memory operations, atomic revisions, operation reconciliation and expected-base candidates with real database tests.
- [x] 2.2 Implement preserved read-only SQLite migration with validated staging activation and interruption tests.

## 3. Runtime integration

- [x] 3.1 Move storage consumers to the new package and propagate async I/O without render-time access.
- [x] 3.2 Implement candidate-pinned dream work and persist-before-publication mutators; verify cancellation and compensating undo.
- [x] 3.3 Integrate CLI lifecycle, remembered preferences, inspection commands and real Dolt fixtures.

## 4. Completion

- [x] 4.1 Integrate package-owned mise/CI fixtures, update docs and AGENTS.md, and run the full local quality gate.
- [x] 4.2 Commit the implementation through hooks, open the reviewed PR, observe hosted checks and resolve failures.
- [x] 4.3 Record observed evidence and strictly validate the completed foundation; retain required checks for the final archive commit and normal merge.

Archive this completed foundation through cospec before the final branch commit.
The PR remains draft while the separately gated embedding and Windows product
changes are implemented and verified.
