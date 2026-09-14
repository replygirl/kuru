## 1. Regression

- [x] 1.1 Add a deterministic first-close-reply waker regression and verify it
  fails against the old admission ordering

## 2. Implementation

- [x] 2.1 Close RPC command admission before cleanup publishes its close result
- [x] 2.2 Resolve a timed-out close reply from authoritative completion while
  retaining errors for unconfirmed cleanup

## 3. Verification

- [x] 3.1 Verify the regression passes after the fix and repeated close completes
- [x] 3.2 Run the focused RPC suite and connector typecheck
