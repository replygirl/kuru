## Context

Windows `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` returns `ERROR_ACCESS_DENIED`
when a regular destination has an outstanding data handle, even when every
handle includes `FILE_SHARE_DELETE`. Verified file edits retain that destination
handle to bind the original bytes, full native identity and access descriptor to
the durable receipt and to finish inherited access after publication. The native
platform regression already demonstrated that dropping this handle makes the
legacy move succeed, so changing ACLs, shares or timeouts cannot correct the
failure.

## Goals / Non-Goals

**Goals:** Use the supported Windows handle-relative POSIX replacement path,
retain exact checked identities and access evidence, and preserve the existing
new-only, same-volume, error and caller-reconciliation contracts.

**Non-Goals:** Change Unix publication, add a pathname fallback, retry native
publication, broaden file or directory access, copy across volumes, or change
domain receipt policy.

## Decisions

While the staged file still has its private creation DACL, derive a narrow
`DELETE | SYNCHRONIZE` handle from that exact object with the same read/write/delete
shares. Retain it inside the existing opaque copied-access token, bind that token
to the staged file's native identity, and keep it across the ordinary target-DACL
copy. This avoids requiring DELETE from either the ordinary source handle or its
possibly restrictive DACL; the destination parent may be the authority that
permits replacement. Reopen the checked destination directory with the narrow
`FILE_ADD_FILE | SYNCHRONIZE | FILE_READ_ATTRIBUTES` rights, then compare its
full native identity with the retained parent before any rename. For
`ReplaceRegular`, call `NtSetInformationFile(FileRenameInformationEx)` with
`FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS`, naming
the checked destination relative to the retained destination-parent handle.
Use the documented full variable-length rename record and retain the source,
parent, record and `IO_STATUS_BLOCK` through the synchronous call. The source
is reopened without `FILE_FLAG_OVERLAPPED`; if the native call unexpectedly
returns pending, wait on its `SYNCHRONIZE`-capable handle until the final status
is known before allowing a caller to reconcile or mutate again. An isolated
native Windows differential showed the equivalent Win32 extended calls returned
error 87 while this native relative-parent call published with the expected
bytes and identities. No pathname replacement fallback is used.
Windows specifies that POSIX replacement leaves existing handles to the replaced
file usable while subsequent opens of its old name resolve to the renamed file.
This preserves both the caller's original-object evidence and one atomic
namespace transition.

Keep `MoveFileExW` for `Publication::New`. The common preflight still verifies
the retained source/name, destination type, same volume and parent identities and
flushes a newly written source with `sync_all` before dispatch. Both native paths
remain synchronous same-volume metadata operations; `MOVEFILE_COPY_ALLOWED`
remains absent. Microsoft documents `MOVEFILE_WRITE_THROUGH`'s additional flush
guarantee for copy/delete moves, which Kuru forbids, so replacing the same-volume
replacement call does not claim a broader crash-durability guarantee.

Failure to encode the record, derive the early DELETE-capable staged handle or
match its bound staged identity is a definite pre-dispatch rejection. Every error returned by the dispatched native
replacement remains `PublicationPhase::Uncertain`: the API may have changed the
namespace before returning failure, and only retained identity plus the caller's
receipt can reconcile it. There is no legacy fallback because falling back would
reproduce the known AccessDenied boundary or require dropping checked identity
evidence.

## Risks / Trade-offs

- **The variable-length rename record can be malformed.** → Build one bounded,
  aligned buffer from the already-validated single destination component, check
  byte-length conversions, and exercise Unicode/native names in existing platform
  tests.
- **A filesystem may reject POSIX replacement.** → Return the exact native error
  as uncertain and retain the stage/receipt; never retry through a weaker API.
- **A retained destination can hide which object owns the public name.** → Assert
  that the old handle keeps its original identity and bytes while a fresh checked
  open of the destination has the candidate identity and bytes.
- **The copied target DACL may deny DELETE on the file itself.** → Acquire the
  exact staged handle before the copy, prove a late DELETE reopen fails under a
  restrictive fixture DACL, and publish through the retained early handle while
  leaving the final copied DACL unchanged.

## Integration contract

The only external boundary is the Windows native file-information API. The
source handle is reopened from an exact retained object, the target name is
resolved relative to an exact retained parent, and the old target handle is not
used as mutable authority. No schema, SDK, wire format or user-facing interface
changes.
