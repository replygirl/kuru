## Context

`file_read` currently retains `MAX_BYTES + 1` then fails, while Unix and Windows
shell captures fail as soon as either independent retained vector crosses that
same limit. The connector scanner already projects secrets incrementally, but
only into a complete `Vec`; later runtime truncation is head-only. Applying a
head/tail formatter after either producer already failed cannot recover a tail,
and changing MCP record framing would conflate tool presentation with transport
admission.

## Goals / Non-Goals

**Goals:**

- Retain fixed-size head/tail excerpts for built-in file text and each shell
  stream after scanner projection.
- Keep redaction markers, UTF-8, shell ownership/EOF draining, independent
  stdout/stderr budgets, valid tool receipt JSON, and call IDs intact.
- Leave the typed `file_list` projection and 10,000-entry cap unchanged, and
  leave MCP protocol limits unchanged.

**Non-Goals:**

- No output archive, retention store, setting, public result field, or schema.
- No MCP oversized-frame admission, parser rewrite, shell deadline change, or
  arbitrary special-file read path.
- No change to the metadata-only interactive TUI activity surface.

## Decisions

1. Generalize the private connector scanner destination to a bounded streaming
   head/tail sink. It receives scanner-produced bytes, reserves one fixed
   omission marker, and treats scanner-emitted redaction markers atomically.
   Existing `truncate_tool_output` uses the same marker-safe head/tail policy
   for already-projected runtime text.
2. File reads keep the existing checked regular-file and UTF-8 contract, but
   scan bounded chunks through the finite regular file instead of retaining the
   complete contents. They do not turn pipes, directories, or device-like paths
   into an unbounded input source. Shell readers continue draining their owned
   pipes until EOF under the existing operation deadline while retaining only
   each stream's independent fixed output budget.
3. `file_list` remains on its typed JSON projection path and its existing
   10,000-entry limit. It has no 2 MiB overflow failure to replace and is not
   converted to an excerpt or a new listing schema.
4. Runtime keeps the existing tool-result API and applies the helper at the
   8 KiB receipt and actor quota boundaries. The helper reserves the omission
   marker and produces a valid UTF-8 string before JSON serialization, so
   call-ID preservation is mechanical.

## Risks / Trade-offs

- A long regular file still requires streaming scan work to find/redact its
  actual tail. Bounded chunks, the checked regular-file admission boundary, and
  the finite file object avoid retaining unbounded bytes or reading a special
  stream; shell work stays bounded by its existing deadline.
- A redaction token can cross a chunk or excerpt boundary. The scanner owns
  detector state across chunks, and the sink reserves whole omission/redaction
  markers before retaining a join.
- Escaped JSON can exceed a text allocation. Runtime keeps its existing loop
  that reduces the output allowance until the JSON envelope fits, rather than
  dropping call IDs or emitting malformed receipts.

## Operational surface

No bind address, secret, container, provider, or executable-architecture
selection changes. The existing Unix and Windows shell fixtures retain their
native process owners and separate output streams; native Windows verification
remains required rather than inferred from a host build.
