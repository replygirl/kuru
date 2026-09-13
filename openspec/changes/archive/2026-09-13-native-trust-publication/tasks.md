## 1. Retained trust storage operations

- [x] 1.1 Add a trust-local checked movable operation directory that retains and revalidates the existing pinned owner-only store authority; verify both capabilities have the same full native identity before file mutation.
- [x] 1.2 Route pending-record publication, reconciliation, and approval-record removal through the movable operation directory while retaining private sealing, lock ownership, checked replacement, and final pinned-directory revalidation; verify no platform retention policy changes.

## 2. Regression coverage

- [x] 2.1 Add a real-filesystem approval-store regression that approves, inspects, replaces, and revokes while storage authority is retained; verify it would expose self-blocked native publication before the fix and preserves expected record state after it.
- [x] 2.2 Run focused trust tests and TUI static checks, record actual results, and leave supported native Windows execution explicitly unrun locally.
