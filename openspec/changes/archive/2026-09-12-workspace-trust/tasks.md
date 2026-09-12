## 1. Core snapshot and held-root primitives

- [x] 1.1 (kuru-core owner) Replace reloadable authority configuration with an immutable, side-effect-free `ConfigSnapshot` that retains layer origins, final-leaf provenance, explicit patches, every CLI override including write/shell/no-dream, and selection overlays; sanitize read/parse/type/validation diagnostics to bounded escaped source/category/available position; verify core tests cover exact-leaf provenance, secret-bearing malformed configurations, ordinary ancestor settings, and post-parse file replacement.
- [x] 1.2 (kuru-core owner) Derive versioned canonical authority claims for write, shell, every MCP leaf, configured memory paths, the active Responses tuple and external agents; hash typed domain/version/length-delimited canonical full values while persisting and displaying only digests/redactions, verify digest changes on each effective ancestor contribution, and prove no authority configuration mutates after snapshot review.
- [x] 1.3 (kuru-platform owner) Expose only the existing retained `Directory` self-revalidation primitive needed by trust consumers; verify native Unix and Windows replacement fixtures reject an observed held-name substitution.

## 2. TUI trust state and activation order

- [x] 2.1 (kuru-tui owner) Implement the checked private non-Dolt approval store keyed by normalized exact root, native identity and complete manifest digest; persist only an explicit full-manifest approval, treat claim digests as audit data, invalidate the whole record on any authority change, and verify unsafe links, hard links, permissive paths, unknown fields, oversized records and uncertain publication never match.
- [x] 2.2 (kuru-tui owner) Add redacted `trust status`, `trust approve`, `trust revoke`, `--trust-workspace-once`, TTY approval and pre-alternate-screen TUI choice; make one-time approval subset-only and nonpersistent, make status a non-state-creating checked read and absent revoke a non-state-creating no-op, and verify EOF/refusal writes no approval and noninteractive failure is bounded and actionable.
- [x] 2.3 (kuru-tui owner) Apply the approved command activation matrix to retained snapshots before memory, preference, provider, `ToolHost`, MCP, external agent or harness construction; make `config` omit saved preferences without opening/migrating/provisioning memory or taking a writer lease, pass the original retained root into the approved host/harness path, and verify each command activates only its listed claim subset.
- [x] 2.4 (kuru-tui owner) Keep fixed ChatGPT login/logout independent of workspace Responses configuration and gate `auth` environment checks; route `undo-dream` through the provider-free runtime path, and verify fake API-key environments remain unread before applicable approval.

## 3. Connector launch boundary

- [x] 3.1 (kuru-connectors owner) Carry the original retained approved workspace check through `ToolHost` into shell and stdio MCP pathname-cwd launches and revalidate immediately before spawning; do not replace it with an independently reopened guard after approval, and bracket any capability-root open with its revalidation; verify observed replacement starts no configured child while existing file-tool containment remains unchanged.
- [x] 3.2 (kuru-connectors owner) Ensure MCP discovery and external-agent advertisement/invocation receive preflight authorization from the TUI activation boundary; verify unapproved stdio and HTTP configurations create no child or socket and advertise no agent.

## 4. Provider-free undo path

- [x] 4.1 (kuru-runtime owner) Extract a narrow public `undo-dream` runtime path from the existing compensating membership-revision and reconciliation logic so it accepts finalized configuration, canonical scope, memory and resume state without a provider, `Harness`, `ToolHost`, actor startup or credential read; verify it preserves later conversations and archived identities.

## 5. Integrated behavior and documentation

- [x] 5.1 (kuru-tui owner) Add isolated real-process and real-PTY fixtures proving no memory probe/mutation, secret read, provider request, child or socket before refusal; prove status/absent revoke create no state, config omits preferences without memory activation, undo-dream avoids provider/auth activation, approval invalidates for root, identity and every authority change but not ordinary settings, and persistent approval follows only complete review.
- [x] 5.2 (kuru-tui owner) Document trust configuration precedence, exact-root scope, full-manifest persistence, one-time subset-only behavior, redacted diagnostics, config preference omission, command-specific noninteractive behavior, invalidation and the process-authority/Unix cwd limitations; verify documented commands in isolated fixtures and docs through their owning mise tasks.
- [x] 5.3 (coordinator) Run the scoped core/platform/TUI/connectors/runtime native checks and one coordinated integration/coverage pass after sibling source changes are integrated; native macOS combined coverage passed with 17,284/17,887 lines (96.63%); format/static/tooling/docs passed; Windows execution explicitly deferred to native CI before merge.
