# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Shared work coordination

`native-windows` owns outstanding application portability corrections and native acceptance; this change owns scheduling of its existing required checks. `refresh-current-dependencies` owns pinned tools already committed on this branch. Neither must archive before the scheduling work can proceed. Coordinate writes to shared workflow/task files and do not mark either change complete based solely on static CI success. The future `bundled-provider-client` change will consume these package-owned preparation dependencies without adding another task workspace.
