## ADDED Requirements

### Requirement: Exactly-once partitioned native test evidence

Native CI test suites that run in partitions SHALL run every test that libtest lists for every executable in the
built inventory exactly once across the partitions of one OS. Each partition MUST build the complete inventory for
its scope, list every executable's tests, and run only the tests deterministically assigned to it with explicit
exact-name selections that fit the native command-line limit. Libtest's announced count MUST equal each selection.
Each partition receipt MUST record the source commit and tree, Cargo.lock digest, Rust, Cargo and coverage-tool
identity, a digest of the profile environment, the inventory digest, the partition index and count, and each
executable's listed and assigned test names. The receipt MUST store those name lists, hash them and upload them.
An executable whose tests are not run on a host MUST be recorded with its reason and never omitted silently.

#### Scenario: A partition skips or repeats a test
- **WHEN** the assigned test sets for one executable overlap, or their union differs from that executable's listed tests
- **THEN** the merge fails naming the executable and writes no report.

#### Scenario: A selection runs a different number of tests
- **WHEN** libtest announces a test count that differs from the names selected for that invocation
- **THEN** the partition fails even if the executable exited successfully.

### Requirement: Receipt-agreement merge with a per-OS coverage gate

Each OS's partition evidence SHALL be merged on one Linux runner. The merge does not rebuild the inventory.
Instead, N independently built partitions MUST agree. The merge MUST require a receipt for every partition index.
It MUST require all receipts to be identical on source, tree, Cargo.lock, toolchain, profile environment, mode and
inventory digest, and consistent with the OS label and the expected source commit. It MUST prove every executable's
partitions disjoint and complete against its listed tests. Any missing, extra, mismatched or unverifiable receipt
MUST fail the merge before any report exists. For an instrumented OS, the merge MUST require identical source-file
and line sets in every partition's normalized LCOV, union their hit counts, write that OS's merged LCOV and enforce
at least 90% line coverage for that OS. No partition percentage may be averaged.

#### Scenario: A partition receipt is missing
- **WHEN** one partition index of an OS has no uploaded receipt
- **THEN** that OS's merge fails and produces no LCOV and no pass.

#### Scenario: Partitions were built from different inputs
- **WHEN** two receipts differ in source, toolchain, profile environment or inventory digest
- **THEN** the merge fails without merging any coverage.

#### Scenario: Merged coverage is below the gate
- **WHEN** the union of an instrumented OS's partitions covers less than 90 percent of its lines
- **THEN** that OS's merge fails, and no other OS's result can satisfy it.

### Requirement: Evidence-neutral seeded dependency cache

A partition MAY speed its build by importing cached dependency artifacts. It SHALL still create a fresh target and
import only allow-listed artifacts of non-workspace packages into it before its first build. It MUST NOT import
workspace-crate artifacts, raw profiles, symbolic links or private state and supervisor directories. A missing,
evicted or stale seed MUST only make the run slower and never cause a failure or change a receipt. Only the default
branch SHALL export and save a seed, and it SHALL export only what an import would accept.

#### Scenario: The seed cache was evicted
- **WHEN** no seed is restored for a partition
- **THEN** the partition rebuilds its dependencies and produces the same receipts as with a seed, and its job ledger records the miss.

#### Scenario: A seed contains a workspace artifact or link
- **WHEN** the restored seed holds a workspace crate's output, a raw profile or a symbolic link
- **THEN** that entry is refused and counted, and Cargo builds the workspace crate itself.
