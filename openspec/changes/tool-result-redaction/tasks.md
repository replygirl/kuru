## 1. Finite scanner

- [x] 1.1 Add the package-private fixed-rule whole-value and streaming scanner with stable marker, checked relative allocation and no new dependency; verify every detector, delimiter, floor, quoted-name/value, AWS boundary, private-block and EOF case with table-driven unit tests
- [x] 1.2 Add false-positive/false-negative and adversarial chunk/allocation controls; verify unmatched hashes, identifiers, generic encodings and unsupported shapes remain exact while every-byte splits equal whole-value projection

## 2. Typed tool boundary

- [x] 2.1 Introduce private text/JSON/application-error result types and one ToolHost projection/serialization boundary while retaining public `Result<String>`; verify nested sensitive/Authorization fields, partial key projection, collision withholding and arbitrary text non-coercion with focused connector tests
- [x] 2.2 Replace outward raw tool error chains with typed category plus projected detail and retain typed MCP application-error distinction; verify Display, alternate Display, Debug and complete source chains contain no synthetic raw credential

## 3. Producer integration

- [ ] 3.1 Route file reads/listings/writes and Unix/Windows shell results through the typed boundary without changing inputs, side effects or producer caps; verify real files retain identity/bytes and native shell/listing near-limit behavior remains successful — unrun: required supported native Windows shell evidence remains pending
- [x] 3.2 Route stdio and HTTP MCP success and `isError` content through typed projection without changing availability; verify isolated protocol fixtures return useful valid projected content and transport failures retain safe typed causes
- [x] 3.3 Keep actual MCP stderr capture outside this change while exposing the scanner for a later owned drain; verify only synthetic stderr-like chunk/tail/EOF inputs in this change and make no process/RPC ownership claim

## 4. CLI and runtime integration

- [x] 4.1 Exercise direct CLI file, native shell and fake MCP tool results with synthetic credentials; verify stdout/stderr show the same marker, ordinary bytes remain unchanged and no original data is rewritten
- [x] 4.2 Add connector-owned whole-marker truncation at the runtime's current and optional tool-receipt byte limits without changing generic chat truncation; exercise a real tool turn, persistence, reopen and subsequent provider context, including adjacent-marker/UTF-8/tiny-budget boundaries; verify only the projected value crosses ToolHost, retained markers stay whole, and earlier history/source/side-effect data remain exact
- [ ] 4.3 Run the supported native Windows CLI/runtime, PowerShell and real-memory projection fixtures; verify parity with Unix behavior and record native evidence rather than inferring it from compilation — unrun: required supported native Windows CI remains pending

## 5. Documentation and coordinated verification

- [x] 5.1 Update curated tool/protocol documentation with the exact marker, detector categories, local floors, producer limits, preserved inputs/history and known false-positive/false-negative boundary; verify the built site contains the guidance and no private design note
- [x] 5.2 Run focused package tests while implementing, then run connector/TUI/runtime static checks and all behavioral targets once under the coordinated workspace coverage task; verify the full ledger with explicit command exits and preserve the 90% line gate without a duplicate broad suite
