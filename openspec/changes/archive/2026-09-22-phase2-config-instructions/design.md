## Context

`ConfigSnapshot` already captures ancestor config and instruction bytes before trust review, then applies only saved mode/model/effort preferences during finalization. Its TOML merge tracks final leaf origins. The CLI presently supplies a manually selected final TOML file and dedicated scalar flags. P01b will extend instruction discovery separately and can consume this snapshot without changing its authority rule.

## Goals / Non-Goals

**Goals:** Keep source classification explicit, retain one preflight snapshot, and make managed constraints inspectable without introducing executable policy code.

**Non-Goals:** A central policy service, arbitrary predicates, repository-discovered managed files, or a second trust/permission approval flow.

## Decisions

1. Use `.kuru/config.local.toml` only at the canonical project root. The CLI captures bytes from a checked regular file and asks Git's built-in index query whether that path is tracked. Core parses those exact captured bytes rather than reopening the path. If tracked, reject it. If Git is unavailable or index status ambiguous inside a repository, reject this optional layer with an explicit `--config` remedy. Outside a Git worktree, the checked file is accepted as local. A path named `local` alone is insufficient; silently accepting a tracked copy would grant repository-supplied content caller authority. The Git query uses fixed arguments, clears inherited `GIT_*` repository-selection/configuration variables so `-C` resolves this worktree's index, and performs no shell interpolation or configured command launch.
2. Read an optional absolute `KURU_MANAGED_CONFIG` path only after canonicalizing it outside the workspace. Its `[defaults]` table uses the ordinary strict config shape. Its `[constraints]` table uses the same typed leaf vocabulary as exact locks. Compare each constrained leaf of the final typed `Config` representation to its typed expected value. Arrays and alias tables are locked as whole values so a later layer cannot add a rule or alias while satisfying only one nested field. This avoids an unbounded policy expression language while covering supported settings.
3. Resolve builtins → managed defaults → existing user config → ancestor project files → saved choices → discovered local → explicit `--config` → repeatable `-c` → dedicated flags. The snapshot captures all file and CLI inputs once; finalization reuses those captured values after loading saved choices. Managed locks are checked both for the pre-memory snapshot and again after saved preferences. We reject a conflicting final value rather than silently replace it, so the caller can correct the source that conflicts.
4. Parse each `-c key=value` as a tiny TOML assignment with a validated dotted key path, then merge it with ordinary layer semantics and run native type/semantic validation. Later assignments win, with tables merged and arrays replaced, and carry CLI origin. Do not echo raw values in diagnostics. Existing dedicated flags remain the final writer.

## Risks / Trade-offs

- [Git is absent or its index cannot be read] → Only the optional automatically discovered local file fails; explicit `--config` continues to work. Kuru's normal installed runtime does not depend on Git.
- [Git resolves through caller PATH] → The new index query uses the caller's existing process environment; Kuru does not prepend project paths. A caller-configured project executable in PATH is caller process authority, outside the repository configuration trust claim. The query's fixed arguments, read-only operation and bounded wait remain the P01a boundary.
- [The local file changes after capture] → The invocation uses captured bytes and final origin; the next invocation rechecks the file and Git index. This retains the existing snapshot boundary.
- [Managed constraints and saved choices conflict] → Finalization fails before provider, memory-dependent authority or tool activation; the stored choice is left unchanged for correction.

## Operational surface

The CLI's existing process, data directory and provider routes remain unchanged. The optional managed file is supplied by an absolute external environment path; it carries no credential value requirement. Git is invoked only for local-file classification with fixed arguments, bounded output and no hooks or repository-configured command. If unavailable, normal runtime still works and only automatic local discovery fails when the local file exists.

## Integration contract

`KURU_MANAGED_CONFIG` identifies the managed document and may not point into the workspace. The published ordinary configuration schema remains the source shape for both managed tables and typed overrides; native validation remains authoritative. The local file lives at `.kuru/config.local.toml` of the exact canonical `-C` root. No SDK, provider wire format or persistent ID changes.
