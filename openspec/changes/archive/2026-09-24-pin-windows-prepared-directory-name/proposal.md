## Why

The Windows #78 fixture observed a successful directory rename while a prepared `file_list` retained a `NameRetention::Pinned` handle. The existing test assumes a sharing violation, so a native differential must establish which retained access rights actually prevent an explicit DELETE open and a pathname rename before changing the shared platform API or its authority claim.

## What Changes

- Add a Windows-only platform fixture with fresh private directory peers for the current metadata-only handle and variants adding `FILE_TRAVERSE` or `FILE_LIST_DIRECTORY`.
- For each peer, record exact handle-open, explicit DELETE-open, and `MoveFileExW` outcomes with full directory identity and child-byte checks; reject unexpected object substitution or fixture setup failure.
- Keep the prepared-read product assertion and platform access mode unchanged until the native result selects a separate scoped correction.

## Impact

Only `kuru-platform` Windows test code and its native coverage time change. The fixture uses bounded local filesystem objects and discloses only operation outcomes, OS codes, identities compared as booleans, and fixed test bytes.
