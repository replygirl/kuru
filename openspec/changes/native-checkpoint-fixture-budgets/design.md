## Context

The failed Windows run exercised real PowerShell, loopback TCP, ConPTY and Dolt
under workspace coverage. Several tests placed shorter wrapper budgets around
those operations, depended on environment variables intentionally removed by the
new shell policy, or queried a command that now deliberately avoids opening
memory. The runtime replay then parsed an error receipt as success JSON and hid
the causal timeout. The Unix PTY helper used `openpty` and duplicated the slave
onto child stdio without creating a child session or assigning the slave as its
controlling terminal. Crossterm reads raw input from terminal stdin but prefers
`/dev/tty` for dimensions, splitting the real input/output PTY from an inherited
outer runner terminal.

## Goals / Non-Goals

**Goals:**

- Preserve the real native integrations and their observable authority,
  persistence, cleanup and retry assertions.
- Let test observation budgets encompass the unchanged production operation.
- Make a future failure report the actual tool receipt.
- Make Unix terminal tests independent of the invoking terminal's dimensions
  while retaining real PTY input, rendering, resize and restoration checks.

**Non-Goals:**

- Changing production retry or shell deadlines.
- Expanding the built-in shell environment allowlist.
- Replacing real PowerShell, loopback sockets, ConPTY or memory with mocks.

## Decisions

- Outer fixture waits exceed the production deadline they observe. A shorter
  outer wait cannot distinguish a product timeout from fixture cancellation.
- Normal Windows compatibility variables are supplied to the deliberately
  isolated test process. Fake credentials, proxy values, module paths and system
  overrides remain hostile so projection is still proved.
- CLI shell evidence uses literal, test-controlled values encoded into the
  PowerShell source. Custom `KURU_*` inheritance would contradict the policy.
- The ToolHost shell fixture also encodes the lexical `PATH`, `HOME`, `TEMP` and
  native system values supplied by its isolated parent. It does not reconstruct
  expected paths from PowerShell's working-directory spelling. Failed checks are
  labeled inside the already bounded shell receipt without dumping the process
  environment.
- The same fixture appends only fixed stage names to a controlled private file at
  script entry and around file writes, `where.exe`, the batch probe and final
  checks. On shell failure the test reads at most 4 KiB of those labels. This
  localizes a native stall without exposing environment contents, changing the
  production timeout or inserting timing between operations.
- Trust corruption fixtures retain the pinned approval-store directory while
  reopening the identity-matched record directory through the production movable
  operation boundary. This lets the fixture replace and remove its own synthetic
  records on Windows without weakening the store anchor.
- Picker persistence is observed through reopening the memory-backed TUI; the
  `config` command remains a non-creating snapshot operation.
- Unix terminal fixtures use the already pinned `portable-pty` spawn boundary,
  which establishes the child session and controlling terminal internally. This
  avoids adding unsafe consumer hooks and gives Crossterm one authoritative PTY
  for dimensions, raw input and output.
- Timeout diagnostics retain a bounded escaped raw-output tail and byte count so
  a missing parsed frame can be distinguished from a child that emitted no bytes
  or only an incomplete control sequence.

## Operational surface

The affected interactive surface is the existing native Kuru CLI/TUI on Windows
x86-64. Tests run as native runner processes with stock Windows PowerShell 5.1,
loopback-only HTTP fixtures, private temporary directories and fake credentials;
they add no bind address, container, secret, binary-version or architecture
change.

## Risks / Trade-offs

- Longer native fixture waits can lengthen a true failure. Bounded diagnostics
  and awaited cleanup keep failures finite and attributable.
- Literal PowerShell fixture values require correct quoting. The values are
  generated under controlled temporary roots and encoded as single-quoted
  literals with embedded quotes escaped.
- Portable PTY cleanup uses a blocking output pump behind a bounded queue. The
  fixture reaps the child, disconnects the queue before a bounded completion
  wait, and retains the completion proof across retries; a source that remains
  open is reported as uncertain instead of causing an unbounded join.
