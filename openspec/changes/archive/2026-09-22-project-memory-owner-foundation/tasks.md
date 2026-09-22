## 1. Native owner lifecycle

- [x] 1.1 Add separate start and owner locks, checked private endpoint publication and the packaged internal service entry; verify a real owner rejects a second owner before another engine opens.
- [x] 1.2 Add starter arbitration and native child launch with authenticated readiness; verify two barrier-started client processes reach one service generation and retain both writes.
- [x] 1.3 Retain connections as attachments and close after 30 idle seconds; verify a surviving client remains warm and endpoint/owner authority retire after Dolt reap.

## 2. Typed private transport

- [x] 2.1 Add full project/store/generation/schema/secret handshake and short checked Unix socket routing; verify wrong identities and deep macOS data paths in native tests.
- [x] 2.2 Add bounded typed storage requests, request/reply identity checks and shared frame-memory limits; verify real Dolt typed append/history exchange and oversized-frame rejection.
- [x] 2.3 Tighten startup/error and client-disconnect paths; verify a failed service publication reaps Dolt and a partial request cannot hold owner shutdown indefinitely.

## 3. Reviewable foundation evidence

- [x] 3.1 Run focused native service tests, memory/platform lint and typecheck, format and strict Cospec validation after the final changes; record exact observed results in verification.md.
- [x] 3.2 Confirm the dependent `project-memory-service` change retains facade/CLI/maintenance work and provide the foundation source diff for independent review without advertising public service use.
