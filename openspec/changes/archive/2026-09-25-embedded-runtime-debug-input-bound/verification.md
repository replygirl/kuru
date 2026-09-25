## 1. Instrumented packaged runtime [critical]

- [x] 1.1 @regression (agent) exercise `packaged_install_and_update_preserve_complete_offline_memory` with the real instrumented selected executable -> full push 1599 failed at the old 168,820,736-byte pre-copy gate on a 171,024,280-byte selected artifact; corrected exact cargo-llvm-cov filter passed 1/1 in 47.10 seconds with a 170,624,424-byte selected artifact, a 117,967,912-byte private stripped input, a 59,338,938-byte archive, real offline direct install and self-update conversations, and a fresh 554,704-byte packaged-child profile.
- [x] 1.2 @integration (agent) inspect a separate private copy stripped with Apple `strip -u -r` -> copied 171,024,280-byte selected executable stripped to 118,213,992 bytes, below unchanged 134,217,728-byte shipping cap; source artifact was not modified.

## 2. Delivery gates

- [~] 2.1 @integration (agent) run normal final-head hooks and combined coverage -> defer: the prior full push failed at the old fixture input guard; the corrected final source has only the focused instrumented pass and has not been pushed.
- [~] 2.2 @e2e (agent) run exact-head native installation and update on supported systems -> defer: hosted CI starts only after the corrected normal push.
