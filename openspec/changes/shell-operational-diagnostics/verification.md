## 1. Source-free operational failures [critical]

- [x] 1.1 @regression (agent) run the focused Unix built-in-shell timeout/cancellation fixture with fake stderr secret and an open pipe -> `cargo test -p kuru-connectors unix_shell --locked` (17 passed): fixed category plus pending EOF, no raw partial output or source-chain leak.
- [ ] 1.2 @regression (agent) run the checked-in Windows-native `windows_shell_timeout_keeps_eof_complete_redacted_stderr` fixture -> pending native Windows CI execution of the stock PowerShell/Job path before archive and merge.
- [x] 1.3 @unit (agent) exercise EOF-complete diagnostic formatting with a fake recognizable credential and over-limit stderr -> `cargo test -p kuru-connectors shell_error_diagnostic_bounds_projected_stderr_without_reprojecting_marker --locked` and `... shell_diagnostic::tests::eof_finalizes_before_another_stream_or_process_completes ...` (1 passed each): redacted 4 KiB-bounded excerpt with intact markers and EOF finalization.
- [x] 1.4 @eval (agent) inspect the public ToolHost failure string across timeout, capture, ownership, cleanup, pre-launch cancellation, and retained-root rejection -> focused Unix suite (17 passed), then `cargo test -p kuru-connectors unix_shutdown_cancels_starting_and_active_shells_and_closes_mcp --locked` and `... retained_root_replacement_blocks_shell_and_keeps_file_capability ...` (1 passed each): timeout and cancellation keep their primary categories, ordinary pre-launch failures use the fixed operation category, the unconfirmed-ownership suffix is retained, and no raw chain is exposed.

## 2. Shell completion and ownership boundaries

- [x] 2.1 @integration (agent) run focused Unix shell tests including pipe-read and unconfirmed-cleanup paths -> `cargo test -p kuru-connectors unix_shell --locked` (17 passed): source-free fixed categories while existing owner retention and cleanup behavior remains asserted.
- [x] 2.2 @integration (agent) run focused native shell completed-nonzero fixture -> `cargo test -p kuru-connectors shell_projection_redacts_both_streams_without_changing_exit_status --locked` (1 passed): exit 7 remains structured JSON with independent projected stdout/stderr fields.
- [ ] 2.3 @equivalence (agent) compare focused platform formatter assertions -> pending native Windows fixture execution; both paths call the shared formatter.
