## Context

The retained tool root is a checked Windows directory capability. The expanded-name guard canonicalizes a candidate parent, but compares it with the retained root's original spelling. Ordinary `C:\...` and canonical verbatim `\\?\C:\...` prefixes are different path components, so valid file targets can fail before their permission rule is evaluated.

## Goals / Non-Goals

**Goals:** Preserve the checked root identity and protected-name guard while comparing equivalent physical path spellings; keep typed allow/ask/deny outcomes observable.

**Non-Goals:** Change permission policy, root trust, file-tool path syntax, or native process lifetime.

## Decisions

Canonicalize the retained root at the expanded-name comparison, matching the canonicalized candidate and the existing permission-target calculation. Keep `Directory::is_within`, the retained root's revalidation, and per-component protected-name checks as separate authority checks.

## Risks / Trade-offs

Canonicalization can fail if the retained path disappears; propagate that failure before any tool effect. The native Windows regression covers both ordinary and verbatim root spellings, because macOS cannot reproduce Windows prefix semantics.

## Operational surface

The change runs inside the existing local Kuru tool host. It adds no bind address, container, runner, secret, connection, or binary-version requirement. Native Windows x64 CI exercises the path behavior; macOS runs focused permission and static checks before the stack is pushed.
