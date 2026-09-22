## 1. Search foundation

- [x] 1.1 Pin upstream Rust ignore and grep crates in workspace dependency
  metadata, add their notices, and verify Cargo resolves exact versions without
  an `rg` executable.
- [x] 1.2 Extend native tool/permission selectors and published schema for
  grep and glob, and verify parser/schema parity plus path-rule matching tests.

## 2. Checked native tools

- [x] 2.1 Implement bounded checked glob traversal with explicit hidden/ignore
  switches and per-candidate permission/protection/link handling; verify nested,
  ignored, hidden, linked, protected, and bounded-result fixtures.
- [x] 2.2 Implement bounded UTF-8 grep over checked glob candidates, with
  compiled-regex error handling and per-candidate permission/protection/link
  handling; verify Unicode, binary, large-file, invalid-pattern, denial, and
  output-bound fixtures.
- [x] 2.3 Implement additive line paging for file_read, preserving unpaged
  response compatibility; verify continuation, Unicode, binary, large-file,
  and invalid-range cases.

## 3. Documentation and verification

- [x] 3.1 Document native search and paging semantics, limits, and permission
  behavior in the curated protocol reference; verify the docs build and link
  check.
- [x] 3.2 Run focused connector/core tests plus relevant format, lint, schema,
  docs, and Cospec validation checks; record observed outcomes in this change.
