## Context

Private stage creation supplies a descriptor owned by `TokenUser`, so its initial privacy check can succeed without consulting OWNER RIGHTS. Under an elevated token, changing that same retained file to the distinct `TokenOwner` activates the owner's implicit `READ_CONTROL | WRITE_DAC` rights unless an effective OWNER RIGHTS ACE suppresses them. Windows does not guarantee that the zero-mask create-time ACE remains effective for this transition, and the correct post-assignment check therefore rejects the stage.

The native negative fixtures separately reopen exact objects for DACL mutation. Their mutation must retain enough authority to read the checked ACL state as well as write the replacement DACL.

## Goals / Non-Goals

**Goals:**

- Keep the staged file private before, during, and after assigning a supported source owner.
- Retain exact-object identity and reject every failed owner or privacy transition before publication.
- Let strict native ACL fixtures mutate only their retained objects with explicit test authority.

**Non-Goals:**

- Broaden accepted owners, private principals, publication policy, or production source-file rights.
- Repair an untrusted source DACL, add privileges, or weaken OWNER RIGHTS validation.

## Decisions

Before owner assignment, call the existing `set_private` boundary on the retained staged handle with the same `FILE_ALL_ACCESS` policy used for private stage creation. This path first validates the current private state, installs a protected `TokenUser` grant plus effective zero-rights OWNER RIGHTS suppression, and uses the staged handle's already-required `WRITE_DAC`; it performs no pathname reopen.

Then retain the existing exact-handle `WRITE_OWNER` reopen, supported-owner validation, native assignment, post-assignment privacy validation, exact source/stage owner equality, and source DACL copy. Any failure leaves the stage behind the private directory and prevents publication.

For test-only arbitrary DACL mutation, reopen the retained object with `READ_CONTROL | WRITE_DAC`. Production sources continue to open with `READ_CONTROL` only.

## Integration contract

The handoff remains wholly inside the Windows platform boundary. All three security operations target retained handles: private DACL refresh on the original staged handle, owner assignment on a short-lived `ReOpenFile` handle derived from it, and source DACL copy back to the original staged handle. The source descriptor retains its owner SID through assignment, and publication still requires the later filesystem identity and access-token checks.

## Risks / Trade-offs

- **A DACL refresh could hide a weak stage.** → `set_private` begins with the existing strict `require_private`; it repairs no rejected stage.
- **Owner assignment could activate implicit control rights.** → The protected OWNER RIGHTS suppression is installed first and checked again after assignment.
- **Fixture authority could leak into production.** → `READ_CONTROL | WRITE_DAC` is confined to the test helper; product source opens remain unchanged.
