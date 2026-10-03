# Proposal

## Why

The bootstrap's `bounded()` streamed each download, decompression, listing
and extraction through a FIFO to `head -c limit+1` and waited for `head` to see
end of file. On macOS the reader sometimes never sees end of file after the
producer exits, so the bootstrap hangs with the stage `stream` present: main
93307431 (run 37130393456, macos-latest coverage partition 4) timed out in
`explicit_github_and_custom_mirror_versions_use_literal_release_directories`
after `kuru` was extracted. Measured on macOS, the stall occurs whether the
children or the bootstrap open the FIFO, and never over an anonymous pipe;
Bash 3.2 cannot keep both process identities across an anonymous pipe.

## What Changes

- `bounded()` no longer uses a FIFO or a reader process. The producer is the
  only child: it runs with stdout redirected to the stage file under a kernel
  file-size limit (`ulimit -f`, 1024-byte blocks with POSIX mode turned off) of
  exactly the cap. A write past the cap stops it with SIGXFSZ (status 153),
  reported as "exceeds size limit"; any other failure is reported with the
  producer's status. A producer that would keep its output open after the cap
  is stopped by the same signal at its first write past it, and cleanup still
  kills it on INT, TERM, HUP or failure.
- An inherited hard file-size limit below a cap cannot be raised; the producer
  then keeps that tighter inherited limit.
- `mkfifo`, `head` and `wc` are no longer required tools.
- The Unix bootstrap fixture's timeout diagnostics list the stage files written
  after `kuru` (`README.md` and the shell-support stage files).
- A mirror-case regression test supplies a hostile `mkfifo` whose FIFOs cannot
  be opened for writing; the FIFO protocol waits for the fixture's bound, the
  fixed bootstrap installs without calling `mkfifo`.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-delivery/support/install.sh`: `bounded()`, cleanup and the
  required-tool list.
- `packages/kuru-delivery/tests/bootstrap_install.rs`: new regression test;
  the xtrace diagnostic test asserts the `ulimit -f` cap instead of the removed
  reader variable.
- `packages/kuru-delivery/tests/support/bootstrap_process.rs`: wider stage
  observation list.
- `docs/install.md`: the bounded-download sentence names the file-size limit.
- Retained earlier bootstraps (published tags) keep their FIFO protocol; the
  fixture still provides `mkfifo` and `head` for them.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
