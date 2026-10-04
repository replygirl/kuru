# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Related

- `owned-unix-shell-lifecycle` (archived 2026-09-13): introduced the Unix
  fresh-process-group owner this change extends.
- `unix-snapshot-stand-in-spawn` (archived 2026-10-01): the same
  descriptor-copy-at-spawn class, fixed there in a test.
- `serialize-test-lock-and-spawn` / `serialize-memory-test-lock-and-spawn`
  (archived 2026-09-18/19): test-only gates for the `flock` duplicate class in
  other crates; untouched and not ordered with the platform lock.
- The superseded test-side draft for the hook start wait
  (`connectors-hook-start-wait`, never opened) is not a dependency: its premise
  was refuted by the evidence that motivates this change.
