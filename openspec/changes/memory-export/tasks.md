## 1. Revision-pinned memory reader

- [x] 1.1 Add public snapshot provenance, opaque cursor/page, and storage-record types with commit-qualified active-main construction, historical-schema validation, signed/keyset paging, and EOF counts; verify with real pinned-Dolt snapshot fixtures.
- [x] 1.2 Add rejection and preservation fixtures for candidate views, cross-snapshot cursors, dirty/later main data, signed boundaries, unknown application records, malformed storage, and page/count failures; verify no record is silently omitted.

## 2. Human export command

- [x] 2.1 Add provider-free `kuru memory export` parsing and existing-store early refusal before legacy import, leasing, or directory creation; verify isolated fresh and legacy-sentinel command fixtures.
- [x] 2.2 Add application-owned JSON and Markdown rendering from the same snapshot, private staging, stdout copy, and checked no-replacement publication; verify arbitrary content, output equality, destination refusal, and failed-export non-publication.

## 3. Documentation and focused validation

- [x] 3.1 Document committed full-project export scope, provenance, formats, output behavior, and retained-history boundary in owning public pages; verify documentation checks.
- [x] 3.2 Run focused memory/TUI tests and granular format, lint, and typecheck tasks; record actual evidence and explicitly retain pending native Windows and coordinated coverage rows.
