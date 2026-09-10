# kuru-platform

Small native operations for Kuru's domain packages. Filesystem APIs support Unix
and Windows; the Windows module supplies process ownership and private local IPC.
The package builds independently of the application and memory engine.

Checked directory operations retain file identity, validate private ownership and
permissions, reject unsafe aliases and accept literal child names. Publication
distinguishes failure before a move from an uncertain result after a native
operation. Callers retain their own receipts and reconcile actual identity before
retrying. Windows write-through moves and Unix parent synchronization each follow
their native contract; the API does not promise identical power-loss behavior.
File publication validates the candidate against the destination's access policy
before moving it. Directory moves require matching policies; changing a
directory's access policy is a separate, explicit operation.

On Windows, close destination data handles before replacement and all descendant
data handles before a directory move. Keep lifecycle locks beside the directory
being moved, so their ownership survives publication. Movable handles permit
delete sharing; they do not remove these native restrictions. A failed native
move still requires identity reconciliation before another attempt.

Private Windows roots explicitly belong to the current token user. Ordinary
descendants can receive a different default token owner under elevation. New
private ACLs therefore include an inheritable zero-access OWNER RIGHTS entry,
which suppresses the owner's implicit access. Validation accepts that alternate
owner only when it exactly matches the current token's default owner and this
suppression is effective. Positive grants still belong only to the token user;
existing unsafe ACLs are rejected without repair. See Microsoft's
[ownership rules](https://learn.microsoft.com/en-us/windows/win32/secauthz/owner-of-a-new-object)
and [OWNER RIGHTS semantics](https://learn.microsoft.com/en-us/windows-server/identity/ad-ds/manage/understand-special-identities-groups#owner-rights).

Windows processes receive explicit executable, argument, environment, stdio and
lifetime descriptions. Owned jobs are assigned during process creation and retain
the process tree through cleanup. A trusted supervisor has an explicit separate
lifetime policy so its caller can request graceful cleanup over a private pipe.
An owned process handle supplies identity; a saved PID grants no cleanup authority.

Use this process API for all concurrent Windows launches. Its explicit allowlist
controls what each created child receives. An unrelated legacy spawn can still
inherit handles temporarily marked inheritable; the library cannot coordinate
Rust's private spawn lock. Keep consumer launch paths and their dependencies
consistent with this requirement.

Inherited stdio and a child-connected lifetime channel have distinct peer identity
rules. Both use overlapped local pipes and explicit access controls. Domain code
owns framing, SQL, shell interpretation and updater recovery.

The public APIs are safe Rust. Necessary Windows FFI stays in the audited Windows
implementation modules with owned handles and allocations. Other modules and
consumer crates retain their unsafe-code restrictions. The filesystem boundary
protects private state from ordinary other-user access; it is not a sandbox against
a malicious process running as the same user.

Run the package checks through mise:

```sh
mise run //packages/kuru-platform:check:native
```

This runs package formatting, strict lint, type checking, behavioral fixtures and
coverage with the 90% line gate. Windows process/IPC fixtures execute on the required
native Windows CI job; Unix runs the shared filesystem checks. Tests use isolated
paths and compiled Rust fixtures. Passing this package's checks establishes its
native operations; full Windows application acceptance belongs to the consumer
integration change.

For Windows type checking from another host, prepare the pinned standard library
and run the package task before native CI:

```sh
mise run //packages/kuru-platform:setup:windows
mise run //packages/kuru-platform:typecheck:windows
```
