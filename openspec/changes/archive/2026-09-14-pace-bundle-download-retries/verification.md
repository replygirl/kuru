## 1. Production retry pacing [critical]

- [x] 1.1 @regression (agent) serve one retryable 500 followed by the verified pinned bytes through ordinary `prepare_asset` -> the prior production policy repeats after 250 ms, while the corrected policy waits at least five seconds, makes exactly two identical GETs, and publishes the verified archive
- [x] 1.2 @integration (agent) exercise the real loopback recovery suite through the private explicit delay policy -> status, send, and body recovery remain bounded to three identical GETs and the existing total deadline, staging reset, cancellation, lock, and publication assertions pass without global test behavior substitution

## 2. Preserved failure policy

- [x] 2.1 @equivalence (agent) exercise permanent status, `Retry-After`, integrity, local I/O, and terminal retry diagnostics -> all remain terminal with their existing context and bounds; only the two retry pauses change
- [~] 2.2 @runtime (agent) run the corrected pull-request head through required GitHub CI -> defer: required checks run only after the completed record is archived and pushed and remain the merge gate
