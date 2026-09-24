## Context

The installer bounds downloads with a named FIFO, retaining the direct producer and `head` consumer so cleanup can stop both without relying on a hidden pipeline process. On macOS, launching an immediately failing writer before the reader can strand the later reader in the FIFO handoff even after the writer process has exited; process inspection showed only `head` remained with the FIFO open for reading.

## Goals / Non-Goals

**Goals:**

- Make immediate producer failure terminate deterministically with its real exit status.
- Preserve the existing byte limit and ownership of both child processes.

**Non-Goals:**

- Replace the bounded transport or change download, checksum, archive, or publication policy.
- Change native updater behavior.

## Decisions

Start `head` first and retain its PID before starting the producer. Its blocking FIFO open becomes the readiness boundary that the producer satisfies, so an immediate producer exit is observed as EOF rather than racing a later reader open.

Keep the current wait, byte-count, producer-status, and trap cleanup sequence. A regular temporary-file download would lose streaming bounds, and a shell pipeline would hide one of the process identities that cleanup must own.

## Operational surface

This remains a local compiler-free Bash installer, not a service: it binds no address, opens no inbound connection, and uses no secret. HTTPS release requests keep their existing curl constraints; local mirrors keep their exact path behavior. Supported release target names and archive limits are unchanged across macOS and Linux architectures.

## Risks / Trade-offs

- The reader blocks until the producer opens the FIFO. The producer is started immediately after the retained reader PID is captured, and existing signal cleanup owns both identities.
- A producer that stays open after the byte cap can remain live. The unchanged size check exits through the cleanup trap, which kills and reaps both retained children.
