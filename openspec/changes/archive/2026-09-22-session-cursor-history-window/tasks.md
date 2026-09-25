## 1. Checked storage window

- [x] 1.1 Add the strict cursor-relative newest-window DTO, request validation and one-view store query using the shared row/byte budget, and verify a real-Dolt fixture returns the newest exact sequences plus the total eligible count across more than 1,024 interleaved rows.
- [x] 1.2 Cover zero limit, exact-latest cursor, invalid cursor/limit and serialized-byte exhaustion, and verify each result or refusal preserves the documented bound without consuming or relabeling rows.

## 2. Managed projection

- [x] 2.1 Route the read-only operation through local and remote facades and the managed service, advance the protocol minor, and verify all-target memory typecheck plus exact response decoding.
- [x] 2.2 Exercise managed main and candidate views with independent appends, and verify exact view/revision provenance, local/remote parity, candidate isolation and absence of mutation receipts or fences.

## 3. Repository closure

- [x] 3.1 Record observed focused evidence, pass formatting, strict Cospec validation and diff checks, then archive the completed change before the conventional branch commit.
