# Design

## Context

ConfigSnapshot records automatic ancestor config.toml as repository-origin authority separately from user/local/explicit patches. Managed constraints currently reject conflicts; they do not rewrite user values. CLI executes every fixed/headless command outside its final None interactive branch. ui::run returns after its owned TerminalSession restoration attempt. Delivery already owns checked files, HTTPS release manifest primitives and fixed Manager upgrade hints.

## Goals / Non-Goals

Implement the proposal through those exact seams, preserving ConfigSnapshot capture and terminal ownership. No provider/engine/memory/UI-render changes, new dependency versions, explicit check command, telemetry, automatic install, alternate repository URL or notification framework. The receipt and Windows installation contracts remain unchanged.

## Decisions

### Personal configuration, unchanged policy

Add serde(default,deny_unknown_fields) UpdateConfig with notice false and Config.update. Reject a true notice in each automatic ancestor patch before merge, so later false cannot hide repository enablement. Automatic config is already repository-origin; checking Git tracking would add process authority and startup cost. User/config.local/explicit/-c use existing capture/precedence; managed false retains existing definite conflict rejection. False repository values may disable an advisory without granting network authority. Mirror object and managed definitions in the existing public schema, and retain leaf projection automatically through Config serialization.

### One concrete advisory session

Delivery notice::Session owns one finite async check and a bounded optional completed text. Start only in CLI's final None branch after finalized config/trust when stderr and interactive streams are terminals. Bounded cache read and check run independently of TUI input; no network is awaited before entering the terminal. Cached stable newer facts can queue immediately when read. Read/write only data/update/notice.json with existing private Directory APIs, schema deny_unknown_fields and bounded read; corrupt/unknown/foreign objects are not overwritten blindly. Fresh cache suppresses fetch for24h including failures; checked_by mismatch, future skew beyond5min or invalid content is due. Concurrent sessions need no new lock; unknown raced file identity refuses cache write rather than harming the command.

Only the finite async task owns HTTP and short advisory cache publication; state has no provider, installation or memory handles. Task publishes at most bounded completed text through a nonblocking owned result seam. Exit cancels the task and takes only already-completed advisory without awaiting it; no JoinHandle shutdown wait extends TUI exit. Cached facts need not be refreshed before display. Do not hold a watch/mutex guard across drawing or await. A check that completed may settle its short synchronous checked cache publication across an exit race: abort cannot interrupt an already-running filesystem operation. Exit never joins that advisory work. An unfinished HTTP response is aborted and never claims completion or creates a completed check cache.

### Fixed HTTPS request and stable parser

Use the existing reqwest version with HTTPS-only, fixed URL and UA, connect3s/whole5s, up to5 HTTPS redirects and streamed64KiB bound. No cookie/auth/query/provider settings. Existing system proxy behavior is documented. Add a bounded archive manifest parser selecting exactly one valid checksum entry for the host's exact archive suffix. Reuse checked_version, then numeric checked major/minor/patch comparison; reject prerelease, ambiguity, unsafe/control text and overflow. Format notices from validated versions plus literal Manager::update_hint; never print raw response/error/cache/path. Manager detection is read-only resolved root classification, not installed() ownership refusal or installation preflight.

### Terminal result ordering and fixture authority

CLI awaits its existing TUI function, then takes/aborts the advisory session. Only after successful terminal restoration is fixed advisory written to stderr; no alternate-screen/frame or stdout change. TUI errors suppress advisory, preserving existing failure handling. No notice for any Some command. Test-support-only HTTPS endpoint/root certificates follow existing connector fixture practice; shipping build ignores these env variables. Keep the seam in an existing dependency feature with exact lock edge only if required, not a new transport or public URL configuration.

## Integration contract

Production endpoint is exactly `https://github.com/replygirl/kuru/releases/latest/download/SHA256SUMS`, GET with fixed `kuru-update-notice` user agent; redirect destinations remain HTTPS with no embedded credentials, fragment or provider headers; server-supplied signed query parameters are allowed on redirects, while the fixed initial request has no query. The response is a UTF8 checksum manifest, not a release API or execution input. A cache schema1 record holds checked_at Unix seconds, checked_by running semantic version, outcome newer/current/failed, optional validated latest and optional fixed offline/http/malformed failure; unknown fields/schema or inconsistent tuples are invalid. Versions are validated numeric triples, never rendered raw response text. Owner-only cache staging/replacement uses existing Directory identity and publication semantics. Fixture endpoint and bounded16KiB CA overrides compile only with test-support and must remain HTTPS; no production configurable URL or authentication route is added. Core schema uses an additionalProperties:false update object with boolean notice defaultfalse and mirrors managed definitions. No public Event, memory schema, RPC, runtime or TurnOutput shape changes.

## Operational surface

Native interactive host, fixed GitHub HTTPS endpoint and existing system proxy environment. No listener/bind/credentials/provider identifiers. GitHub may observe the connecting IP; opt-in docs state this plainly. Advisory cache is private within configured data directory after its existing authority preflight. User preference cannot activate from repository text or skill instructions.

## Risks / Trade-offs

- [Network or stalled peer delays terminal] -> async finite owned check, abort/take without exit wait, causal held-server fixture.
- [Cache text leaks or escapes terminal] -> strict typed fields and stable numeric version validation, fixed diagnostic labels and bounded formatted notice.
- [Personal permission confused with repository trust] -> reject repository true before merge regardless of trust; allow only existing user-authority routes.
- [Two sessions race cache] -> retained checked file identity; refuse uncertain cache publication without changing command outcome.
- [Unavailable check is mistaken for upgrade proof] -> fixed advisory only; no candidate verification or install effects, failures silent and separate from live release acceptance.
