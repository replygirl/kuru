## ADDED Requirements

### Requirement: Distinct project-local and managed provenance

Kuru SHALL discover `.kuru/config.local.toml` only at the exact canonical workspace root. The file SHALL be bounded and checked as a regular file, and when the root is inside a Git worktree Kuru SHALL reject the local file if it is tracked or if its untracked status cannot be established. A valid local file SHALL be explicit user-local authority outside the automatic repository trust manifest. Managed configuration SHALL be loaded only from an explicitly provisioned external absolute path outside the workspace. The immutable snapshot SHALL retain final-leaf provenance across these layers, and a local or CLI value MUST NOT reclassify a remaining repository-origin leaf or instruction as user authority.

#### Scenario: Tracked local file
- **WHEN** a repository tracks `.kuru/config.local.toml`
- **THEN** Kuru rejects the file as a local override before activation and offers a bounded remedy.

#### Scenario: Untracked local and repo authority coexist
- **WHEN** an untracked local file changes one leaf and an ancestor config still contributes an effective MCP or tool-permission leaf
- **THEN** local authority needs no trust approval, while the remaining repository-origin claim still requires applicable workspace review.

#### Scenario: Dead repository authority
- **WHEN** an explicit local value disables a repository-contributed shell default
- **THEN** no shell authority claim is required for that now-ineffective repository value.
