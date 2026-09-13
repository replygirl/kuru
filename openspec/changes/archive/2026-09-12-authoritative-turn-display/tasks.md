## 1. Completed turn delivery

- [x] 1.1 Add and observe a failing regression showing that a broadcast response becomes transcript content before the fix.
- [x] 1.2 Return a typed command/turn outcome from dispatch and transport the full turn result through the existing generation-checked completion channel.
- [x] 1.3 Render returned answer text once, the returned speaker, input/output tokens and a visible generic limited-result indication; activity events cannot replace completed facts.
- [x] 1.4 Preserve slash-command feedback, error and cancel behavior; verify dropped, delayed and duplicate response/speaker activity cannot corrupt the completed answer.

## 2. Verification and documentation

- [x] 2.1 Update terminal documentation for the completion information.
- [x] 2.2 Run relevant regression and real-terminal checks at practical wide and narrow sizes; record actual outcomes and platform limits.
- [x] 2.3 Run formatting, affected-package static checks, documentation build/content checks and the required workspace coverage gate; record evidence without duplicating the full behavioral suite.
- [x] 2.4 Validate the completed artifacts and acceptance evidence for cospec archive.
