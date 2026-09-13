## 1. Streaming connector excerpts

- [x] 1.1 Extend the private redaction scanner with fixed marker-safe head/tail
  retention and verify a chunked recognized secret crossing every excerpt join
  fails before the correction and passes after it.
- [x] 1.2 Replace built-in regular-file and separate Unix/Windows shell capture
  overflow failures with bounded streaming excerpts, preserving UTF-8, regular
  file admission, EOF drain, shell deadlines, and independent stream budgets.
- [x] 1.3 Preserve the existing typed `file_list` 10,000-entry cap and MCP
  stdio/HTTP/SSE wire admission failures at their current protocol bounds.

## 2. Runtime and direct command receipts

- [x] 2.1 Apply marker-safe head/tail truncation at the existing runtime receipt
  and optional tool-history boundaries; verify valid JSON, original call IDs,
  visible tails, and no partial redaction marker.
- [x] 2.2 Exercise a real direct CLI file/shell tool result with exact marked
  output, and retain existing metadata-only TUI activity behavior.

## 3. Documentation and gates

- [x] 3.1 Update built-in-tool documentation to distinguish bounded retained
  excerpts from unchanged shell deadlines and hard MCP parser framing limits.
- [ ] 3.2 Run focused connector/runtime/TUI checks, record native Windows and
  coordinated coverage evidence honestly, then validate the change strictly.
