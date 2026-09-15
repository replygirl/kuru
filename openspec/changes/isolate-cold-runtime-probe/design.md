## Context

Windows native CI completed extraction and the exact Dolt version probe for an empty updated-install cache, then returned checked `ERROR_ACCESS_DENIED` for 65 attempts to move the verified runtime directory over more than two seconds. Every reconciliation proved the held source identity remained and the destination remained absent. The evidence establishes the durable native-move denial, but neither its mechanism nor any external actor.

The current cold path runs the probe executable from inside the directory that must immediately be renamed. Process and Job completion prove that Kuru's owned process tree is finished; they do not make reliable publication depend on when Windows permits a recently executed image's containing directory to move.

## Goals / Non-Goals

**Goals:** Preserve full byte, identity, privacy, exact-version, cancellation and cache-lock guarantees while making cold publication independent of handles to the executable used for probing.

**Non-Goals:** Increase the activation retry window, attribute the observed handle to a particular external actor, weaken checked directory publication, change the installed cache layout, or alter warm-cache verification.

## Decisions

Cold provisioning will create a private probe sibling under the existing owned staging area, outside the candidate runtime subtree. Through a retained checked candidate directory and executable handle, it will verify the source's exact size, full pinned digest and name identity, copy exactly those bytes to a newly created private file, seal that file as executable, and verify the copied file's exact size, full digest, name identity and distinct file identity.

The exact-version command will execute only the verified copy. The existing retained `(stage, installation lock)` value keeps both candidate and probe sibling alive until the owned probe process tree is reaped, and staging ownership continues through the candidate move. Successful probe-area removal is not a publication prerequisite: after a successful move, staging cleanup is best-effort and cannot undo the published candidate. Cancellation drops the staging owner, including the probe sibling, before releasing the installation lock from the probe owner thread; checked activation failure preserves that private stage before releasing the lock.

Warm verification will continue to digest and probe the installed executable through `CheckedCache`. It performs no directory rename and therefore does not need a second copy.

## Risks / Trade-offs

Cold provisioning copies and hashes the executable one additional time → this occurs only when installing a missing cache and retains the stronger exact-byte validation instead of trading integrity for speed.

An unrelated actor can still deny the final native move → retain the existing checked no-move/uncertain reconciliation and bounded recovery; the change removes only Kuru's dependency on the probed image being movable.

A canceled caller could otherwise release installation authority while probe work continues → retain all cold resources in the existing independent owned-probe thread, with explicit drop order before the lock.
