## 1. Usable expressive terminal [critical]

- [x] 1.1 @e2e (agent) Drive the real binary through a PTY with chat, selectors, resize, paste and quit -> Final mise run check passed the real_pty_accepts_chat_navigation_commands_and_restores_terminal test, including both normal and reduced-motion startup, RGB output, input and terminal restoration. Readiness waits for the actual first frame rather than assuming a fixed process startup time.
- [x] 1.2 @integration (agent) Render real View states with TestBackend at wide, narrow and tiny sizes -> Seven visual integration tests passed across all four frameworks, long catalogs, rich messages and Unicode input; sizes include 140x50, 120x45, 70x25, 54x35, and tiny cells.
- [x] 1.3 @manual (agent) Inspect rendered welcome, conversation, busy and selector screens and open the build in cmux -> Inspected actual TestBackend HTML/text exports and PNG captures of welcome, conversation and busy graph; confirmed seven distinct numbered nodes and their legend, readable rich content, and centered welcome. OpenAI-backed build launched in cmux workspace:29, surface:39, titled Kuru · new design; original surface:37 session preserved.

## 2. Accurate activity and optional motion [critical]

- [x] 2.1 @integration (agent) Feed actual runtime event shapes into View and verify peer/relationship labels and frames -> Real DemoProvider turn events and PeerMessage envelopes preserve group membership and speaker identity, conceal private message content, and animate route cells while leaving the draft/cursor unchanged. Whole buffers match across frames with reduced motion.
- [x] 2.2 @e2e (agent) Toggle reduced motion in the PTY without losing chat and editor behavior -> Actual terminal output changes during welcome animation and becomes quiescent when F6 disables motion; startup KURU_REDUCED_MOTION=1 and toggling back are both exercised by the passing PTY test.
- [x] 2.3 @unit (agent) Check animation cadence, finite welcome, settled idle and disabled-motion frame behavior -> animation_has_a_fixed_cadence_finite_welcome_and_static_reduced_mode passes, checking the 80ms boundary, four-second welcome cutoff, idle freeze, busy ticks and F6 cancellation compatibility.

## 3. Repository quality

- [x] 3.1 @regression (agent) Run mise run check including the 90% workspace coverage gate -> Passed with 127 Rust tests, 11 Python installer tests, Clippy warnings denied, all format/tooling checks, and 97.62% LLVM line coverage (5997/6143). No dependencies changed.
- [x] 3.2 @integration (agent) Validate cospec and check generated drift before archival -> Final mise run check passed strict validation of vivid-tui and all four durable specs; cospec update --check reported no drift. Archival is the following delivery operation.
