## 1. Shared memory diagnostic

- [x] 1.1 Move the existing safe Unix directory-remedy composition into the memory filesystem layer and route unit-returning, OwnerOnly parent, and Movable/Pinned retained-handle boundaries through it without changing platform policy, source errors, permissions, ownership, or identity checks.
- [x] 1.2 Cover Dolt cache, versions, cold-probe, install-destination, private-home, server-owned, staging, lifecycle, lock, purge, and remaining production memory-owned private directories through the shared boundary while leaving Inherited payload reads unchanged.

## 2. Regression coverage

- [x] 2.1 Add Unix regressions that fail before the fix and prove exact-path mode-0700 guidance plus no mutation at real provisioning and server boundaries.
- [x] 2.2 Preserve and extend negative controls proving links, foreign-owned paths, other errors, and Windows non-store boundaries do not receive new remedy text, while existing Windows store/migration owner-privacy guidance remains unchanged and no Windows path receives Unix guidance.

## 3. Documentation and verification

- [x] 3.1 Document the existing authored cold-facing order, selection precedence, reason strings, subsequent-member fallback, and no-authority guarantee in both architecture/framework pages.
- [ ] 3.2 Run the scoped memory, docs, core/runtime, format, lint, and typecheck checks and record only observed evidence in the verification ledger.
