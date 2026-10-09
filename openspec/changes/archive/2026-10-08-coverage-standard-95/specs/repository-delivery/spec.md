# Spec Delta

## MODIFIED Requirements

### Requirement: Reproducible workspace quality gates

The repository SHALL use apps/ and packages/, pinned Rust/mise tooling, hk hooks,
cospec change gates, Cargo.lock, and CI checks for format, lint, tests and at
least 95% workspace line coverage from meaningful behavioral tests. Native
Windows x64/MSVC verification SHALL be required by the aggregate CI gate.
Windows-only behavior MUST be executed and measured on Windows, not inferred
from Unix checks or compiled-out tests.
The existing standalone archive and native platform coverage checks SHALL
also enforce at least 95% by their cargo-llvm-cov line metric. Test additions
SHALL exercise observable contracts and failure modes; application exclusions,
metric substitutions and threshold reductions MUST NOT satisfy the minimum.

#### Scenario: Coverage regression
- **WHEN** measured workspace line coverage is below 95 percent
- **THEN** the coverage check fails rather than silently reducing the threshold or excluding application code.

#### Scenario: Native Windows gate fails
- **WHEN** the Windows job has a failing required test or coverage check
- **THEN** the aggregate gate fails regardless of successful macOS and Linux jobs.

#### Scenario: Package coverage is below the minimum
- **WHEN** a standalone archive or native platform coverage check measures less than 95 percent
- **THEN** that package check fails while its complete application source inventory and existing line metric remain included.

### Requirement: Receipt-agreement merge with a per-OS coverage gate

Each OS's partition evidence SHALL be merged on one Linux runner. The merge does not rebuild the inventory.
Instead, N independently built partitions MUST agree. The merge MUST require a receipt for every partition index.
It MUST require all receipts to be identical on source, tree, Cargo.lock, toolchain, profile environment, mode and
inventory digest, and consistent with the OS label and the expected source commit. The profile environment is the
named build and test process variables and cargo-llvm-cov's `show-env` coverage environment. Only the partition's
target path and the merge-pool size `N` of a `%<N>m` specifier in the profile file name, which `show-env` derives from
the host's available parallelism, MAY be replaced by fixed tokens before digesting. Every other value MUST be compared
as written. Each partition MUST upload the neutralised environment whose digest its receipt carries. That environment
MUST NOT contain credentials. The merge MUST refuse an uploaded environment that differs from its receipt's digest
and, when two partitions' environments differ, MUST name every differing key. It MUST prove every executable's
partitions disjoint and complete against its listed tests. Any missing, extra, mismatched or unverifiable receipt
MUST fail the merge before any report exists. Each instrumented partition MUST export, per function instantiation,
its main source file, group location and mapped and covered line sets, derived from its own llvm-cov export as
llvm-cov derives line statistics. Before its receipt is written, that export MUST reproduce the partition's own
cargo-llvm-cov `--summary-only` line figures exactly, per file and in total, and the receipt MUST carry the export's
digest. For an instrumented OS, the merge MUST refuse a receipt without that digest or an export that differs from
it. It MUST require identical source files, instantiations, group locations and mapped lines in every partition,
union each instantiation's covered lines, and enforce at least 95% by cargo-llvm-cov's line metric: per file, the
sum over instantiation groups of the most mapped and the most covered lines of any instantiation. This is the
metric `mise run coverage` holds to `--fail-under-lines 95`. The merge MUST also require identical source-file and
line sets in every partition's normalized LCOV, union their hit counts and write that OS's merged LCOV, whose
unique-line figure is informational. No partition percentage may be averaged.

#### Scenario: A partition receipt is missing
- **WHEN** one partition index of an OS has no uploaded receipt
- **THEN** that OS's merge fails and produces no LCOV and no pass.

#### Scenario: Partitions were built from different inputs
- **WHEN** two receipts differ in source, toolchain, profile environment or inventory digest
- **THEN** the merge fails without merging any coverage.

#### Scenario: Merged coverage is below the gate
- **WHEN** the union of an instrumented OS's partitions covers less than 95 percent of its lines by cargo-llvm-cov's line metric
- **THEN** that OS's merge fails, and no other OS's result can satisfy it.

#### Scenario: A partition's line export does not reproduce its own summary
- **WHEN** the figures derived from a partition's line export differ from its cargo-llvm-cov `--summary-only` figures in any file or in total
- **THEN** the partition fails, naming the first mismatching files, and writes no receipt.

#### Scenario: A receipt lacks the line export digest
- **WHEN** an instrumented partition's receipt has no line export digest, or its uploaded export differs from that digest
- **THEN** that OS's merge fails and produces no report.

#### Scenario: Partitions ran on hosts with different CPU counts
- **WHEN** two partitions of one OS were built identically on runners whose available parallelism differs, so `show-env` reported `%3m` and `%5m` merge pools at different target paths
- **THEN** their profile-environment digests are equal and the merge accepts them.

#### Scenario: Profile environments genuinely differ
- **WHEN** two partitions' profile environments differ in any other value, such as `RUSTFLAGS`, a size-less `%m` profile file pattern or another file-name specifier
- **THEN** the merge fails naming every differing key with both values and writes no report.

#### Scenario: An uploaded profile environment misdescribes its receipt
- **WHEN** a partition's uploaded profile environment is missing or does not digest to its receipt's profile-environment digest
- **THEN** the merge fails before comparing receipts and writes no report.
