## 1. Rules, schema and trust

- [x] 1.1 Add bounded pure core selectors, anchored file matching and deny/ask/allow precedence with legacy boolean fallbacks; verify overlapping rules, literal paths, invalid patterns and fallback fixtures.
- [x] 1.2 Extend strict layered configuration and the published versioned schema for replacement `permissions` arrays; verify TOML/JSON example parity and unknown/invalid key rejection.
- [x] 1.3 Include effective automatic permission rules in the exact-root trust authority manifest with final-leaf provenance; verify unapproved ancestor rules cannot activate tools and manifest approval does not grant a call.

## 2. Effect-bound execution and checked grants

- [x] 2.1 Add the shared connector permission service, immutable invocation identity and typed permission-required denial at `ToolHost::execute`; verify direct/model native file, shell and MCP fixtures and no effect on deny or unanswered ask.
- [x] 2.2 Route runtime outbound `a2a_send` through the same service before network dispatch; verify alias/endpoint changes, headless ask refusal and zero remote effect on deny.
- [x] 2.3 Add app-owned checked private once/session/always grant records and revocation using existing platform file primitives; verify exact-file literal scopes, full-invocation once consumption, root/manifest/route invalidation, restart, resume reset and store-failure refusal.

## 3. Foreground decisions and presentation

- [x] 3.1 Connect a bounded cancellation-aware approval channel only to an attached foreground TUI operation; verify closed UI, cancellation, late reply and unattended CLI/A2A/dream refusal without a held memory/UI lock.
- [x] 3.2 Render inline once/session/always/deny choices with the exact redacted scope, permission chip and `/permissions` inspection/revocation; verify real PTY interaction, composer usability and displayed/stored scope equality.
- [x] 3.3 Update user configuration, tools and command documentation for rules, legacy false-as-ask, no-sandbox shell authority and grant lifetime; verify docs build, formatting and links.

## 4. Integrated acceptance

- [x] 4.1 Run the declared cross-boundary native file/shell/MCP/A2A effect and trust fixtures plus real PTYs; record observed local outcomes against verification rows 1–3 and distinguish native CI delivery evidence, including Windows, from local acceptance.
- [ ] 4.2 Run scoped typecheck/lint/format/schema/docs checks, one combined coverage gate and strict cospec validation; record the actual results and resolve independent final review findings before archive.
