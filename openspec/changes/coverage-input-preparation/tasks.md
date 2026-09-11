## 1. Prepare only coverage inputs

- [x] 1.1 Replace the ordinary prefetch dependency with explicit memory-owned bundle inputs; verify the resolved mise graph retains host and Windows payload preparation, a single instrumented suite and ordinary snapshots only for ordinary tests.
- [x] 1.2 Correct contributor/release guidance and run relevant format, tooling, cospec and documentation checks; verify no application source, gate threshold, secret or release dependency changes.
- [ ] 1.3 Run the normal concurrent pre-push with a fresh private Dolt cache, then observe actual native CI coverage and installed-runtime checks; record cold startup and measured preparation separately from historical timings before archiving.
