## 1. Correct native acknowledgment timing

- [x] 1.1 Preserve the observed pre-fix installed update failure, add a real slow-publication helper regression and separate the bounded publication wait from startup/frame limits. CI 34607501091 completed actual Windows embedded install/update in 134.46 seconds and passed all 12 native updater tests, including publication held beyond ten seconds.
- [ ] 1.2 Verify actual IPC timeout, truncated/oversized frames and existing exact-image/recovery behavior without weakening their checks.
- [ ] 1.3 Run owning static checks and native Windows CI, record installed update acceptance, then validate and archive before the final branch commit.

The subsequent test-only cleanup and retained-root observation correction in
e080f84 has not yet run natively. Its outcome must be recorded separately from
the production acknowledgment correction already exercised by CI.
