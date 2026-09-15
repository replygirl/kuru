## 1. Shared diagnostic contract

- [x] 1.1 Add a private finite shell operational category and bounded projected stderr formatter, and verify it cannot retain a raw source chain.
- [x] 1.2 Adapt Windows shell failures to use that formatter without changing completed shell JSON or cleanup ordering.

## 2. Unix capture retention

- [x] 2.1 Retain Unix capture EOF/projection state through primary failure into the existing cleanup finalizer, and verify pipe ownership and cleanup ordering are preserved.
- [x] 2.2 Format Unix operational failures with the shared category and EOF-complete or pending stderr state without stringifying primary or cleanup errors.

## 3. Evidence and documentation

- [x] 3.1 Add focused native regression coverage for incomplete and EOF-complete stderr operational failures, including raw-chain secrecy and cross-platform grammar.
- [x] 3.2 Verify completed nonzero exits remain structured JSON and update the shell protocol documentation with the bounded diagnostic contract.
- [x] 3.3 Run strict Cospec validation and focused connector shell tests; record observed evidence in verification.
