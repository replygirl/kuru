## Context

`ToolHost` already owns a retained checked directory capability, path
normalization, protected-path rejection, permission service, redaction, and a
bounded text projection. P03 extends that boundary. It must not turn search
into an ambient filesystem walk or a shell substitute.

## Goals / Non-Goals

**Goals:** native repository search backed by upstream Rust components, exact
native permission semantics for discovered files, and a continuation protocol
that does not split Unicode text.

**Non-Goals:** edits, external web search, parallel tool execution, byte-based
or arbitrary binary paging, symlink traversal, and an embedded or externally
installed `rg` executable.

## Decisions

1. **Use ripgrep's published Rust crates (`ignore`, `grep-regex`, and
   `grep-searcher`) through exact workspace pins.** This preserves ripgrep's
   ignore and regex semantics without a process dependency. A hand-rolled walk
   plus Rust `regex` was rejected because it would not faithfully supply the
   requested ripgrep components or ignore behavior.
2. **Traverse from the retained root but check every candidate using the
   existing contained open and permission service.** A request-level decision
   alone cannot safely disclose a denied descendant. Treating an allowed root
   selector as an implicit grant for all descendants was rejected because it
   bypasses specific denies.
3. **Default to skip hidden and ignored paths, with `include_hidden` and
   `include_ignored` opt-ins.** This matches normal repository search
   expectations while keeping intentional exceptional discovery explicit.
   Always including or always hiding these paths was rejected because neither
   gives callers a predictable way to search generated or dotfile content.
4. **Page file_read in one-based logical text lines.** The response preserves
   the current `text` result and conditionally adds additive page metadata.
   Byte offsets were rejected because they can split UTF-8 and make a caller's
   continuation sensitive to encoding; Unicode-scalar offsets were rejected
   because they do not match users' visible line-oriented reading task.
5. **Reject binary and non-UTF-8 reads/search candidates.** This avoids false
   text matches and lossy continuations. Rendering arbitrary bytes as escaped
   or base64 was rejected because it would add a distinct binary transport to
   P03 and could consume output budgets unexpectedly.

## Integration contract

`ToolHost::catalog` owns provider-visible JSON schemas for `grep`, `glob`, and
the extended `file_read`; `ToolHost::execute_with_approval` remains the sole
dispatch and result-projection entry point. Inputs and outputs are JSON values:
paths are project-relative UTF-8 strings, boolean inclusion switches default to
false, and paging metadata appears only when either paging input is supplied.
The retained `Directory` capability remains the filesystem authority and the
existing `PermissionService` remains the approval authority. Test fixtures use
temporary checked project roots and the real ToolHost, never an ambient mock.

## Risks / Trade-offs

- [Search traversal consumes resources before output bounds apply] → Cap
  candidates, metadata reads, bytes read, matches, and total result bytes;
  terminate deterministically at the first cap.
- [Ignore files may be malformed or unavailable] → use the upstream walker's
  bounded error handling and skip inaccessible entries without returning their
  contents.
- [A file changes between enumeration and opening] → reopen it through the
  checked directory capability and derive returned contents only from that
  handle.
- [Permission decisions are asynchronous] → execute native traversal in a
  bounded blocking worker only for filesystem/regex work and return to async
  evaluation before each candidate result is admitted. A candidate configured
  as `ask` uses the operation's existing foreground approval sender; headless
  results report permission-required coverage rather than pretending a
  complete search.
