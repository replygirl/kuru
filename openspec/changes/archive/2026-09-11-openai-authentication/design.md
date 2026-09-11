## Context

The external Codex adapter conflated authentication with another harness. The
existing API-key Responses adapter already supplies request conversion and
per-actor tool-continuation handling; reuse it for direct subscription HTTP.
Verified protocol source is OpenAI commit 6b9826e3aa83b1a5947db50f4332cb9c65f1b340,
cross-checked with the user-named OpenClaw and Hermes implementations. Source maps
and exact request contracts are retained in /tmp/kuru-native-openai-auth-research.md
and /tmp/kuru-native-openai-transport-plan.md. These are current implementation
details, not a guarantee that the ChatGPT backend is a stable public API.

## Decisions

1. Preserve existing provider names and selection behavior. codex uses native
   subscription authentication; responses uses the existing configurable API-key
   environment variable. No new auto-auth mode, provider labels or fallback.
   Remove codex_command with an actionable config error. Native auth belongs
   behind connector APIs; CLI resolves paths, presents login and opens the browser.
   Windows browser opening uses a narrow safe platform wrapper over the desktop
   ShellExecute URL handoff on a COM-initialized thread. Browser lifetime is not
   an owned process Job; an unavailable opener leaves the printed URL usable.

2. AuthManager construction is side-effect free and receives explicit data/root
   paths and an optional in-memory API key. Only Kuru's private data/auth/openai
   store is eligible; reject tool-root-contained state and unsafe objects. Status
   is existing-only. Stable short leases and atomic replacement protect token
   rotation across processes; generation and login-session identity prevent stale
   refresh from overwriting a newer login or resurrecting logout. Refresh once
   on expiry or one pre-body401, reload under lease, preserve rotated material if
   the caller cancels, and do not repeat uncertain refresh POSTs. Tokens never
   appear in Debug/errors/config, and other harness stores are never imported.

3. Browser OAuth uses the verified public client app_EMoamEEZ73f0CkXaXp7hrann,
   https://auth.openai.com/oauth/authorize and /oauth/token, scopes
   openid/profile/email/offline_access, cryptographic random state and PKCE S256.
   Bind 127.0.0.1:1455 before presenting redirect
   http://localhost:1455/auth/callback. A busy port reports an error and device
   option; never stop another listener. Validate method/path/host and unique
   exact state/code before token exchange. Exchange form fields include the
   original redirect and verifier; official refresh uses JSON. Device login uses
   the verified usercode/token polling and code-exchange flow with bounded expiry
   and server pacing. Cancellation/expiry releases the owned listener and cannot
   publish an unvalidated login. Decode token claims for account routing/expiry
   only; never represent decoding as JWT signature verification.

4. Subscription requests use fixed https://chatgpt.com/backend-api/codex
   /models?client_version=0.154.0 and /responses, Bearer plus ChatGPT-Account-ID,
   truthful Kuru identity, stream=true and store=false. OAuth material never
   reaches a configurable API-key endpoint. Collect full SSE output_item.done
   records and require successful response.completed, which can omit output.
   Convert through existing Completion/tool-continuation code, preserving
   account/session identity and rejecting partial/error responses. Existing Kuru
   context and tool policy are unchanged. Keep arbitrary model/effort strings;
   public API models still require explicit model selection because that endpoint
   supplies no default/effort catalog. No upstream catalog prompts or tools apply.

## Operational surface

Native macOS/Linux arm64/x86-64 and Windows x86-64 use the existing CI/install
graph; no container or extra executable. The callback binds loopback only with
one bounded request at a time, short request-read deadlines and a 600-second login
window. Auth/catalog/API HTTP keeps 60-second totals and 10-second connection limits.
Subscription inference retains 600-second total and 60-second idle limits, plus the
existing 2 MiB transport bounds. Tests use synthetic credentials and local HTTP fixtures,
not hosted account secrets. Browser opening failure leaves a usable printed URL.

## Integration contract

AuthManager provides redacted status, browser/device login handles, logout and
opaque explicit-route credential snapshots. Login finish owns exchange and
private publication; transport refresh cannot change account/session or billing
route. CLI cancellation drops the login handle, while already-dispatched token
rotation retains bounded ownership through safe completion. The Provider trait
and existing request/Completion types stay intact. Real local HTTP tests observe exact
requests, callback handling, SSE and token rotation. This is auth/transport
verification, not runtime evaluation or a peer behavior change.

## Risks / Trade-offs

- Backend protocol drift → isolate constants, test exact wire contracts and report
  provider errors without changing model, account or billing route.
- Credential races/disclosure → checked private files, short stable leases,
  generations, redacted errors and real concurrent rotation/logout tests.
- Callback spoofing/hangs → PKCE/state, loopback validation, byte/deadline limits
  and explicit cancellation with no listener takeover.
- Fixtures cannot establish account eligibility → keep a user-participating live
  login and conversation check distinct from deterministic tests.
