## Context

The stable-speaker specification and implementation already define the authored cold-facing policy, but the two architecture/framework pages omit it. The durable memory specification also already requires safe guidance for any rejected Unix data directory, while the diagnostic is owned by `migration.rs` and is applied at only two store-open boundaries. Provisioning and server code reach the same checked owner-private filesystem primitive through `files::private_dir`, local wrappers, or direct `Directory::ensure_private` calls, so eligible failures there lose the exact-path remedy.

## Goals / Non-Goals

**Goals:** Document the existing cold-facing contract exactly. Put the existing private-directory diagnostic in the memory filesystem composition layer and use it at every production memory-owned owner-private directory boundary, including boundaries that must return a retained `Directory` handle.

**Non-Goals:** Change speaker selection, add focus controls, change any persisted format, widen `kuru-platform` policy, alter ownership or identity validation, change permissions automatically, relax refusal, or add signing, Phase 1 behavior, dependencies, or toolchains.

## Decisions

Move the existing Unix diagnostic composition from `migration.rs` into `packages/kuru-memory/src/files.rs`. The existing unit-returning `files::private_dir` will apply it around `Directory::ensure_private`; the OwnerOnly directory and parent composition paths will apply it while preserving each caller's `Movable` or `Pinned` name-retention contract. Production direct `Directory::ensure_private` and OwnerOnly `Directory::open` sites will use those shared boundaries. Store, migration, provisioning, server, staging, lifecycle, purge, and other memory-owned callers thereby stop duplicating policy or bypassing memory composition. The underlying error remains in the anyhow chain, preserving its kind for programmatic inspection. Inherited file paths, including the legacy SQLite read after its parent is validated, remain unchanged.

Keep the eligibility policy byte-for-byte equivalent in meaning. On Unix, guidance is added only when the source error is permission-denied and `symlink_metadata` proves the exact path is a real current-user-owned directory with group or other mode bits. The helper names only that path, suggests mode 0700, and never mutates it. Links, foreign-owned paths, other error kinds, and already-private directories receive no Unix mode advice. Windows retains its existing ACL enforcement and store/migration native owner-privacy guidance; the new Unix composition does not spread additional Windows remedy text, and no Unix mode text is introduced there.

Document selection after the stronger rules: explicit targeting and active focus constrain selection first, maximum activation narrows eligible candidates, and previous-completed-speaker continuity wins an equal maximum. A remaining cold tie follows built-in authored order—Self, Connection, Desire, or Continuity first for the corresponding mode, then later authored members. `mode-authored-order` identifies that choice; `stable-id-order` applies only when no authored identity is among the tied eligible candidates. Both are deterministic runtime reasons and grant no identity supervisory authority.

## Risks / Trade-offs

Routing the composition helper broadly changes human-readable context for more permission failures → preserve the original error in the chain and add regression controls for each production family plus noneligible links, foreign ownership, and other errors.

A central helper could accidentally become a platform-wide policy → keep it private to `kuru-memory::files`; `kuru-platform` continues to report checked filesystem facts without product guidance.

Documentation could imply a permanent leader → describe only the final cold-tie fallback, include the stronger preceding rules and later-member behavior, and state explicitly that authored order grants no authority.
