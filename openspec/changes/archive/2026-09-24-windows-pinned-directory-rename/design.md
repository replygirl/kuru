## Context

The native A/B/C fixture on Windows job 107807656183 showed that a metadata-only retained handle allowed DELETE-open and pathname rename. Adding `FILE_TRAVERSE` or `FILE_LIST_DIRECTORY` to separate pinned handles made both operations return sharing code 32; after dropping each handle, the same-path operations succeeded. Full retained identities and child bytes stayed unchanged.

## Decisions

Use the narrower `FILE_TRAVERSE` bit only for pinned directory admission, with the existing READ/WRITE share mode and all identity checks. Reuse that admission for checked tree-removal ancestors. For an enumerated descendant, inspect with the existing metadata-only handle because it might be a regular file; once known to be a directory, acquire the strong pin while retaining the first handle and compare full identity and type before recursive pathname traversal. Keep movable directory and regular-file access unchanged. A pinned directory open that cannot obtain traversal authority fails rather than returning a metadata-only handle that does not pin its name.

Regressions exercise `Directory::open` and the checked-removal `pin_directory` helper, not only the raw Win32 probe: while held, DELETE-open and rename must be refused with the original directory still identifiable and readable; after drop, same-path controls must succeed. Existing nested-tree removal and prepared-read connector assertions remain independent consumers.

## Integration contract

Windows `CreateFileW` combines requested access with sharing checks. The observed pinned-name behavior requires traversal access on this runner; no broader enumeration right is needed. Native CI on the final implementation remains decisive across the supported Windows filesystem and ACL setup.

## Risks / Trade-offs

Inherited ACLs that allow metadata inspection but deny traversal may now reject a pinned open. This is a safe, explicit refusal; no weaker fallback or change to movable handles is introduced. Owner-private directories already grant the owner broad access.
