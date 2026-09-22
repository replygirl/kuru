## 1. Registry and editor

- [x] 1.1 Add a typed TUI built-in registry with canonical names, usage/help metadata and dispatch IDs; verify unique names and parser/help parity in focused tests.
- [x] 1.2 Route existing public UI and runtime slash handling through registry IDs, keeping internal review/cancel tokens modal-only; verify existing command and unknown-name behavior in focused tests.
- [x] 1.3 Add bounded deterministic leading-token `Tab` completion with edit reset and modal priority; verify unique/ambiguous prefixes, arguments and ordinary digits in view and real-PTY tests.

## 2. Working local commands and guidance

- [x] 2.1 Implement `/clear` and `/status` against the owned current View without provider or memory effects; verify visible clearing, persisted same-session continuity and current facts in real PTY fixtures.
- [x] 2.2 Update usage and published command references from the working registry surface; verify documentation omits unimplemented commands and describes retained history.

## 3. Integrated evidence

- [x] 3.1 Run owning TUI tests, lint, typecheck, format, docs checks, strict Cospec validation/apply, managed drift and diff checks; record observed results and any deferred native hook/CI evidence before archive.
