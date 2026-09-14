## Why

The blanket requirement to clear inherited Git repository selectors for every repository command adds unnecessary wrappers and audits beyond the specific foreign-repository test boundary it was meant to protect.

## What Changes

- Remove that blanket repository-command requirement from `AGENTS.md`.
- Preserve the existing guidance for foreign-repository child-process tests and their environment isolation.

## Impact

Only repository contributor instructions and this maintenance record change. Application source, tests, and test behavior remain unchanged.
