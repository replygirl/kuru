## Context

Windows already retains an EOF bit and a streaming-projected shell capture, but
its failure formatter first combines raw `anyhow` and process metadata.  Unix
only returns captures after successful EOF finalization, so its cleanup path
cannot format a retained stderr state after timeout, cancellation, or a pipe
failure.  Both cases can leak raw operational detail through the error chain.

## Goals / Non-Goals

**Goals:**

- Use one finite source-free diagnostic grammar on both native platforms.
- Retain only a bounded, projected stderr excerpt once EOF has made it complete.
- Preserve normal completed shell JSON and every existing process/pipe owner.

**Non-Goals:**

- Change recognizable-secret detectors or their public API.
- Redesign shell admission, retained-owner backoff, group signalling, or Windows Job cleanup.
- Add a new operational log or expose partial pipe bytes.

## Decisions

- A private category enum plus a private shared formatter is the model-facing
  boundary.  The formatter takes only a category and an EOF/pending stderr
  state, never an error chain, command, stdout, or native metadata.
- Reuse the existing 4 KiB `truncate_tool_output` diagnostic bound and
  `StreamingProjection`.  A non-EOF capture becomes the literal pending-EOF
  state rather than a partially finalized projection.
- Unix will return/retain capture state alongside its primary result so
  `finish_with_cleanup` can format failure after dropping pipe handles and
  running its current cleanup protocol.  This avoids moving ownership between
  worker, pipes, or retained cleanup.

## Risks / Trade-offs

- [A bounded stderr clue can be less detailed than raw platform errors] → use
  finite categories and an EOF-complete redacted excerpt, while keeping raw
  details outside the public error path.
- [Refactoring Unix result plumbing could disturb cleanup sequencing] → keep
  pipe dropping and cleanup calls in their existing order, and cover timeout,
  pipe failure, cleanup uncertainty, and completed nonzero behavior with
  focused native tests.
