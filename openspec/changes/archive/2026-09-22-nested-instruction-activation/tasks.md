## 1. Deterministic nested snapshots

- [x] 1.1 Extend core instruction capture with a bounded checked-source cache and canonical union of applicable nested directories; verify root/ancestor bytes remain immutable, activation order is independent, and import/cap/notice unit fixtures pass.
- [x] 1.2 Derive exact extended manifests and sorted active nested source path-digest sets without rereading old files; verify replaced/linked/changed source, sibling isolation, and source identity fixtures pass.

## 2. Persistent and foreground trust

- [x] 2.1 Add strict backward-compatible v2 approval records with base plus bounded nested entries, fresh publication generation, and exact lookup under the existing root lock; verify v1 migration, matching base startup, stale bytes, malformed records, and bounds in trust-store tests.
- [x] 2.2 Preserve physical revoke and make persistent nested writes compare the pre-review generation and base under lock; verify concurrent revoke/recreate, same-base merge, changed-base clearing, and absent-state explicit approval fixtures.
- [x] 2.3 Route path-qualified complete-manifest review through the existing foreground TUI surface with once/persist/deny and bounded headless diagnostics; verify a real PTY review and headless trust-required/once flows.

## 3. Checked tool admission and actor continuation

- [x] 3.1 Add an actor-only instruction gate after exact file permission and checked target revalidation, leaving direct CLI tools independent; verify denied targets never activate nested sources and direct tools remain unchanged.
- [x] 3.2 Make grep/glob collect their bounded candidate union and exact per-candidate decisions before instruction review or result exposure; verify allowed/denied mixed search, independent hidden/ignore rules, result order, and truthful cap omissions.
- [x] 3.3 Pass a typed reviewed instruction update and replan result into the runtime; verify a first newly instructed mutation has no effect or replay, settles its call once, and the next fake-provider request receives the updated prompt.

## 4. Documentation and acceptance

- [x] 4.1 Update configuration, tool and user documentation to describe path scope, approval choices, replan, headless remedy and caps; verify docs checks and no premature command/skill support claim.
- [x] 4.2 Complete all verification ledger rows with observed focused, PTY, package and native evidence; run format, lint, typecheck, docs, strict Cospec validation/apply, diff check, archive and normal commit hooks before delivery. Local evidence is complete; native hook/CI is explicitly deferred to delivery after this archive and commit.
