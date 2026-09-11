## Why

CI 34617836357 passed the command's owned-tree cleanup and complete output
assertions, then failed its instantaneous file-lock reacquisition. Windows
[documents deferred lock release](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex)
after process termination; the test must observe release within a bounded wait.

## What Changes

- Update `packages/kuru-delivery/tests/windows_command_capture.rs` to retain one
  file handle while waiting at most five seconds for the actual descendant lock.
  Retry only contention; fail other errors immediately and a persistent lock at
  the deadline. Preserve the original process, output and cleanup assertions.
- Use an actual held-file control to prove the bounded observation cannot pass
  while the lease remains held, then verify reacquisition after explicit release.

## Impact

Only the Windows delivery test and its cospec record change. Production process
ownership, command deadlines, authentication, runtime behavior, dependencies and
CI scheduling remain unchanged. The existing native suite verifies the correction;
record its result separately from the earlier run's plain shell pass.
