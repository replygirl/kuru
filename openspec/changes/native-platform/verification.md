## 1. Private filesystem and identity [critical]

- [ ] 1.1 @integration (agent) Create/open private native roots and inherited owner-only descendants under Unicode/spaced paths -> actual ACL/mode and full identity checks accept legitimate objects on Windows and Unix.
- [ ] 1.2 @regression (agent) Present unsafe ACLs, junction/symlink ancestors and leaves, hardlinks and special files -> checked operations reject them without changing outside content or permissions.
- [ ] 1.3 @integration (agent) Contend for a real lock and replace names while movable/pinned handles are held -> lock ownership remains exclusive and substitution is prevented or detected before protected work.

## 2. Publication outcomes [critical]

- [ ] 2.1 @integration (agent) Publish new files/directories and replace checked ordinary files on native filesystems -> content and identity are correct, occupied new-only targets remain intact and unsupported moves fail without copy/delete fallback.
- [ ] 2.2 @regression (agent) Exercise pre-move failure and a controlled post-move error boundary using real filesystem operations -> old bytes survive rejection and uncertain results retain enough identity/outcome evidence for caller reconciliation rather than blind retry.

## 3. Process and inheritance ownership [critical]

- [ ] 3.1 @integration (agent) Round-trip Windows argv/environment through compiled fixture executables -> empty, quoted, Unicode and metacharacter values remain exact and duplicate-key handling is explicit.
- [ ] 3.2 @regression (agent) Terminate a Windows fixture owner at creation/startup phases under its Job policy -> no owned child/grandchild persists beyond bounded cleanup and unrelated processes survive.
- [ ] 3.3 @integration (agent) Run concurrent Windows children with distinct handles and a root that exits before a grandchild retaining output -> no inheritance leak, root-only false completion or orphaned output reader occurs.

## 4. Private IPC and cancellation [critical]

- [ ] 4.1 @integration (agent) Stall Windows pipe connect/read/write at parent-controlled handshakes, then cancel and shut down the runtime -> bounded completion with no stranded worker or handle leak.
- [ ] 4.2 @regression (agent) Present preexisting Windows endpoints and unexpected peer processes -> first-instance/private-access/identity checks reject them before private payload transfer.

## 5. Independent native quality gate [critical]

- [ ] 5.1 @integration (agent) Run package-owned format/lint/test/coverage tasks for all primitives on Windows and shared filesystem contracts on Unix without Dolt, bundles or consumer compilation -> actual fixture counts and native coverage pass with no required skips and the existing workspace floor is preserved.
- [ ] 5.2 @integration (agent) Observe the required package-only windows-2025 job and aggregate gate at the candidate commit -> all real primitive checks pass and an injected fixture failure in local gate validation is not hidden by successful Unix/product jobs.
- [ ] 5.3 @manual (agent) Review API/dependency tree, unsafe allowances, mise ownership and CI labels -> the library has no domain dependency, unsafe code is contained and no product Windows support claim has been introduced.
