## Context

`kuru-platform::windows::pipe::PrivateListener` is an identity-bound rendezvous for one known child, using an overlapped first-instance pipe. A storage service needs many independent clients whose identities are not known at bind time. Unix currently has checked private directory objects but no service-socket wrapper.

## Goals / Non-Goals

**Goals:** Supply small OS-specific byte channels that a later authenticated service can use without changing the existing child rendezvous.

**Non-Goals:** Framing, service credentials, endpoint discovery, RPC routing, SQL access or a public daemon.

## Decisions

### Unix uses one unique filesystem socket per service generation

The caller provides a retained checked directory and a fresh component name. Bind refuses an occupied name; the socket is changed to mode 0600, checked for owner/type/identity, and connected only through a revalidated owner-private parent. Drop removes the pathname only if it still identifies the bound socket. A crashed owner may leave an orphan pathname, but a new generation chooses a different name and never treats the orphan as authority. Reject a fixed-name delete-and-rebind flow: stale-name removal would couple IPC cleanup to process takeover policy.

### Windows keeps the rendezvous API and adds a repeated listener

The first named-pipe instance still uses `FILE_FLAG_FIRST_PIPE_INSTANCE` and the existing private DACL. Later instances share its name and allow up to 255 simultaneous native instances. `accept_connection` retains its pending overlapped connect across timeout so a future accept does not overwrite kernel-owned state. The new listener returns each accepted `Pipe` and prepares a successor instance; if preparation fails, it retries on the next accept rather than discarding an accepted client. Reject extending `PrivateListener` to unknown clients: its retained child identity check is a distinct security contract.

## Operational surface

Unix binds only an owner-private local filesystem socket; Windows binds only a local named pipe with remote clients rejected. There is no TCP address, container registration, runtime secret, installed helper or new executable. The platform accepts bytes and caps native writes at its existing 16 MiB limit; the memory service supplies bounded frames, timeouts, connection limits and authentication later. The code is compiled for the existing native Windows/macOS/Linux targets; the package-only tests need no Dolt bundle or provider.

## Risks / Trade-offs

- **[Same-user caller]** → Document that checked filesystem mode and DACL protect against other users; the service must authenticate every connection and should not claim same-user isolation.
- **[Unix pathname replacement]** → Retain the parent directory and bound inode identity; drop never removes a different named inode. A same-user process with access to the directory remains inside the account authority.
- **[Windows overlapped cancellation]** → Retain a pending connect across timeout and exercise timeout-then-connect on native Windows CI.
