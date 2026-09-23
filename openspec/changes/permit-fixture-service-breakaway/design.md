## Context

The Windows delivery command facade places a command and its descendants in an ordinary kill-on-close `OwnedJob`, and `NativeChild::wait` correctly waits for the whole Job to become empty. A managed memory owner requests explicit breakaway so it can remain warm after the starter exits. When the immediate Job forbids breakaway, the supported contained-owner fallback starts the owner inside that Job instead; the command root exits and closes stdout/stderr, but the wrapper waits on the live owner and never returns to the fixture's authenticated cleanup.

Windows nested-Job semantics provide the needed boundary. If the immediate Job permits explicit breakaway, a child using `CREATE_BREAKAWAY_FROM_JOB` leaves that Job and moves up the hierarchy only until the first nonpermitting ancestor. Ordinary descendants which do not request breakaway remain owned by the command Job.

## Decisions

Add an explicit fixture selector to the Windows delivery command facade. It changes only the immediate Job from `OwnedJob` to the existing `FixtureBreakawayJob`; the default remains ordinary full-tree ownership. Application fixtures opt in only for command chains that can launch Kuru's managed service. The ConPTY fixture applies the same lifetime to the Kuru child it owns.

The service remains inside any nonpermitting runner/host Job. Existing `ServiceCleanup` guards retire exact project owners before temporary roots are deleted. Fixtures which currently open managed memory without such an ownership boundary add an exact authenticated retirement step; they do not wait for the product idle timeout or kill an inferred process.

## Operational surface

This changes native Windows test topology only. It adds no environment switch, timeout, retry, production Job flag, user configuration, or release behavior. Hosted Windows acceptance must demonstrate that command output completes, the same-generation owner stays warm across sequential fixture commands, explicit cleanup removes the endpoint and lifecycle lock, and the enclosing runner still owns residual processes.

## Risks / Trade-offs

- A fixture could unintentionally let an unrelated descendant escape. The permitting Job still requires the descendant to request explicit breakaway; call sites are limited to known Kuru/service fixture chains.
- The service could outlive its temporary root. Every opted-in memory fixture retains or adds authenticated retirement before root deletion, and cleanup failure retains the root and fails the test.
- The correction could hide a real command-child leak. Children without explicit breakaway remain in the immediate Job, so the existing whole-tree quiescence check remains effective.
