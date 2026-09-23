## Context

Windows chooses the owner of an ordinary file from `TokenOwner`, which can differ from `TokenUser` in an elevated process. Kuru deliberately creates each private staging root with `TokenUser` ownership, and its staged payload currently inherits an explicit `TokenUser` owner as well. Requiring that staged owner to already equal an ordinary source owner therefore rejects the valid elevated case before the source DACL can be copied.

The owner matters independently of the DACL because Windows gives an owner implicit control rights unless an effective OWNER RIGHTS ACE changes them. Copying only the DACL while retaining a different owner would not preserve the source access policy.

## Goals / Non-Goals

**Goals:**

- Preserve the exact source owner and DACL on a staged payload while it remains unreachable behind the retained private stage.
- Acquire `WRITE_OWNER` only on the exact retained staged object during the handoff.
- Reject an unassignable or unsupported owner before publication.

**Non-Goals:**

- Changing general private-root ownership, connector policy, Windows privileges, retries or publication rules.
- Claiming isolation from a process running with the same user authority.

## Decisions

Use `ReOpenFile` on the retained staged handle to obtain one short-lived `WRITE_OWNER` handle with the same movable sharing contract. This avoids adding owner-changing authority to general file opens and avoids a pathname lookup. Assign the owner returned from `GetSecurityInfo` on the retained source, close the narrow handle, then require both exact source/staged owner equality and the existing private-status invariant before copying the DACL.

The private-status check deliberately accepts either `TokenUser` or the exact current `TokenOwner`; a distinct `TokenOwner` remains safe only with the already-required effective zero-access OWNER RIGHTS ACE. Any other owner, failed assignment or failed check rejects the handoff before publication.

The failing ACL fixture reopens its retained exact file handle with `WRITE_DAC` before calling `SetSecurityInfo`. Production source handles remain read-control-only because the product never mutates the source DACL.

## Integration contract

The integration boundary is the Win32 handle/security API. `ReOpenFile` derives authority from the retained staged object rather than a path; `GetSecurityInfo` retains the source descriptor through `SetSecurityInfo`; both handles are RAII-owned and the source and stage identities remain those already checked by the filesystem boundary. The staged file stays under its owner-private parent until owner/DACL handoff and all later identity checks complete.

## Risks / Trade-offs

- **Owner assignment can require authority the process does not hold.** → Return the native failure and leave the private stage unpublished; never accept an approximate owner.
- **Changing owner could activate implicit owner rights.** → Revalidate private status immediately after assignment, including exact `TokenOwner` and effective OWNER RIGHTS suppression, before copying the source DACL.
- **A second handle can change sharing behavior.** → Request only `WRITE_OWNER`, retain the existing read/write/delete share modes, close it immediately, and use the original retained handle for every subsequent check and publication.
